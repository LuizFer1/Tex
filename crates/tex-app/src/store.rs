use tex_index::{MatchKind, Query, fold};
use tex_ipc::{Filter, Hit};
use tex_platform::{STORE_PREFIX, StoreApp};

use crate::results::strip_app_extension;

/// Whether `path` is the parsing name of a Store app instead of a file path.
pub fn is_store_path(path: &str) -> bool {
    path.starts_with(STORE_PREFIX)
}

/// The shell's display name of the Store app behind `path`.
pub fn display_name<'a>(apps: &'a [StoreApp], path: &str) -> Option<&'a str> {
    apps.iter()
        .find(|app| app.path == path)
        .map(|app| app.name.as_str())
}

/// Adds the Store apps that match `query` to the service's hits and orders the apps by match
/// quality (the service's first on ties). Store apps are left out when the filter is Files or
/// Folders, when apps are hidden, for queries with a folder or drive, and when a shortcut with
/// the same name is already there. Keeps at most `limit` hits, apps first.
pub fn merge(
    hits: Vec<Hit>,
    apps: &[StoreApp],
    query: &str,
    filter: Filter,
    show_apps: bool,
    limit: usize,
) -> Vec<Hit> {
    let wanted = show_apps && matches!(filter, Filter::All | Filter::Apps);
    let parsed = Query::parse(query).filter(|_| !query.contains(['/', '\\']));
    let Some(parsed) = parsed.filter(|_| wanted && !apps.is_empty()) else {
        return hits;
    };
    let mut scratch = String::new();
    let (service_apps, others): (Vec<Hit>, Vec<Hit>) = hits.into_iter().partition(|hit| hit.is_app);
    let known: Vec<String> = service_apps
        .iter()
        .map(|hit| fold(&service_name(&hit.path)))
        .collect();
    let mut ranked: Vec<(Option<MatchKind>, Hit)> = service_apps
        .into_iter()
        .map(|hit| {
            let name = service_name(&hit.path);
            (parsed.matches(name.as_bytes(), &mut scratch), hit)
        })
        .collect();
    for app in apps {
        if known.contains(&fold(&app.name)) {
            continue;
        }
        if let Some(kind) = parsed.matches(app.name.as_bytes(), &mut scratch) {
            ranked.push((
                Some(kind),
                Hit {
                    path: app.path.clone(),
                    is_dir: false,
                    is_app: true,
                },
            ));
        }
    }
    ranked.sort_by_key(|(kind, _)| std::cmp::Reverse(*kind));
    let mut merged: Vec<Hit> = ranked.into_iter().map(|(_, hit)| hit).collect();
    merged.extend(others);
    merged.truncate(limit);
    merged
}

fn service_name(path: &str) -> String {
    let name = path.rsplit_once('\\').map_or(path, |(_, name)| name);
    strip_app_extension(name).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> StoreApp {
        StoreApp {
            name: name.into(),
            path: format!("{STORE_PREFIX}{name}!App"),
        }
    }

    fn app_hit(path: &str) -> Hit {
        Hit {
            path: path.into(),
            is_dir: false,
            is_app: true,
        }
    }

    fn file_hit(path: &str) -> Hit {
        Hit {
            path: path.into(),
            is_dir: false,
            is_app: false,
        }
    }

    fn paths(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.path.as_str()).collect()
    }

    #[test]
    fn matching_store_apps_join_the_apps() {
        let apps = [store("Windows Terminal"), store("Calculadora")];
        let merged = merge(
            vec![file_hit(r"C:\terminal.txt")],
            &apps,
            "terminal",
            Filter::All,
            true,
            50,
        );
        assert_eq!(
            paths(&merged),
            [r"shell:AppsFolder\Windows Terminal!App", r"C:\terminal.txt"]
        );
        assert!(merged[0].is_app && !merged[0].is_dir);
    }

    #[test]
    fn better_matches_come_first_and_the_service_wins_ties() {
        let apps = [store("Meu Terminal"), store("Terminal")];
        let hits = vec![app_hit(r"C:\ProgramData\Menu\Terminal.lnk")];
        let merged = merge(hits, &apps, "terminal", Filter::All, true, 50);
        assert_eq!(
            paths(&merged),
            [
                r"C:\ProgramData\Menu\Terminal.lnk",
                r"shell:AppsFolder\Meu Terminal!App",
            ]
        );
    }

    #[test]
    fn a_store_app_with_the_name_of_a_shortcut_is_dropped() {
        let apps = [store("Calculadora")];
        let hits = vec![app_hit(r"C:\ProgramData\Menu\calculadora.lnk")];
        let merged = merge(hits, &apps, "calc", Filter::All, true, 50);
        assert_eq!(paths(&merged), [r"C:\ProgramData\Menu\calculadora.lnk"]);
    }

    #[test]
    fn nothing_is_added_for_other_filters_or_hidden_apps() {
        let apps = [store("Terminal")];
        for (filter, show_apps) in [
            (Filter::Files, true),
            (Filter::Folders, true),
            (Filter::All, false),
            (Filter::Apps, false),
        ] {
            assert!(merge(Vec::new(), &apps, "terminal", filter, show_apps, 50).is_empty());
        }
        assert_eq!(
            merge(Vec::new(), &apps, "terminal", Filter::Apps, true, 50).len(),
            1
        );
    }

    #[test]
    fn path_queries_and_blank_queries_add_nothing() {
        let apps = [store("Terminal")];
        assert!(merge(Vec::new(), &apps, r"users\terminal", Filter::All, true, 50).is_empty());
        assert!(merge(Vec::new(), &apps, "  ", Filter::All, true, 50).is_empty());
    }

    #[test]
    fn the_limit_keeps_the_best_hits() {
        let apps = [store("Terminal")];
        let hits = vec![file_hit(r"C:\a"), file_hit(r"C:\b")];
        let merged = merge(hits, &apps, "terminal", Filter::All, true, 2);
        assert_eq!(paths(&merged), [r"shell:AppsFolder\Terminal!App", r"C:\a"]);
    }

    #[test]
    fn display_names_come_from_the_store_list() {
        let apps = [store("Windows Terminal")];
        assert_eq!(
            display_name(&apps, r"shell:AppsFolder\Windows Terminal!App"),
            Some("Windows Terminal")
        );
        assert_eq!(display_name(&apps, r"C:\x.exe"), None);
        assert!(is_store_path(r"shell:AppsFolder\x!App"));
        assert!(!is_store_path(r"C:\shell:AppsFolder\x"));
    }

    #[test]
    fn the_app_leads_even_when_a_file_matches_better() {
        let apps = [store("Calculadora")];
        let hits = vec![
            app_hit(r"C:\ProgramData\Menu\Microsoft Edge.lnk"),
            file_hit(r"C:\Steam\bin\edgewallpaper64.exe"),
        ];
        let merged = merge(hits, &apps, "edge", Filter::All, true, 50);
        let rows = crate::results::build_rows(&merged, &apps);
        let names: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                crate::results::Row::Item { name, .. } => Some(name.as_str()),
                crate::results::Row::Header(_) => None,
            })
            .collect();
        assert_eq!(names, ["Microsoft Edge", "edgewallpaper64.exe"]);
    }
}
