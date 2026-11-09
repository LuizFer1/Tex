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
    pub hidden: Vec<String>,
}

fn decayed(entry: &Use, now: i64) -> f64 {
    let elapsed = (now - entry.last).max(0) as f64;
    entry.score * 0.5f64.powf(elapsed / HALF_LIFE_SECS)
}

fn keep_recent(hidden: &mut Vec<String>) {
    let extra = hidden.len().saturating_sub(MAX_ENTRIES);
    hidden.drain(..extra);
}

impl Usage {
    /// Counts one opening of `path` at `now` (Unix seconds); past `MAX_ENTRIES` the weakest goes.
    /// A hidden path is not counted.
    pub fn record(&mut self, path: &str, now: i64) {
        if self.is_hidden(path) {
            return;
        }
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

    /// Drops `path` from the counts and keeps it out of the suggestions until the history is cleared.
    pub fn hide(&mut self, path: &str) {
        self.apps
            .retain(|entry| !entry.path.eq_ignore_ascii_case(path));
        if !self.is_hidden(path) {
            self.hidden.push(path.to_owned());
            keep_recent(&mut self.hidden);
        }
    }

    pub fn is_hidden(&self, path: &str) -> bool {
        self.hidden
            .iter()
            .any(|hidden| hidden.eq_ignore_ascii_case(path))
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
        self.hidden.clear();
    }

    pub fn from_toml(text: &str) -> Option<Self> {
        let table: Table = text.parse().ok()?;
        let mut hidden = Vec::new();
        if let Some(Value::Array(paths)) = table.get("hidden") {
            for path in paths {
                if let Value::String(path) = path
                    && !path.is_empty()
                    && !hidden
                        .iter()
                        .any(|kept: &String| kept.eq_ignore_ascii_case(path))
                {
                    hidden.push(path.clone());
                }
            }
        }
        keep_recent(&mut hidden);
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
        let mut usage = Self { apps, hidden };
        usage.apps.retain(|entry| {
            !usage
                .hidden
                .iter()
                .any(|hidden| hidden.eq_ignore_ascii_case(&entry.path))
        });
        Some(usage)
    }

    pub fn to_toml(&self) -> String {
        #[derive(Serialize)]
        struct File<'a> {
            #[serde(skip_serializing_if = "Vec::is_empty")]
            hidden: Vec<String>,
            app: &'a [Use],
        }
        toml::to_string(&File {
            hidden: self.hidden.clone(),
            app: &self.apps,
        })
        .unwrap_or_default()
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
            ..Default::default()
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

    #[test]
    fn hide_removes_the_entry_and_top_skips_it() {
        let mut usage = Usage::default();
        usage.record("a", 10);
        usage.record("b", 20);
        usage.hide("a");
        assert!(usage.is_hidden("a"));
        assert_eq!(usage.top(5, 30, |_| false), ["b"]);
    }

    #[test]
    fn recording_a_hidden_path_does_not_bring_it_back() {
        let mut usage = Usage::default();
        usage.record("a", 10);
        usage.hide("a");
        usage.record("a", 20);
        assert!(usage.apps.is_empty());
        assert!(usage.top(5, 30, |_| false).is_empty());
    }

    #[test]
    fn hide_ignores_case_and_does_not_duplicate() {
        let mut usage = Usage::default();
        usage.hide("C:\\App.lnk");
        usage.hide("c:\\app.LNK");
        assert_eq!(usage.hidden, ["C:\\App.lnk"]);
        assert!(usage.is_hidden("c:\\app.lnk"));
        usage.record("C:\\APP.lnk", 5);
        assert!(usage.apps.is_empty());
    }

    #[test]
    fn hidden_paths_keep_the_newest() {
        let mut usage = Usage::default();
        for number in 0..=MAX_ENTRIES {
            usage.hide(&format!("app{number}"));
        }
        assert_eq!(usage.hidden.len(), MAX_ENTRIES);
        assert!(!usage.is_hidden("app0"));
        assert!(usage.is_hidden(&format!("app{MAX_ENTRIES}")));
    }

    #[test]
    fn clear_empties_hidden() {
        let mut usage = Usage::default();
        usage.hide("a");
        usage.clear();
        assert!(usage.hidden.is_empty());
        usage.record("a", 1);
        assert_eq!(usage.apps.len(), 1);
    }

    #[test]
    fn hidden_survives_toml_round_trip_and_drops_hidden_apps() {
        let mut usage = Usage::default();
        usage.record("a", 5);
        usage.hide("b");
        let text = usage.to_toml();
        assert!(text.starts_with("hidden = ["));
        assert_eq!(Usage::from_toml(&text), Some(usage.clone()));
        let loaded = Usage::from_toml(concat!(
            "hidden = ['B']\n",
            "[[app]]\npath = 'a'\nscore = 1.0\nlast = 1\n\n",
            "[[app]]\npath = 'b'\nscore = 1.0\nlast = 1\n",
        ))
        .unwrap();
        assert_eq!(loaded.hidden, ["B"]);
        assert_eq!(loaded.apps.len(), 1);
        assert_eq!(loaded.apps[0].path, "a");
    }

    #[test]
    fn file_without_hidden_loads_and_saves_without_the_key() {
        let loaded = Usage::from_toml("[[app]]\npath = 'a'\nscore = 1.0\nlast = 1\n").unwrap();
        assert!(loaded.hidden.is_empty());
        assert_eq!(loaded.apps.len(), 1);
        assert!(!loaded.to_toml().contains("hidden"));
        assert_eq!(Usage::from_toml("hidden = 3\n"), Some(Usage::default()));
        let mixed = Usage::from_toml("hidden = [1, '', 'x', 'X']\n").unwrap();
        assert_eq!(mixed.hidden, ["x"]);
    }
}
