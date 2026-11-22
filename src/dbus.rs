//! Everything that touches BlueFerry's D-Bus interface: the `Messages1` and
//! `Events1` proxies, the JSON wire types they return, and the conversion of
//! those into the public types. Private to this crate.

use std::collections::HashMap;

use chrono::DateTime;
use zbus::OwnedVariant;

use crate::{Address, Conversation, ConversationId, Error, Message, MessageId};

#[zbus::proxy(
    interface = "io.weirdware.BlueFerry.Messages1",
    default_service = "io.weirdware.BlueFerry",
    default_path = "/io/weirdware/BlueFerry"
)]
pub(crate) trait Messages {
    /// JSON array of up to `limit` threads (newest first), each with its messages.
    fn list_threads(&self, limit: u32) -> zbus::Result<String>;
    /// Sends `body` to a single address as a 1:1 message. Returns the OBEX transfer
    /// path (`transfer_path`), an opaque string.
    fn send(&self, recipient: &str, body: &str) -> zbus::Result<String>;
    /// Marks a thread's incoming messages read in BlueFerry and on the phone.
    /// Returns how many messages changed.
    fn mark_thread_read(&self, thread_key: &str) -> zbus::Result<u32>;
    /// Whether the BlueFerry backend reports itself healthy.
    fn is_healthy(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "io.weirdware.BlueFerry.Events1",
    default_service = "io.weirdware.BlueFerry",
    default_path = "/io/weirdware/BlueFerry"
)]
pub(crate) trait Events {
    /// Emitted whenever stored message history changes: a new message, a message
    /// backfilled after a reconnect, or a thread marked read. The payload is only
    /// a revision counter, so it says nothing about *what* changed. Treat it as
    /// "go and look".
    #[zbus(signal)]
    fn history_changed(&self, props: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
}

/// One message as returned inside a `ListThreads` thread.
///
/// Unknown JSON fields are ignored. Every field below is always present in
/// BlueFerry's output, so a missing one is a parse error rather than a guess.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct WireMessage {
    /// Opaque, stable message id.
    pub handle: String,
    /// The other party for 1:1 threads, the actual sender for group threads, the
    /// recipient for outgoing messages. May be empty. Parse this for identity.
    pub address: String,
    pub body: String,
    /// ISO-8601; incoming are UTC, outgoing carry the local offset.
    pub timestamp: String,
    pub outgoing: bool,
    /// Display name of the sender; only set for incoming group messages.
    /// Not an address, so never parse it for identity.
    pub sender: String,
    /// Shared BlueFerry state (phone, any BlueFerry client, popup dismissal).
    pub read: bool,
    pub body_truncated: bool,
}

/// One conversation as returned by `ListThreads`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct WireThread {
    /// Opaque thread key, e.g. `address:phone:14165550123`.
    pub key: String,
    /// Heuristic: group info is correlated from a second Bluetooth channel.
    pub is_group: bool,
    /// Reply addresses. For groups these are the member addresses.
    pub recipients: Vec<String>,
    /// Possibly a 1:1 thread that is really a group (e.g. backfilled messages).
    pub group_ambiguous: bool,
    pub messages: Vec<WireMessage>,
}

impl Message {
    /// Builds a message from its wire form. Never fails: an address we can't
    /// classify becomes `None` instead of dropping the message.
    fn from_wire(wire: WireMessage, conversation: ConversationId, possibly_group: bool) -> Self {
        // For outgoing messages `address` is the recipient, not the sender.
        let sender = if wire.outgoing {
            None
        } else {
            parse_address(&wire.address)
        };
        Self {
            id: MessageId(wire.handle),
            conversation,
            sender,
            sender_name: non_empty(wire.sender),
            text: wire.body,
            text_truncated: wire.body_truncated,
            timestamp: DateTime::parse_from_rfc3339(&wire.timestamp).ok(),
            raw_timestamp: wire.timestamp,
            outgoing: wire.outgoing,
            read: wire.read,
            possibly_group,
        }
    }
}

impl From<WireThread> for Conversation {
    /// Converts a wire thread. Every message inherits the thread's `group_ambiguous`.
    fn from(wire: WireThread) -> Self {
        let id = ConversationId(wire.key);
        let mut messages: Vec<Message> = wire
            .messages
            .into_iter()
            .map(|m| Message::from_wire(m, id.clone(), wire.group_ambiguous))
            .collect();
        // Stable sort by instant; `None` (unparseable) orders first.
        messages.sort_by_key(|m| m.timestamp);
        Self {
            participants: wire
                .recipients
                .iter()
                .filter_map(|r| parse_address(r))
                .collect(),
            is_group: wire.is_group,
            messages,
            id,
        }
    }
}

/// Parses a BlueFerry-reported address, discarding anything we can't classify.
fn parse_address(raw: &str) -> Option<Address> {
    Address::parse(raw).ok()
}

/// `None` for an empty or whitespace-only string.
fn non_empty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

