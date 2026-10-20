use std::collections::HashMap;

use tex_reader::Reason;

pub const BUDGET_BYTES: u64 = 192 * 1024 * 1024;

pub enum State<T> {
    Pending,
    Ready(T),
    Failed(Reason),
}

/// Decoded images by source, within a byte budget; results of an older generation are ignored.
pub struct Cache<T> {
    entries: HashMap<String, State<T>>,
    used: u64,
    limit: u64,
    generation: u64,
}

impl<T> Cache<T> {
    pub fn new(limit: u64) -> Cache<T> {
        Cache {
            entries: HashMap::new(),
            used: 0,
            limit,
            generation: 0,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, key: &str) -> Option<&State<T>> {
        self.entries.get(key)
    }

    /// Marks `key` as being fetched; false when it already is, was fetched or failed.
    pub fn begin(&mut self, key: &str) -> bool {
        if self.entries.contains_key(key) {
            return false;
        }
        self.entries.insert(key.to_owned(), State::Pending);
        true
    }

    pub fn finish_ok(&mut self, generation: u64, key: &str, value: T, bytes: u64) {
        if generation != self.generation {
            return;
        }
        let state = if self.used + bytes > self.limit {
            State::Failed(Reason::Budget)
        } else {
            self.used += bytes;
            State::Ready(value)
        };
        self.entries.insert(key.to_owned(), state);
    }

    pub fn finish_err(&mut self, generation: u64, key: &str, reason: Reason) {
        if generation == self.generation {
            self.entries.insert(key.to_owned(), State::Failed(reason));
        }
    }

    pub fn reset(&mut self) {
        self.entries.clear();
        self.used = 0;
        self.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_asks_for_a_fetch_only_once() {
        let mut cache: Cache<u32> = Cache::new(100);
        assert!(cache.begin("a"));
        assert!(!cache.begin("a"));
        assert!(matches!(cache.get("a"), Some(State::Pending)));
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn finishing_stores_the_value_and_counts_the_budget() {
        let mut cache: Cache<u32> = Cache::new(100);
        let generation = cache.generation();
        cache.begin("a");
        cache.begin("b");
        cache.finish_ok(generation, "a", 7, 60);
        cache.finish_ok(generation, "b", 8, 60);
        assert!(matches!(cache.get("a"), Some(State::Ready(7))));
        assert!(matches!(
            cache.get("b"),
            Some(State::Failed(Reason::Budget))
        ));
    }

    #[test]
    fn failures_are_kept_so_they_are_not_retried() {
        let mut cache: Cache<u32> = Cache::new(100);
        let generation = cache.generation();
        cache.begin("a");
        cache.finish_err(generation, "a", Reason::Network);
        assert!(matches!(
            cache.get("a"),
            Some(State::Failed(Reason::Network))
        ));
        assert!(!cache.begin("a"));
    }

    #[test]
    fn reset_drops_everything_and_ignores_late_results() {
        let mut cache: Cache<u32> = Cache::new(100);
        let old = cache.generation();
        cache.begin("a");
        cache.reset();
        assert!(cache.get("a").is_none());
        assert_ne!(cache.generation(), old);
        cache.finish_ok(old, "a", 1, 10);
        assert!(cache.get("a").is_none());
        assert!(cache.begin("a"));
    }
}
