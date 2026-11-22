//! [`BlueDinghy`]: your handle on a running BlueFerry.

use futures_util::StreamExt;

use crate::dbus::{EventsProxy, MessagesProxy, fetch_conversations};
use crate::history::{messages, newest_incoming, take_new};
use crate::watch::{MessageStream, WatchOptions, watch_stream};
use crate::{Address, Conversation, ConversationId, Error, Message, TransferId};

/// Your handle on a running BlueFerry: read history, watch for new messages and send.
///
/// Cheap to clone: clones share the underlying D-Bus connection.
#[derive(Clone)]
pub struct BlueDinghy {
    messages: MessagesProxy<'static>,
    events: EventsProxy<'static>,
}

impl std::fmt::Debug for BlueDinghy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlueDinghy").finish_non_exhaustive()
    }
}

impl BlueDinghy {
    /// Connects to the BlueFerry service on the session bus.
    ///
    /// # Errors
    /// Fails if the session bus is unreachable or the proxies can't be created.
    pub async fn new() -> Result<Self, Error> {
        let connection = zbus::Connection::session().await?;
        Ok(Self {
            messages: MessagesProxy::new(&connection).await?,
            events: EventsProxy::new(&connection).await?,
        })
    }

    /// Whether BlueFerry reports its backend as healthy. A cheap way to answer
    /// "is BlueFerry up?" before doing real work.
    ///
    /// # Errors
    /// [`Error::NotRunning`] if BlueFerry isn't on the bus; otherwise as for the
    /// other calls.
    pub async fn is_healthy(&self) -> Result<bool, Error> {
        Ok(self.messages.is_healthy().await?)
    }

    /// Returns up to `limit` conversations, newest first, with their messages.
    ///
    /// `limit` counts threads, not messages, and BlueFerry caps it at 1000.
    ///
    /// # Errors
    /// Fails if the D-Bus call fails ([`Error::Dbus`], [`Error::NotRunning`],
    /// [`Error::Refused`]) or BlueFerry's JSON doesn't match the expected shape
    /// ([`Error::Protocol`]).
    pub async fn conversations(&self, limit: u32) -> Result<Vec<Conversation>, Error> {
        fetch_conversations(&self.messages, limit).await
    }

    /// Returns the newest `n` incoming messages, oldest first.
    ///
    /// The read flag is deliberately ignored: it is shared state and says nothing
    /// about whether this app has handled a message. Filter by
    /// [`MessageId`](crate::MessageId) against the app's own record instead.
    ///
    /// Looks across all conversations BlueFerry will list (up to 1000 threads), so
    /// a busy inbox won't push an older thread out of the window.
    ///
    /// # Errors
    /// Same as [`conversations`](Self::conversations).
    pub async fn recent_messages(&self, n: usize) -> Result<Vec<Message>, Error> {
        let conversations = self.conversations(WatchOptions::MAX_THREADS).await?;
        Ok(newest_incoming(messages(conversations), n))
    }

    /// Streams each new incoming message, with default [`WatchOptions`].
    /// See [`watch_with`](Self::watch_with).
    pub async fn watch(&self) -> Result<MessageStream, Error> {
        self.watch_with(WatchOptions::default()).await
    }

    /// Streams each new incoming message as it arrives.
    ///
    /// Messages already in history when this is called are *not* delivered; fetch
    /// those with [`recent_messages`](Self::recent_messages). To avoid a gap, call
    /// `watch` first and `recent_messages` second. A message that arrives in
    /// between may then be seen by both, so delivery is **at-least-once**:
    /// deduplicate on [`MessageId`](crate::MessageId).
    ///
    /// Things to know:
    /// - A message backfilled after a reconnect, with an old timestamp, is
    ///   delivered as new. Within one refresh messages come oldest first by
    ///   timestamp, which is not necessarily the order they arrived in.
    /// - Every `HistoryChanged` signal (a new message, a backfill, or a thread
    ///   marked read) re-reads up to `max_threads` threads with all their
    ///   messages. Raise `settle` to trade latency for fewer reads.
    /// - The stream remembers the ids it has delivered, bounded by the history
    ///   window, so memory use doesn't grow over time.
    ///
    /// The stream does the work as it is polled, so a slow consumer delays the
    /// next refresh rather than queueing unboundedly.
    ///
    /// # Errors
    /// Fails if the signal subscription or the initial history read fails.
    /// Later refresh failures arrive as `Err` items instead.
    pub async fn watch_with(&self, options: WatchOptions) -> Result<MessageStream, Error> {
        // Subscribe before seeding, so nothing that arrives in between is lost.
        let signals = self.events.receive_history_changed().await?.map(|_| ());

        let proxy = self.messages.clone();
        let max_threads = options.max_threads;
        let mut seen = std::collections::HashSet::new();
        take_new(
            &mut seen,
            messages(fetch_conversations(&proxy, max_threads).await?),
        );

        let fetch = move || {
            let proxy = proxy.clone();
            async move { fetch_conversations(&proxy, max_threads).await }
        };
        Ok(MessageStream::new(watch_stream(
            signals, fetch, seen, options,
        )))
    }

    /// Sends `text` to `to` as a new 1:1 message.
    ///
    /// Returns BlueFerry's opaque transfer id. This does not send into an
    /// existing group conversation, even if `to` is a member.
    ///
    /// # Errors
    /// Fails if the phone isn't connected for messaging, the text is rejected
    /// (e.g. empty or too long) or the D-Bus call fails. Check
    /// [`Error::code`] for `NotReady`, `RateLimited`, `SendFailed` and
    /// `SendOutcomeUnknown` (the message may still have gone out).
    pub async fn send(&self, to: &Address, text: &str) -> Result<TransferId, Error> {
        Ok(TransferId(self.messages.send(to.as_str(), text).await?))
    }

    /// Marks a conversation's incoming messages read, in BlueFerry and on the
    /// phone. Returns how many messages changed.
    ///
    /// # Errors
    /// Same as [`send`](Self::send) for the D-Bus failure modes.
    pub async fn mark_conversation_read(
        &self,
        conversation: &ConversationId,
    ) -> Result<u32, Error> {
        Ok(self
            .messages
            .mark_thread_read(conversation.as_str())
            .await?)
    }
}
