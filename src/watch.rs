//! Turning BlueFerry's "history changed" signals into a stream of new messages.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::stream::{self, Fuse};
use futures_util::{Stream, StreamExt};
use tokio::time::{Instant, timeout};

use crate::history::{messages, take_new};
use crate::{Conversation, Error, Message, MessageId};

/// Tuning for [`BlueDinghy::watch_with`](crate::BlueDinghy::watch_with).
///
/// Start from [`WatchOptions::default`] and adjust with the builder methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WatchOptions {
    /// After a `HistoryChanged` signal, wait this long for further signals before
    /// re-reading history. One incoming message usually raises several in a row.
    pub settle: Duration,
    /// Upper bound on that wait. A steady drip of signals can't postpone a
    /// refresh for longer than this.
    pub max_wait: Duration,
    /// Most threads to read on each refresh. BlueFerry caps this at 1000.
    pub max_threads: u32,
}

impl WatchOptions {
    /// Most threads BlueFerry will return from one `ListThreads` call.
    pub(crate) const MAX_THREADS: u32 = 1_000;

    /// Sets [`settle`](Self::settle).
    pub fn settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }
    /// Sets [`max_wait`](Self::max_wait).
    pub fn max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = max_wait;
        self
    }
    /// Sets [`max_threads`](Self::max_threads).
    pub fn max_threads(mut self, max_threads: u32) -> Self {
        self.max_threads = max_threads;
        self
    }
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            settle: Duration::from_millis(200),
            max_wait: Duration::from_secs(2),
            max_threads: Self::MAX_THREADS,
        }
    }
}

/// A stream of new incoming messages, oldest first within each refresh.
///
/// Created by [`BlueDinghy::watch`](crate::BlueDinghy::watch).
/// Each item is a message or the error from a failed history refresh; an error
/// doesn't end the stream, and the refresh is retried on the next signal. The
/// stream ends when BlueFerry's signal stream closes (e.g. the daemon exits).
/// Drop it to stop watching.
///
/// Polling it needs a Tokio runtime with the time driver enabled.
pub struct MessageStream {
    inner: Pin<Box<dyn Stream<Item = Result<Message, Error>> + Send>>,
}

impl MessageStream {
    pub(crate) fn new(inner: impl Stream<Item = Result<Message, Error>> + Send + 'static) -> Self {
        Self {
            inner: Box::pin(inner),
        }
    }
}

impl Stream for MessageStream {
    type Item = Result<Message, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl std::fmt::Debug for MessageStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageStream").finish_non_exhaustive()
    }
}

/// Swallows the rest of a burst of signals, for at most `options.max_wait`.
async fn settle<S: Stream<Item = ()> + Unpin>(signals: &mut Fuse<S>, options: &WatchOptions) {
    let deadline = Instant::now() + options.max_wait;
    loop {
        let wait = options
            .settle
            .min(deadline.saturating_duration_since(Instant::now()));
        if wait.is_zero() {
            return;
        }
        match timeout(wait, signals.next()).await {
            Ok(Some(())) => continue,
            // Quiet for `settle`, or the stream ended: go and read.
            _ => return,
        }
    }
}

/// The watch loop: on each (debounced) signal, calls `fetch` and yields the
/// messages not yet in `seen`.
///
/// Generic over the signal source and the fetch so it can be tested without D-Bus.
pub(crate) fn watch_stream<S, F, Fut>(
    signals: S,
    fetch: F,
    seen: HashSet<MessageId>,
    options: WatchOptions,
) -> impl Stream<Item = Result<Message, Error>> + Send
where
    S: Stream<Item = ()> + Unpin + Send,
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = Result<Vec<Conversation>, Error>> + Send,
{
    struct State<S, F> {
        signals: S,
        fetch: F,
        seen: HashSet<MessageId>,
        options: WatchOptions,
        pending: VecDeque<Result<Message, Error>>,
    }

    // Fused: `settle` may see the end of the signals, and the loop then polls them again.
    let state = State {
        signals: signals.fuse(),
        fetch,
        seen,
        options,
        pending: VecDeque::new(),
    };
    stream::unfold(state, |mut st| async move {
        loop {
            if let Some(item) = st.pending.pop_front() {
                return Some((item, st));
            }
            st.signals.next().await?;
            settle(&mut st.signals, &st.options).await;
            match (st.fetch)().await {
                Ok(conversations) => {
                    let fresh = take_new(&mut st.seen, messages(conversations));
                    st.pending.extend(fresh.into_iter().map(Ok));
                }
                Err(error) => st.pending.push_back(Err(error)),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::mpsc;

    use super::*;
    use crate::dbus::tests::{convert, msg, thread};

    /// A signal source fed by the test.
    fn channel() -> (
        mpsc::UnboundedSender<()>,
        impl Stream<Item = ()> + Unpin + Send,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            tx,
            Box::pin(stream::unfold(rx, |mut rx| async {
                rx.recv().await.map(|v| (v, rx))
            })),
        )
    }

    fn one_message(handle: &str) -> Vec<Conversation> {
        convert(&thread(&[msg(handle, "2026-11-12T10:00:00+00:00", false)]))
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_signals_causes_one_refresh() {
        let fetches = Arc::new(AtomicUsize::new(0));
        let counter = fetches.clone();
        let items: Vec<_> = watch_stream(
            stream::iter([(), (), ()]),
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok(one_message("a")) }
            },
            HashSet::new(),
            WatchOptions::default(),
        )
        .collect()
        .await;

        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].as_ref().unwrap().id, MessageId::new("a"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_is_reported_and_the_stream_continues() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let (tx, signals) = channel();
        let mut stream = Box::pin(watch_stream(
            signals,
            move || {
                let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
                async move {
                    if first {
                        Err(Error::NotRunning)
                    } else {
                        Ok(one_message("a"))
                    }
                }
            },
            HashSet::new(),
            WatchOptions::default(),
        ));

        tx.send(()).unwrap();
        assert!(matches!(stream.next().await, Some(Err(Error::NotRunning))));
        tx.send(()).unwrap();
        assert_eq!(
            stream.next().await.unwrap().unwrap().id,
            MessageId::new("a")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn already_seen_messages_are_not_delivered() {
        let (tx, signals) = channel();
        let seen = HashSet::from([MessageId::new("a")]);
        let mut stream = Box::pin(watch_stream(
            signals,
            || async { Ok(one_message("a")) },
            seen,
            WatchOptions::default(),
        ));
        tx.send(()).unwrap();
        drop(tx); // the stream ends after the refresh
        assert!(stream.next().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_steady_drip_of_signals_cannot_postpone_a_refresh_forever() {
        let (tx, signals) = channel();
        // A signal every 100ms: always inside the 200ms settle window.
        tokio::spawn(async move {
            while tx.send(()).is_ok() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        let options = WatchOptions::default()
            .settle(Duration::from_millis(200))
            .max_wait(Duration::from_secs(1));
        let mut stream = Box::pin(watch_stream(
            signals,
            || async { Ok(one_message("a")) },
            HashSet::new(),
            options,
        ));

        let start = Instant::now();
        assert!(stream.next().await.unwrap().is_ok());
        assert!(
            start.elapsed() <= Duration::from_millis(1_100),
            "waited {:?}",
            start.elapsed()
        );
    }
}
