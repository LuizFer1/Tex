use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tex_platform::{StoreApp, store_apps};

const STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// Whether the list asked for at `asked_at` should be read again at `now`.
fn is_stale(asked_at: Option<Instant>, now: Instant) -> bool {
    asked_at.is_none_or(|asked| now.duration_since(asked) > STALE_AFTER)
}

#[derive(Default)]
struct State {
    apps: Arc<Vec<StoreApp>>,
    asked_at: Option<Instant>,
}

/// Keeps the list of Store apps, read by a worker thread so the shell never blocks the UI.
/// Nothing runs on a timer: the popup asks for a new read when it opens and the list is old.
#[derive(Clone)]
pub struct StoreClient {
    state: Arc<Mutex<State>>,
    requests: Sender<()>,
}

impl StoreClient {
    /// Starts the worker and the first read.
    pub fn start() -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let (requests, receiver) = mpsc::channel::<()>();
        thread::spawn({
            let state = state.clone();
            move || {
                while receiver.recv().is_ok() {
                    let apps = Arc::new(store_apps());
                    if let Ok(mut state) = state.lock() {
                        state.apps = apps;
                    }
                }
            }
        });
        let client = Self { state, requests };
        client.refresh_if_stale();
        client
    }

    /// Asks for a new read when the last one is more than ten minutes old.
    pub fn refresh_if_stale(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let now = Instant::now();
        if is_stale(state.asked_at, now) {
            state.asked_at = Some(now);
            let _ = self.requests.send(());
        }
    }

    pub fn apps(&self) -> Arc<Vec<StoreApp>> {
        self.state
            .lock()
            .map(|state| state.apps.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_asked_is_stale() {
        assert!(is_stale(None, Instant::now()));
    }

    #[test]
    fn fresh_until_ten_minutes_pass() {
        let start = Instant::now();
        assert!(!is_stale(Some(start), start + Duration::from_secs(599)));
        assert!(is_stale(Some(start), start + Duration::from_secs(601)));
    }
}
