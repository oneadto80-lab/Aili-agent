use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Per-frame token batching: queue arrivals with timestamps, drain when the
/// oldest is too old or the queue is too long. Keeps redraws to ~10/s during
/// fast streams without visible delay.
pub struct StreamState {
    pending: VecDeque<(Instant, String)>,
    max_age: Duration,
    max_len: usize,
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            max_age: Duration::from_millis(80),
            max_len: 16,
        }
    }
    pub fn enqueue(&mut self, t: String) {
        self.pending.push_back((Instant::now(), t));
    }
    pub fn ready(&self) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        if self.pending.len() >= self.max_len {
            return true;
        }
        self.pending
            .front()
            .map(|(at, _)| at.elapsed() >= self.max_age)
            .unwrap_or(false)
    }
    pub fn drain_pending(&mut self) -> String {
        self.drain_all()
    }
    pub fn drain_all(&mut self) -> String {
        let mut s = String::new();
        while let Some((_, t)) = self.pending.pop_front() {
            s.push_str(&t);
        }
        s
    }
    #[allow(dead_code)]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_all_flushes_tokens_before_ready_threshold() {
        let mut stream = StreamState::new();
        stream.enqueue("你的吗".to_string());
        assert!(!stream.ready());
        assert_eq!(stream.drain_all(), "你的吗");
        assert!(!stream.has_pending());
    }
}