/// Reads up to `limit` threads from BlueFerry and converts them.
pub(crate) async fn fetch_conversations(
    proxy: &MessagesProxy<'_>,
    limit: u32,
) -> Result<Vec<Conversation>, Error> {
    let json = proxy.list_threads(limit).await?;
    let threads: Vec<WireThread> = serde_json::from_str(&json)?;
    Ok(threads.into_iter().map(Conversation::from).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Trimmed from a real `ListThreads` response, with extra fields kept in on purpose.
    const SAMPLE: &str = r#"[{"key":"address:phone:14165550123","aliases":[],"name":"+14165550123",
        "is_group":false,"members":[],"recipients":["+14165550123"],"unread":false,
        "group_ambiguous":false,"last_ts":"2026-11-12T10:18:00.829121-05:00",
        "messages":[{"handle":"message920927130248545412","address":"+14165550123","body":"Test",
        "timestamp":"2026-11-12T03:10:26+00:00","outgoing":false,"sender":"","read":true,
        "body_truncated":false}]}]"#;

    #[test]
    fn missing_field_is_an_error() {
        let json = r#"[{"key":"k","is_group":false,"recipients":[],"group_ambiguous":false,
            "messages":[{"handle":"h","body":"b","timestamp":"t","outgoing":false}]}]"#;
        assert!(serde_json::from_str::<Vec<WireThread>>(json).is_err());
    }

    #[test]
    fn deserializes_real_response() {
        let threads: Vec<WireThread> = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(threads[0].key, "address:phone:14165550123");
        assert!(!threads[0].is_group);
        assert_eq!(threads[0].messages[0].address, "+14165550123");
    }

    /// Parses a `ListThreads`-shaped JSON array into public conversations.
    pub(crate) fn convert(json: &str) -> Vec<Conversation> {
        serde_json::from_str::<Vec<WireThread>>(json)
            .unwrap()
            .into_iter()
            .map(Conversation::from)
            .collect()
    }

    /// Builds a minimal message JSON object for tests.
    pub(crate) fn msg(handle: &str, ts: &str, outgoing: bool) -> String {
        format!(
            r#"{{"handle":"{handle}","address":"+14165550123","body":"b","timestamp":"{ts}",
            "outgoing":{outgoing},"sender":"","read":false,"body_truncated":false}}"#
        )
    }

    /// A one-thread JSON array containing the given message objects.
    pub(crate) fn thread(messages: &[String]) -> String {
        format!(
            r#"[{{"key":"k","is_group":false,"recipients":["+14165550123"],
            "group_ambiguous":false,"messages":[{}]}}]"#,
            messages.join(",")
        )
    }

    const ONE_TO_ONE: &str = r#"[{"key":"address:phone:14165550123","is_group":false,
        "recipients":["+14165550123"],"group_ambiguous":false,"messages":[
        {"handle":"m1","address":"+14165550123","body":"Hi","timestamp":"2026-11-12T03:10:26+00:00",
         "outgoing":false,"sender":"","read":false,"body_truncated":false},
        {"handle":"m2","address":"+14165550123","body":"Hello","timestamp":"2026-11-12T09:55:06-05:00",
         "outgoing":true,"sender":"","read":true,"body_truncated":false}]}]"#;

    #[test]
    fn converts_one_to_one_thread() {
        let c = &convert(ONE_TO_ONE)[0];
        assert_eq!(c.id.as_str(), "address:phone:14165550123");
        assert!(!c.is_group);
        assert_eq!(c.participants.len(), 1);

        let incoming = &c.messages[0];
        assert_eq!(incoming.id, MessageId::new("m1"));
        assert_eq!(incoming.conversation, c.id);
        assert_eq!(incoming.sender.as_ref().unwrap().as_str(), "+14165550123");
        assert_eq!(incoming.sender_name, None);
        assert_eq!(incoming.raw_timestamp, "2026-11-12T03:10:26+00:00");
        assert!(incoming.timestamp.is_some());
        assert!(!incoming.read && !incoming.outgoing && !incoming.possibly_group);
    }

    #[test]
    fn outgoing_message_has_no_sender() {
        let c = &convert(ONE_TO_ONE)[0];
        assert!(c.messages[1].outgoing);
        assert_eq!(c.messages[1].sender, None);
    }

    #[test]
    fn group_message_keeps_email_sender_and_display_name() {
        let json = r#"[{"key":"group:abc","is_group":true,
            "recipients":["+14165550123","bob@example.com"],"group_ambiguous":false,"messages":[
            {"handle":"g1","address":"Bob@Example.com","body":"yo","timestamp":"t",
             "outgoing":false,"sender":"Bob","read":true,"body_truncated":false}]}]"#;
        let c = &convert(json)[0];
        assert!(c.is_group);
        assert_eq!(c.participants.len(), 2);
        let m = &c.messages[0];
        assert_eq!(m.sender, Some(Address::parse("bob@example.com").unwrap()));
        assert_eq!(m.sender_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn unusable_address_becomes_none_instead_of_failing() {
        let json = r#"[{"key":"k","is_group":true,"recipients":["Mom","+14165550123"],
            "group_ambiguous":true,"messages":[
            {"handle":"h","address":"","body":"b","timestamp":"t","outgoing":false,
             "sender":"Someone","read":true,"body_truncated":true}]}]"#;
        let c = &convert(json)[0];
        assert_eq!(c.participants.len(), 1); // "Mom" dropped
        let m = &c.messages[0];
        assert_eq!(m.sender, None);
        assert_eq!(m.sender_name.as_deref(), Some("Someone"));
        assert!(m.text_truncated);
        assert!(m.possibly_group); // inherited from the thread
        assert_eq!(m.timestamp, None); // "t" doesn't parse...
        assert_eq!(m.raw_timestamp, "t"); // ...but is kept
    }

    #[test]
    fn conversation_messages_are_ordered_oldest_first_by_instant() {
        let json = thread(&[
            msg("later", "2026-11-12T09:00:00-05:00", false), // 14:00 UTC
            msg("earlier", "2026-11-12T13:00:00+00:00", false),
        ]);
        let ids: Vec<_> = convert(&json)[0]
            .messages
            .iter()
            .map(|m| m.id.to_string())
            .collect();
        assert_eq!(ids, ["earlier", "later"]);
    }
}
