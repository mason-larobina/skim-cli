//! `--map` reports its selection: a header counting what was printed and, when
//! the budget dropped definitions, a footer naming how many and how to see them.

#![cfg(feature = "lang-java")]

use std::path::PathBuf;

use skim::map::{self, MapOptions};

/// Two small Java files: `Beta` references `Alpha`.
fn tempdir(tag: &str) -> PathBuf {
    let base =
        std::env::temp_dir().join(format!("skim-map-elision-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("A.java"), "class Alpha { void a() {} }\n").unwrap();
    std::fs::write(base.join("B.java"), "class Beta { Alpha x; Alpha y; }\n").unwrap();
    base
}

#[test]
fn a_map_that_fits_the_budget_reports_no_omissions() {
    let dir = tempdir("fits");
    let map = map::run(std::slice::from_ref(&dir), &[], &MapOptions::default()).unwrap();
    assert_eq!(map.omitted, 0, "{}", map.text);
    assert_eq!(map.selected, map.entries.len());
    assert!(map.text.starts_with("# map: "), "{}", map.text);
    assert!(!map.text.contains("# omitted:"), "{}", map.text);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_map_over_budget_counts_and_labels_what_it_dropped() {
    let dir = tempdir("tight");
    let opts = MapOptions {
        budget_tokens: 1,
        ..MapOptions::default()
    };
    let map = map::run(std::slice::from_ref(&dir), &[], &opts).unwrap();
    assert!(map.omitted > 0, "{}", map.text);
    assert!(map.selected < map.selected + map.omitted, "{}", map.text);
    let footer = format!(
        "# omitted: {} - raise --map-budget to include them",
        map.omitted
    );
    assert_eq!(
        map.text.trim_end().lines().last(),
        Some(footer.as_str()),
        "{}",
        map.text
    );
    std::fs::remove_dir_all(dir).unwrap();
}
