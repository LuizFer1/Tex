use std::time::{Duration, Instant};

use tex_index::{Attributes, Categories, Filter, Query, ROOT, SearchRequest, VolumeIndex, search};

const WORDS: [&str; 16] = [
    "report", "invoice", "photo", "backup", "notes", "draft", "final", "budget", "project",
    "readme", "config", "index", "main", "test", "data", "image",
];
const EXTENSIONS: [&str; 6] = ["txt", "pdf", "rs", "jpg", "json", "md"];
const ENTRIES: u32 = 3_000_000;
const DIRS: u32 = 30_000;
const FIRST: u32 = 16;

fn build() -> VolumeIndex {
    let mut volume = VolumeIndex::new("C:");
    let dir = Attributes {
        dir: true,
        ..Attributes::default()
    };
    for d in 0..DIRS {
        let parent = if d < 100 { ROOT } else { FIRST + d % 100 };
        volume.upsert(
            FIRST + d,
            parent,
            &format!("{}_{d}", WORDS[d as usize % 16]),
            dir,
        );
    }
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    for f in 0..ENTRIES - DIRS {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let a = WORDS[(seed % 16) as usize];
        let b = WORDS[((seed >> 8) % 16) as usize];
        let extension = EXTENSIONS[((seed >> 16) % 6) as usize];
        let parent = FIRST + ((seed >> 24) % u64::from(DIRS)) as u32;
        volume.upsert(
            FIRST + DIRS + f,
            parent,
            &format!("{a}_{b}_{f}.{extension}"),
            Attributes::default(),
        );
    }
    volume
}

fn time(volume: &VolumeIndex, text: &str) -> (u32, Duration) {
    let query = Query::parse(text).unwrap();
    let request = SearchRequest {
        query: &query,
        filter: Filter::All,
        limit: 50,
        home: None,
        include_hidden: false,
        include_system: false,
        excluded_folders: &[],
        categories: Categories::ALL,
        app_roots: &[],
        apps: None,
        language: "",
        viewer: None,
    };
    search(&[volume], &request, &|| false).unwrap();
    let started = Instant::now();
    let outcome = search(&[volume], &request, &|| false).unwrap();
    (outcome.total, started.elapsed())
}

#[test]
#[ignore = "slow; run with: cargo test --release -p tex-index --test performance -- --ignored --nocapture"]
fn searches_three_million_entries_under_50ms() {
    let volume = build();
    assert_eq!(volume.len(), ENTRIES as usize);
    println!("name buffer: {} MB", volume.name_bytes() / 1_000_000);

    let (total, elapsed) = time(&volume, "budget final");
    println!("'budget final': {total} matches in {elapsed:?}");
    assert!(total > 0);
    assert!(
        elapsed < Duration::from_millis(50),
        "search took {elapsed:?}"
    );

    for text in ["draft_5/.pdf", "draft_5/"] {
        let (total, elapsed) = time(&volume, text);
        println!("'{text}': {total} matches in {elapsed:?}");
        assert!(total > 0);
        assert!(
            elapsed < Duration::from_millis(50),
            "'{text}' took {elapsed:?}"
        );
    }

    let (total, elapsed) = time(&volume, "e");
    println!("'e' (worst case): {total} matches in {elapsed:?}");
}
