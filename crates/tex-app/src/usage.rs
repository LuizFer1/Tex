use serde::{Deserialize, Serialize};
use toml::{Table, Value};

pub const HALF_LIFE_SECS: f64 = 14.0 * 86_400.0;
pub const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Use {
    pub path: String,
    pub score: f64,
    pub last: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub apps: Vec<Use>,
}

fn decayed(entry: &Use, now: i64) -> f64 {
    let elapsed = (now - entry.last).max(0) as f64;
    entry.score * 0.5f64.powf(elapsed / HALF_LIFE_SECS)
}

impl Usage {
    /// Counts one opening of `path` at `now` (Unix seconds); past `MAX_ENTRIES` the weakest goes.
    pub fn record(&mut self, path: &str, now: i64) {
        match self
            .apps
            .iter_mut()
            .find(|entry| entry.path.eq_ignore_ascii_case(path))
        {
            Some(entry) => {
                entry.score = decayed(entry, now) + 1.0;
                entry.last = now;
            }
            None => self.apps.push(Use {
                path: path.to_owned(),
                score: 1.0,
                last: now,
            }),
        }
        if self.apps.len() > MAX_ENTRIES
            && let Some(weakest) = (0..self.apps.len()).min_by(|&a, &b| {
                decayed(&self.apps[a], now).total_cmp(&decayed(&self.apps[b], now))
            })
        {
            self.apps.remove(weakest);
        }
    }

    /// Up to `limit` paths, most used first (ties: most recent), leaving out those `skip` refuses.
    pub fn top(&self, limit: usize, now: i64, skip: impl Fn(&str) -> bool) -> Vec<&str> {
        let mut ranked: Vec<&Use> = self
            .apps
            .iter()
            .filter(|entry| !skip(&entry.path))
            .collect();
        ranked.sort_by(|a, b| {
            decayed(b, now)
                .total_cmp(&decayed(a, now))
                .then(b.last.cmp(&a.last))
        });
        ranked
            .into_iter()
            .take(limit)
            .map(|entry| entry.path.as_str())
            .collect()
    }

    pub fn clear(&mut self) {
        self.apps.clear();
    }

    pub fn from_toml(text: &str) -> Option<Self> {
        let table: Table = text.parse().ok()?;
        let apps = match table.get("app") {
            Some(Value::Array(entries)) => entries
                .iter()
                .filter_map(|entry| entry.clone().try_into::<Use>().ok())
                .filter(|entry| {
                    !entry.path.is_empty() && entry.score.is_finite() && entry.score > 0.0
                })
                .collect(),
            _ => Vec::new(),
        };
        Some(Self { apps })
    }

    pub fn to_toml(&self) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            app: &'a [Use],
        }
        toml::to_string(&File { app: &self.apps }).unwrap_or_default()
    }
}

/// `%LOCALAPPDATA%\Tex\usage.toml`: data of this machine, not a preference.
pub fn usage_path() -> std::path::PathBuf {
    crate::clipboard_client::history_folder().join("usage.toml")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn entry(path: &str, score: f64, last: i64) -> Use {
        Use {
            path: path.to_owned(),
            score,
            last,
        }
    }

    #[test]
    fn recording_again_adds_one_to_the_decayed_score() {
        let mut usage = Usage::default();
        usage.record("a", 0);
        usage.record("a", 14 * DAY);
        assert!((usage.apps[0].score - 1.5).abs() < 1e-9);
        assert_eq!(usage.apps[0].last, 14 * DAY);
    }

    #[test]
    fn top_orders_by_decayed_score_then_recency() {
        let now = 100 * DAY;
        let mut usage = Usage::default();
        for _ in 0..3 {
            usage.record("a", now - 60 * DAY);
        }
        usage.record("b", now);
        assert_eq!(usage.top(5, now, |_| false), ["b", "a"]);

        let tie = Usage {
            apps: vec![
                entry("older", 2.0, now - 14 * DAY),
                entry("newer", 1.0, now),
            ],
        };
        assert_eq!(tie.top(5, now, |_| false), ["newer", "older"]);
    }

    #[test]
    fn top_skips_and_limits() {
        let mut usage = Usage::default();
        usage.record("a", 10);
        usage.record("b", 20);
        usage.record("c", 30);
        assert_eq!(usage.top(5, 30, |path| path == "a"), ["c", "b"]);
        assert_eq!(usage.top(1, 30, |_| false), ["c"]);
    }

    #[test]
    fn keeps_at_most_max_entries_dropping_the_weakest() {
        let now = 1000 * DAY;
        let mut usage = Usage::default();
        usage.record("ancient", 0);
        for number in 0..MAX_ENTRIES {
            usage.record(&format!("app{number}"), now);
        }
        assert_eq!(usage.apps.len(), MAX_ENTRIES);
        assert!(usage.apps.iter().all(|entry| entry.path != "ancient"));
    }

    #[test]
    fn clear_and_toml_round_trip() {
        let mut usage = Usage::default();
        usage.record("a", 5);
        usage.record("b", 9);
        assert_eq!(Usage::from_toml(&usage.to_toml()), Some(usage.clone()));
        let text = concat!(
            "[[app]]\npath = 'x'\nscore = -1.0\nlast = 1\n\n",
            "[[app]]\npath = 'y'\nscore = nan\nlast = 1\n\n",
            "[[app]]\npath = 'z'\nscore = 2.0\nlast = 1\n",
        );
        let loaded = Usage::from_toml(text).unwrap();
        assert_eq!(loaded.apps.len(), 1);
        assert_eq!(loaded.apps[0].path, "z");
        assert_eq!(Usage::from_toml("x ["), None);
        usage.clear();
        assert!(usage.apps.is_empty());
    }
}
