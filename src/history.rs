//! Pure functions over message history: no D-Bus, easy to test.

use std::collections::HashSet;

use crate::{Conversation, Message, MessageId};

/// Flattens conversations into one stream of their messages.
pub(crate) fn messages(conversations: Vec<Conversation>) -> impl Iterator<Item = Message> {
    conversations.into_iter().flat_map(|c| c.messages)
}

/// Picks the newest `n` incoming messages from `messages`, oldest first.
///
/// Outgoing messages are dropped. Order is by instant (incoming timestamps are
/// UTC, outgoing carry a local offset, so string order would be wrong). A message
/// whose timestamp doesn't parse sorts before all the others.
pub(crate) fn newest_incoming(
    messages: impl IntoIterator<Item = Message>,
    n: usize,
) -> Vec<Message> {
    let mut incoming: Vec<Message> = messages.into_iter().filter(|m| !m.outgoing).collect();
    // `Option` orders `None` first, which gives the "unparseable sorts first" rule.
    incoming.sort_by_key(|m| m.timestamp);
    let skip = incoming.len().saturating_sub(n);
    incoming.split_off(skip)
}

/// Returns the incoming messages in `messages` that are not in `seen`, oldest
/// first, and updates `seen`.
///
/// Calling it once with the current history seeds `seen`, so later calls report
/// only what arrived since. Outgoing messages are ignored.
///
/// `seen` is bounded: after each call it is replaced by the ids in this snapshot,
/// so ids that have left BlueFerry's history window are forgotten. The exception
/// is a snapshot less than half the size of `seen`, which looks like a transient
/// empty or partial listing (e.g. while BlueFerry reconnects); then `seen` is
/// kept, so the whole history isn't delivered a second time when it comes back.
pub(crate) fn take_new(
    seen: &mut HashSet<MessageId>,
    messages: impl IntoIterator<Item = Message>,
) -> Vec<Message> {
    let incoming: Vec<Message> = messages.into_iter().filter(|m| !m.outgoing).collect();
    let snapshot: HashSet<MessageId> = incoming.iter().map(|m| m.id.clone()).collect();
    let known_before = seen.len();

    let mut fresh: Vec<Message> = incoming
        .into_iter()
        .filter(|m| seen.insert(m.id.clone()))
        .collect();
    fresh.sort_by_key(|m| m.timestamp);

    if snapshot.len() * 2 >= known_before {
        *seen = snapshot;
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbus::tests::{convert, msg, thread};

    fn ids(messages: &[Message]) -> Vec<String> {
        messages.iter().map(|m| m.id.to_string()).collect()
    }

    #[test]
    fn take_new_reports_only_unseen_incoming_messages() {
        let mut seen = HashSet::new();
        let first = thread(&[
            msg("a", "2026-11-12T10:00:00+00:00", false),
            msg("out", "2026-11-12T10:05:00+00:00", true),
        ]);
        // First call seeds: everything incoming is new, outgoing is ignored.
        assert_eq!(take_new(&mut seen, messages(convert(&first))).len(), 1);
        // Same history again: nothing new.
        assert!(take_new(&mut seen, messages(convert(&first))).is_empty());

        // One more arrives: only it is reported.
        let second = thread(&[
            msg("a", "2026-11-12T10:00:00+00:00", false),
            msg("b", "2026-11-12T11:00:00+00:00", false),
        ]);
        assert_eq!(ids(&take_new(&mut seen, messages(convert(&second)))), ["b"]);
    }

    #[test]
    fn take_new_forgets_ids_that_left_history() {
        let mut seen = HashSet::new();
        let both = thread(&[
            msg("a", "2026-11-12T10:00:00+00:00", false),
            msg("b", "2026-11-12T11:00:00+00:00", false),
        ]);
        take_new(&mut seen, messages(convert(&both)));
        assert_eq!(seen.len(), 2);

        // "a" aged out of the window; the snapshot is half the size, so it replaces `seen`.
        let only_b = thread(&[msg("b", "2026-11-12T11:00:00+00:00", false)]);
        assert!(take_new(&mut seen, messages(convert(&only_b))).is_empty());
        assert_eq!(seen, HashSet::from([MessageId::new("b")]));
    }

    #[test]
    fn take_new_keeps_seen_across_a_transient_empty_listing() {
        let mut seen = HashSet::new();
        let history = thread(&[
            msg("a", "2026-11-12T10:00:00+00:00", false),
            msg("b", "2026-11-12T11:00:00+00:00", false),
            msg("c", "2026-11-12T12:00:00+00:00", false),
        ]);
        take_new(&mut seen, messages(convert(&history)));

        assert!(take_new(&mut seen, std::iter::empty()).is_empty()); // BlueFerry blips
        // History comes back unchanged: nothing is redelivered.
        assert!(take_new(&mut seen, messages(convert(&history))).is_empty());
    }

    #[test]
    fn newest_incoming_drops_outgoing_and_keeps_last_n() {
        let json = thread(&[
            msg("a", "2026-11-12T10:00:00+00:00", false),
            msg("out", "2026-11-12T10:30:00+00:00", true),
            msg("b", "2026-11-12T11:00:00+00:00", false),
            msg("c", "2026-11-12T12:00:00+00:00", false),
        ]);
        assert_eq!(
            ids(&newest_incoming(messages(convert(&json)), 2)),
            ["b", "c"]
        );
    }

    #[test]
    fn newest_incoming_sorts_by_instant_not_string() {
        // 09:00-05:00 is 14:00 UTC, which is later than 13:00 UTC even though
        // "09:00..." sorts before "13:00..." as a string.
        let json = thread(&[
            msg("later", "2026-11-12T09:00:00-05:00", false),
            msg("earlier", "2026-11-12T13:00:00+00:00", false),
        ]);
        assert_eq!(
            ids(&newest_incoming(messages(convert(&json)), 10)),
            ["earlier", "later"]
        );
    }

    #[test]
    fn newest_incoming_sorts_unparseable_timestamp_first() {
        let json = thread(&[
            msg("good", "2026-11-12T10:00:00+00:00", false),
            msg("bad", "not a time", false),
        ]);
        assert_eq!(
            ids(&newest_incoming(messages(convert(&json)), 10)),
            ["bad", "good"]
        );
    }

    #[test]
    fn newest_incoming_handles_n_larger_than_available_and_zero() {
        let json = thread(&[msg("a", "2026-11-12T10:00:00+00:00", false)]);
        assert_eq!(newest_incoming(messages(convert(&json)), 50).len(), 1);
        assert!(newest_incoming(messages(convert(&json)), 0).is_empty());
    }
}
