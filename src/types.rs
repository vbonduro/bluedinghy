//! The public data types: ids, messages and conversations.

use std::fmt;

use chrono::{DateTime, FixedOffset};

use crate::Address;

/// Stable id of one message (BlueFerry's `handle`). Use it as the idempotency
/// key when recording which messages the app has handled.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(pub(crate) String);

/// Opaque key of one conversation (BlueFerry's thread key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationId(pub(crate) String);

/// Opaque handle for an outgoing transfer, as returned by
/// [`send`](crate::BlueDinghy::send) (BlueFerry's `transfer_path`).
///
/// This is **not** a [`MessageId`]: nothing guarantees it matches the `handle`
/// the sent message later gets in history, so don't use it to recognise the
/// message there.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId(pub(crate) String);

/// Gives an id newtype `new`, `as_str` and `Display`.
macro_rules! string_id {
    ($t:ident) => {
        impl $t {
            /// Wraps a raw id string, e.g. one the app stored earlier.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            /// The raw id string, suitable for storing.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
string_id!(MessageId);
string_id!(ConversationId);
string_id!(TransferId);

/// One message, incoming or outgoing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Message {
    pub id: MessageId,
    pub conversation: ConversationId,
    /// Who sent it. `None` for outgoing messages (we are the sender), and for
    /// incoming ones whose address BlueFerry didn't report or we can't classify.
    pub sender: Option<Address>,
    /// Display name of the sender, only available for incoming group messages.
    /// For display only; never use it to identify anyone.
    pub sender_name: Option<String>,
    pub text: String,
    /// True if BlueFerry cut `text` short.
    pub text_truncated: bool,
    /// When the message was sent, or `None` if BlueFerry's timestamp didn't
    /// parse as RFC 3339 (see [`raw_timestamp`](Self::raw_timestamp)). Incoming
    /// messages are UTC, outgoing carry the local offset; compare as instants.
    pub timestamp: Option<DateTime<FixedOffset>>,
    /// The timestamp string exactly as BlueFerry reported it.
    pub raw_timestamp: String,
    pub outgoing: bool,
    /// Whether BlueFerry considers it read. This is shared state (phone, any
    /// BlueFerry client, popup dismissal), so don't use it to track what the app
    /// has handled.
    pub read: bool,
    /// The conversation looks 1:1 but may really be a group (e.g. a message
    /// backfilled after a disconnect). A reply to `sender` alone could miss the group.
    pub possibly_group: bool,
}

/// A conversation: a 1:1 thread or a group, with its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Conversation {
    pub id: ConversationId,
    /// Heuristic: BlueFerry infers groups by correlating two Bluetooth channels.
    pub is_group: bool,
    /// Addresses a reply goes to: the other party for 1:1, the members for a group.
    /// Entries we can't classify are omitted.
    pub participants: Vec<Address>,
    /// Oldest first, ordered by instant. A message with an unparseable timestamp
    /// sorts before the rest.
    pub messages: Vec<Message>,
}
