//! Small bounded queues used at the GStreamer/IPC boundary.
//!
//! Capture must never be allowed to grow memory without bound when the WebView
//! is busy. New frames replace the oldest frame, which keeps the preview close
//! to real time instead of replaying stale video.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub struct BoundedQueue<T> {
    capacity: usize,
    items: Mutex<VecDeque<T>>,
    wake: Condvar,
}

impl<T> BoundedQueue<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "bounded queue capacity must be positive");
        Self {
            capacity,
            items: Mutex::new(VecDeque::with_capacity(capacity)),
            wake: Condvar::new(),
        }
    }

    /// Push a value and drop the oldest value when the queue is full.
    pub fn push_drop_oldest(&self, value: T) -> bool {
        let mut items = self.items.lock().expect("queue mutex poisoned");
        let dropped = if items.len() == self.capacity {
            items.pop_front();
            true
        } else {
            false
        };
        items.push_back(value);
        self.wake.notify_one();
        dropped
    }

    /// Wait briefly for a value. A timeout lets worker threads observe stop
    /// without requiring a sentinel value or an unbounded channel.
    pub fn pop_timeout(&self, timeout: Duration) -> Option<T> {
        let mut items = self.items.lock().expect("queue mutex poisoned");
        if items.is_empty() {
            let (guard, _) = self
                .wake
                .wait_timeout(items, timeout)
                .expect("queue mutex poisoned");
            items = guard;
        }
        items.pop_front()
    }

    /// Remove all queued values when a media session ends. Signals and frames
    /// belong to one session and must never leak into its replacement.
    pub fn clear(&self) {
        self.items.lock().expect("queue mutex poisoned").clear();
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.lock().expect("queue mutex poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedQueue;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn keeps_only_the_newest_values() {
        let queue = BoundedQueue::new(2);
        queue.push_drop_oldest(1);
        queue.push_drop_oldest(2);
        queue.push_drop_oldest(3);
        assert_eq!(queue.pop_timeout(Duration::ZERO), Some(2));
        assert_eq!(queue.pop_timeout(Duration::ZERO), Some(3));
        assert_eq!(queue.pop_timeout(Duration::ZERO), None);
    }

    #[test]
    fn supports_producer_and_consumer_threads() {
        let queue = Arc::new(BoundedQueue::new(2));
        let producer_queue = Arc::clone(&queue);
        let producer = std::thread::spawn(move || producer_queue.push_drop_oldest(42));
        let value = queue.pop_timeout(Duration::from_secs(1));
        producer.join().expect("producer should finish");
        assert_eq!(value, Some(42));
    }

    #[test]
    fn clears_values_between_sessions() {
        let queue = BoundedQueue::new(2);
        queue.push_drop_oldest(1);
        queue.clear();
        assert_eq!(queue.pop_timeout(Duration::ZERO), None);
    }
}
