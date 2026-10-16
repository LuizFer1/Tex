use std::collections::{HashMap, HashSet, VecDeque};
use std::iter;
use std::sync::mpsc::{self, Sender};
use std::thread;

use tex_platform::{IconPixels, app_icon, init_icon_thread};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconState<T> {
    Pending,
    Loaded(T),
    Missing,
}

/// Icons by path, including requests still in flight and paths without an icon.
/// Beyond `capacity` the oldest path is forgotten.
pub struct IconCache<T> {
    capacity: usize,
    entries: HashMap<String, IconState<T>>,
    order: VecDeque<String>,
}

impl<T> IconCache<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, path: &str) -> Option<&IconState<T>> {
        self.entries.get(path)
    }

    /// `true` the first time a path is seen, meaning the caller should ask the loader for it.
    pub fn mark_requested(&mut self, path: &str) -> bool {
        if self.entries.contains_key(path) {
            return false;
        }
        self.insert(path.to_owned(), IconState::Pending);
        true
    }

    pub fn store(&mut self, path: String, icon: Option<T>) {
        self.insert(path, icon.map_or(IconState::Missing, IconState::Loaded));
    }

    fn insert(&mut self, path: String, state: IconState<T>) {
        if self.entries.insert(path.clone(), state).is_some() {
            return;
        }
        self.order.push_back(path);
        if self.order.len() > self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
    }
}

/// Loads app icons on its own thread. `on_loaded` runs on that thread.
#[derive(Clone)]
pub struct IconLoader {
    requests: Sender<String>,
}

impl IconLoader {
    pub fn start(on_loaded: impl Fn(String, Option<IconPixels>) + Send + 'static) -> Self {
        let (requests, receiver) = mpsc::channel::<String>();
        thread::spawn(move || {
            init_icon_thread();
            while let Ok(first) = receiver.recv() {
                let mut seen = HashSet::new();
                for path in iter::once(first).chain(receiver.try_iter()) {
                    if seen.insert(path.clone()) {
                        let icon = app_icon(&path);
                        on_loaded(path, icon);
                    }
                }
            }
        });
        Self { requests }
    }

    pub fn request(&self, path: String) {
        let _ = self.requests.send(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_the_loader_once_per_path() {
        let mut cache: IconCache<u8> = IconCache::new(4);
        assert!(cache.mark_requested("a"));
        assert!(!cache.mark_requested("a"));
        assert_eq!(cache.get("a"), Some(&IconState::Pending));
    }

    #[test]
    fn stores_loaded_and_missing_icons() {
        let mut cache: IconCache<u8> = IconCache::new(4);
        cache.mark_requested("a");
        cache.store("a".into(), Some(7));
        cache.store("b".into(), None);
        assert_eq!(cache.get("a"), Some(&IconState::Loaded(7)));
        assert_eq!(cache.get("b"), Some(&IconState::Missing));
        assert!(!cache.mark_requested("b"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn drops_the_oldest_beyond_capacity() {
        let mut cache: IconCache<u8> = IconCache::new(2);
        cache.store("a".into(), Some(1));
        cache.store("b".into(), Some(2));
        cache.store("c".into(), Some(3));
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("a"), None);
        assert!(cache.mark_requested("a"));
        assert_eq!(cache.get("b"), None);
    }
}
