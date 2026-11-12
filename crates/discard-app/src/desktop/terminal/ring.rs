//! A bounded byte buffer that keeps the newest output of a terminal session.
use std::collections::VecDeque;

/// Output kept for a session. About 1 MiB.
pub const SCROLLBACK_BYTES: usize = 1024 * 1024;

pub struct Ring {
    capacity: usize,
    bytes: VecDeque<u8>,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, bytes: VecDeque::new() }
    }

    /// Append `data`. The oldest bytes go when the buffer is full.
    pub fn push(&mut self, data: &[u8]) {
        if self.capacity == 0 {
            return;
        }
        if data.len() >= self.capacity {
            self.bytes.clear();
            self.bytes.extend(&data[data.len() - self.capacity..]);
            return;
        }
        let overflow = (self.bytes.len() + data.len()).saturating_sub(self.capacity);
        self.bytes.drain(..overflow);
        self.bytes.extend(data);
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The kept bytes, oldest first. A cut can split a UTF-8 sequence at the start,
    /// so the copy skips the leftover continuation bytes.
    pub fn snapshot(&self) -> Vec<u8> {
        let start = self.bytes.iter().take(3).take_while(|byte| **byte & 0xC0 == 0x80).count();
        self.bytes.iter().skip(start).copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_everything_below_capacity() {
        let mut ring = Ring::new(8);
        ring.push(b"abc");
        ring.push(b"def");
        assert_eq!(ring.snapshot(), b"abcdef");
        assert_eq!(ring.len(), 6);
    }

    #[test]
    fn drops_the_oldest_bytes_at_capacity() {
        let mut ring = Ring::new(5);
        ring.push(b"abc");
        ring.push(b"defg");
        assert_eq!(ring.snapshot(), b"cdefg");
        ring.push(b"h");
        assert_eq!(ring.snapshot(), b"defgh");
    }

    #[test]
    fn a_chunk_larger_than_capacity_keeps_its_tail() {
        let mut ring = Ring::new(4);
        ring.push(b"xy");
        ring.push(b"0123456789");
        assert_eq!(ring.snapshot(), b"6789");
        assert_eq!(ring.len(), 4);
    }

    #[test]
    fn a_zero_capacity_ring_keeps_nothing() {
        let mut ring = Ring::new(0);
        ring.push(b"abc");
        assert!(ring.is_empty());
    }

    #[test]
    fn the_snapshot_never_starts_inside_a_utf8_sequence() {
        // "é" is 0xC3 0xA9. A cut after its first byte leaves a lone continuation byte.
        let mut ring = Ring::new(3);
        ring.push("éxy".as_bytes());
        assert_eq!(ring.snapshot(), b"xy");
        let mut whole = Ring::new(4);
        whole.push("éxy".as_bytes());
        assert_eq!(whole.snapshot(), "éxy".as_bytes());
    }

    #[test]
    fn stays_bounded_under_many_pushes() {
        let mut ring = Ring::new(SCROLLBACK_BYTES);
        for _ in 0..40 {
            ring.push(&[b'x'; 100_000]);
        }
        assert_eq!(ring.len(), SCROLLBACK_BYTES);
    }
}
