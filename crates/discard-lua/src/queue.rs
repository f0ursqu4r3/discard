//! A bounded queue with two kinds of entries. Requests are rejected when the queue is full.
//! Events are lossy: when the queue is full, the oldest event is dropped to make room.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Event,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PushOutcome {
    Queued,
    /// The entry was queued after dropping this many older events.
    QueuedDropped(usize),
    /// The queue is full and the entry was not stored.
    Rejected,
    Closed,
}

pub struct BoundedQueue<T> {
    cap: usize,
    inner: Mutex<Inner<T>>,
    ready: Condvar,
}

struct Inner<T> {
    items: VecDeque<(Kind, T)>,
    closed: bool,
}

impl<T> BoundedQueue<T> {
    pub fn new(cap: usize) -> Self {
        Self { cap, inner: Mutex::new(Inner { items: VecDeque::new(), closed: false }), ready: Condvar::new() }
    }

    pub fn push(&self, kind: Kind, item: T) -> PushOutcome {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.closed {
            return PushOutcome::Closed;
        }
        let mut dropped = 0;
        if inner.items.len() >= self.cap {
            // Only an event may displace an older event.
            let oldest_event = (kind == Kind::Event)
                .then(|| inner.items.iter().position(|(k, _)| *k == Kind::Event))
                .flatten();
            match oldest_event {
                Some(i) => {
                    inner.items.remove(i);
                    dropped = 1;
                }
                None => return PushOutcome::Rejected,
            }
        }
        inner.items.push_back((kind, item));
        drop(inner);
        self.ready.notify_one();
        if dropped > 0 { PushOutcome::QueuedDropped(dropped) } else { PushOutcome::Queued }
    }

    /// Wait up to `timeout` for the next entry. `None` on timeout, or when closed and empty.
    pub fn pop(&self, timeout: Duration) -> Option<T> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.items.is_empty() && !inner.closed {
            inner = self.ready.wait_timeout(inner, timeout).unwrap_or_else(|p| p.into_inner()).0;
        }
        inner.items.pop_front().map(|(_, item)| item)
    }

    pub fn close(&self) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).closed = true;
        self.ready.notify_all();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const SHORT: Duration = Duration::from_millis(20);

    #[test]
    fn entries_come_out_in_order() {
        let q = BoundedQueue::new(4);
        for i in 0..3 {
            assert_eq!(q.push(Kind::Request, i), PushOutcome::Queued);
        }
        assert_eq!((q.pop(SHORT), q.pop(SHORT), q.pop(SHORT)), (Some(0), Some(1), Some(2)));
        assert_eq!(q.pop(SHORT), None);
    }

    #[test]
    fn requests_are_rejected_when_full() {
        let q = BoundedQueue::new(2);
        q.push(Kind::Request, 1);
        q.push(Kind::Request, 2);
        assert_eq!(q.push(Kind::Request, 3), PushOutcome::Rejected);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn a_full_queue_drops_the_oldest_event_for_a_new_event() {
        let q = BoundedQueue::new(2);
        q.push(Kind::Event, "e1");
        q.push(Kind::Event, "e2");
        assert_eq!(q.push(Kind::Event, "e3"), PushOutcome::QueuedDropped(1));
        assert_eq!((q.pop(SHORT), q.pop(SHORT)), (Some("e2"), Some("e3")));
    }

    #[test]
    fn events_never_evict_requests() {
        let q = BoundedQueue::new(2);
        q.push(Kind::Request, "r1");
        q.push(Kind::Request, "r2");
        assert_eq!(q.push(Kind::Event, "e"), PushOutcome::Rejected);
        assert_eq!((q.pop(SHORT), q.pop(SHORT)), (Some("r1"), Some("r2")));
    }

    #[test]
    fn a_new_request_does_not_evict_events_either() {
        let q = BoundedQueue::new(1);
        q.push(Kind::Event, "e");
        assert_eq!(q.push(Kind::Request, "r"), PushOutcome::Rejected);
        assert_eq!(q.pop(SHORT), Some("e"));
    }

    #[test]
    fn pop_wakes_when_an_entry_arrives() {
        let q = Arc::new(BoundedQueue::new(2));
        let q2 = Arc::clone(&q);
        let t = std::thread::spawn(move || q2.pop(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(30));
        q.push(Kind::Request, 7);
        assert_eq!(t.join().unwrap(), Some(7));
    }

    #[test]
    fn close_wakes_waiters_rejects_pushes_and_drains_remaining_entries() {
        let q = Arc::new(BoundedQueue::new(4));
        q.push(Kind::Request, 1);
        q.close();
        assert!(q.is_closed());
        assert_eq!(q.push(Kind::Request, 2), PushOutcome::Closed);
        assert_eq!(q.pop(SHORT), Some(1));
        let q2 = Arc::clone(&q);
        let waiter = std::thread::spawn(move || q2.pop(Duration::from_secs(5)));
        assert_eq!(waiter.join().unwrap(), None, "closed and empty must return immediately");
    }
}
