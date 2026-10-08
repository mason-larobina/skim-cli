//! End-to-end tests over the public API: rendering, skeleton counting, and the
//! band/comment gate.

#[cfg(any(
    feature = "lang-java",
    feature = "lang-python",
    feature = "lang-go",
    feature = "lang-kotlin",
    feature = "lang-rust"
))]
use std::path::Path;
use std::path::PathBuf;

use skim::check::{self, check_doc, CheckOptions};
use skim::discover::{self, Config, PairRule, Relation};
use skim::lang::Lang;
#[cfg(any(feature = "lang-java", feature = "lang-rust"))]
use skim::map::{self, MapOptions};
#[cfg(feature = "lang-java")]
use skim::metrics;
use skim::metrics::Metric;
use skim::render;
use skim::search::GrepOptions;

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("skim-it-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn opts() -> CheckOptions {
    CheckOptions {
        cli_against: vec![],
        cli_band: None,
        cli_comment_max: None,
        cli_comment_min: None,
        metric: None,
    }
}

#[cfg(feature = "lang-java")]
#[test]
fn render_elides_bodies_but_keeps_signatures() {
    let src = "class A {\n    int f() { return 42; }\n    int g();\n}\n";
    let out = render::render_source(src, Lang::from_path(Path::new("x.java"))).unwrap();
    assert!(out.contains("class A {"), "{out}");
    assert!(out.contains("int f() { … }"), "{out}");
    assert!(out.contains("int g();"), "{out}");
    assert!(!out.contains("return 42"), "body must be elided: {out}");
}

#[test]
fn markdown_skeleton_ignores_blank_and_lazy_lines() {
    let src = "# T\n\n> a\nlazy continuation\n\nbody text\n";
    let sk = render::markdown_skeleton(src, Lang::markdown()).unwrap();
    // heading + the one `>` line; the lazy line and blanks are body.
    assert_eq!(sk.len(), 2, "{sk:?}");
}

#[cfg(feature = "lang-java")]
#[test]
fn comment_lines_are_syntax_aware() {
    // The `//` inside the string is code; the line comment is comment.
    let src = "class A {\n  String s = \"http://x\"; // tail\n  // whole\n  /* block\n     more */\n  int f() { return 1; }\n}\n";
    let m = metrics::measure_source(src, Lang::from_path(Path::new("x.java"))).unwrap();
    assert_eq!(m.comment, 3, "{m:?}"); // whole, block start, block end
    assert_eq!(m.code, 4, "{m:?}"); // class, String init, return, closing brace
    assert_eq!(m.decls, 3, "{m:?}"); // class + field + method
}

#[cfg(feature = "lang-java")]
#[test]
fn sqrt_band_flags_under_documented_doc() {
    let dir = tempdir("under");
    let java: String = (0..200).map(|i| format!("int v{i};\n")).collect();
    std::fs::write(dir.join("Thing.java"), &java).unwrap();
    // 200 code lines -> floor 4*sqrt(200) ~= 56.6. 20 doc lines is under.
    let doc = dir.join("DESIGN.md");
    std::fs::write(&doc, "# Thing\n\nshort.\n").unwrap();
    let config = Config::empty(dir.clone());
    let r = check_doc(&doc, &config, &opts()).unwrap();
    assert_eq!(r.status, "under", "{r:?}");
    assert_eq!(r.band, [4.0, 12.5]);
    assert_eq!(r.relation, "sqrt");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn sqrt_band_passes_a_proportional_doc() {
    let dir = tempdir("ok");
    let java: String = (0..200).map(|i| format!("int v{i};\n")).collect();
    std::fs::write(dir.join("Thing.java"), &java).unwrap();
    // ratio must be in 4..12.5; aim ~7 => 7*sqrt(200) ~= 99 lines.
    let mut doc = String::from("# Thing\n\n");
    for i in 0..98 {
        doc.push_str(&format!("line {i}\n\n"));
    }
    let path = dir.join("DESIGN.md");
    std::fs::write(&path, doc).unwrap();
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &opts()).unwrap();
    assert_eq!(r.status, "ok", "{r:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn directive_overrides_band_and_relation() {
    let dir = tempdir("directive");
    let java: String = (0..100).map(|i| format!("int v{i};\n")).collect();
    std::fs::write(dir.join("Thing.java"), &java).unwrap();
    // Linear 0.5: 50 doc lines over 100 code lines.
    let mut doc = String::from("> Skim: [0.4, 0.6] relation linear\n\n# Thing\n\n");
    for i in 0..46 {
        doc.push_str(&format!("line {i}\n\n"));
    }
    let path = dir.join("DESIGN.md");
    std::fs::write(&path, doc).unwrap();
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &opts()).unwrap();
    assert_eq!(r.relation, "linear");
    assert_eq!(r.band, [0.4, 0.6]);
    assert_eq!(r.status, "ok", "{r:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn self_check_flags_over_indexed_skeleton() {
    let dir = tempdir("self");
    let mut doc = String::from("> Skim: self\n\n# Title\n\n");
    for i in 0..40 {
        doc.push_str(&format!("> q{i}\n\n"));
    }
    doc.push_str("the body\n");
    let path = dir.join("item.md");
    std::fs::write(&path, doc).unwrap();
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &opts()).unwrap();
    assert!(r.self_check, "{r:?}");
    assert_eq!(r.status, "over", "{r:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn comment_ceiling_fires() {
    let dir = tempdir("comments");
    let mut java = String::new();
    for i in 0..50 {
        java.push_str("// comment\n");
        if i % 5 == 0 {
            java.push_str("int x;\n");
        }
    }
    std::fs::write(dir.join("Thing.java"), &java).unwrap();
    let path = dir.join("DESIGN.md");
    std::fs::write(&path, "# Thing\n\nbody\n").unwrap();
    // Wide band isolates the comment failure.
    let o = CheckOptions {
        cli_comment_max: Some(0.3),
        cli_comment_min: Some(0),
        cli_band: Some((0.0, 1000.0)),
        ..opts()
    };
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &o).unwrap();
    assert_eq!(r.status, "comments", "{r:?}");
    assert!(r.comment.unwrap().ratio > 0.3);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn comment_ceiling_is_per_file_not_aggregate() {
    let dir = tempdir("comments-per-file");
    // One clean file plus one saturated file: the pair aggregate stays under
    // the ceiling while the saturated file alone exceeds it.
    let mut clean = String::new();
    for i in 0..300 {
        clean.push_str(&format!("int v{i};\n"));
    }
    std::fs::write(dir.join("Clean.java"), &clean).unwrap();
    let mut hot = String::new();
    for _ in 0..40 {
        hot.push_str("// c\n");
        hot.push_str("int x;\n");
    }
    for i in 0..40 {
        hot.push_str(&format!("// extra {i}\n"));
    }
    std::fs::write(dir.join("Hot.java"), &hot).unwrap();
    let path = dir.join("DESIGN.md");
    std::fs::write(&path, "# Thing\n\nbody\n").unwrap();
    let o = CheckOptions {
        cli_comment_max: Some(0.5),
        cli_comment_min: Some(50),
        cli_band: Some((0.0, 1000.0)),
        ..opts()
    };
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &o).unwrap();
    assert_eq!(r.status, "comments", "{r:?}");
    let cm = r.comment.unwrap();
    assert!(cm.ratio < 0.5, "aggregate must be under: {cm:?}");
    assert_eq!(cm.over.len(), 1, "{cm:?}");
    assert!(cm.over[0].file.ends_with("Hot.java"), "{cm:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn linear_ratio_is_plain_division() {
    assert!((Relation::Linear.ratio(50, 100) - 0.5).abs() < 1e-9);
    assert!((Relation::Sqrt.ratio(40, 100) - 4.0).abs() < 1e-9);
    assert_eq!(Metric::from_name("code"), Some(Metric::Code));
}

#[cfg(feature = "lang-python")]
#[test]
fn renders_python() {
    let src = "class Greeter:\n    def greet(self):\n        return 1\n";
    let out = render::render_source(src, Lang::from_path(Path::new("x.py"))).unwrap();
    assert!(out.contains("class Greeter:"), "{out}");
    assert!(out.contains("def greet(self):"), "{out}");
    assert!(!out.contains("return 1"), "{out}");
}

#[cfg(feature = "lang-go")]
#[test]
fn renders_go_types_with_their_keyword() {
    let src =
        "package m\ntype Point struct {\n X int\n}\ntype Shape interface {\n Area() float64\n}\n";
    let out = render::render_source(src, Lang::from_path(Path::new("x.go"))).unwrap();
    assert!(out.contains("type Point struct {"), "{out}");
    assert!(out.contains("type Shape interface {"), "{out}");
    assert!(out.contains("X int"), "{out}");
}

#[cfg(feature = "lang-kotlin")]
#[test]
fn renders_kotlin_without_hoisting_members() {
    let src = "class Widget(val n: Int) {\n    fun size(): Int { return n }\n}\n";
    let out = render::render_source(src, Lang::from_path(Path::new("x.kt"))).unwrap();
    assert!(out.contains("class Widget(val n: Int) {"), "{out}");
    assert!(out.contains("fun size(): Int"), "{out}");
    // The nested function body must stay under its own declaration.
    assert_eq!(out.matches('{').count(), out.matches('}').count(), "{out}");
}

#[test]
fn expand_handles_glob_and_directory() {
    let dir = tempdir("expand");
    std::fs::write(dir.join("A.java"), "class A {}\n").unwrap();
    std::fs::write(dir.join("B.md"), "# B\n").unwrap();
    let files = discover::expand(&[dir.join("*.java")]);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].ends_with("A.java"));
    let all = discover::expand(std::slice::from_ref(&dir));
    assert_eq!(all.len(), 2, "{all:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn aggregate_sums_docs_and_sources() {
    let dir = tempdir("agg");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let java: String = (0..100).map(|i| format!("int v{i};\n")).collect();
    std::fs::write(dir.join("src/A.java"), java).unwrap();
    for i in 0..2 {
        let mut doc = String::new();
        for j in 0..25 {
            doc.push_str(&format!("line {j}\n\n"));
        }
        std::fs::write(dir.join(format!("docs/D{i}.md")), doc).unwrap();
    }
    let rule_doc = globset::GlobBuilder::new("docs/*.md")
        .literal_separator(true)
        .build()
        .unwrap();
    let config = Config {
        root: dir.clone(),
        pairs: vec![PairRule {
            doc: rule_doc,
            doc_pattern: "docs/*.md".into(),
            source: vec!["src/**/*.java".into()],
            band: Some((0.4, 0.6)),
            comment_max: None,
            comment_min: None,
            metric: None,
            self_check: false,
            aggregate: true,
            relation: Relation::Linear,
        }],
    };
    let report = check::run(&[dir.join("docs")], &config, &opts()).unwrap();
    assert_eq!(report.checks.len(), 1, "{report:?}");
    assert_eq!(report.checks[0].status, "ok");
    assert_eq!(report.checks[0].doc_lines, 50);
    assert_eq!(report.checks[0].source_metric, 100);
    assert!((report.checks[0].ratio - 0.5).abs() < 1e-9);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn docs_keeps_leading_comment_and_skips_trailing() {
    let src = "class A {\n    /** The answer. */\n    int f() { return 42; }\n    int g() { return 1; } // trailing\n    int h() { return 2; }\n}\n";
    let lang = Lang::from_path(Path::new("x.java"));
    let plain = render::render_source(src, lang).unwrap();
    assert!(
        !plain.contains("The answer"),
        "docs off must drop comments: {plain}"
    );
    let out = render::render_source_with(src, lang, true).unwrap();
    assert!(out.contains("/** The answer. */"), "{out}");
    assert!(out.contains("int f()"), "{out}");
    assert!(
        !out.contains("trailing"),
        "a trailing comment must not attach to the next declaration: {out}"
    );
}

#[cfg(feature = "lang-rust")]
#[test]
fn docs_keeps_rust_line_doc_comment_but_not_across_a_blank() {
    let src = "/// Doc for f.\nfn f() {}\n\n// detached\n\nfn g() {}\n";
    let out = render::render_source_with(src, Lang::from_path(Path::new("x.rs")), true).unwrap();
    assert!(out.contains("/// Doc for f."), "{out}");
    assert!(
        !out.contains("detached"),
        "a blank line must break the doc run: {out}"
    );
}

#[cfg(feature = "lang-java")]
#[test]
fn map_ranks_referenced_types_above_isolated_ones() {
    let dir = tempdir("map");
    std::fs::write(dir.join("A.java"), "class Alpha { void a() {} }\n").unwrap();
    std::fs::write(dir.join("B.java"), "class Beta { Alpha x; Alpha y; }\n").unwrap();
    std::fs::write(dir.join("C.java"), "class Gamma { void c() {} }\n").unwrap();
    let map = map::run(std::slice::from_ref(&dir), &[], &MapOptions::default()).unwrap();
    let names: Vec<&str> = map.entries.iter().map(|e| e.name.as_str()).collect();
    let alpha = names.iter().position(|n| *n == "Alpha").expect("Alpha");
    let gamma = names.iter().position(|n| *n == "Gamma").expect("Gamma");
    assert!(
        alpha < gamma,
        "referenced type should outrank an isolated one: {names:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn map_nodes_are_types_not_members() {
    let dir = tempdir("mapnodes");
    std::fs::write(
        dir.join("A.java"),
        "class Alpha { int field; int helper() { return 1; } }\n",
    )
    .unwrap();
    let map = map::run(std::slice::from_ref(&dir), &[], &MapOptions::default()).unwrap();
    let names: Vec<&str> = map.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"Alpha"), "{names:?}");
    assert!(
        !names.contains(&"helper"),
        "methods are not map nodes: {names:?}"
    );
    assert!(
        !names.contains(&"field"),
        "fields are not map nodes: {names:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn map_balances_an_unselected_ancestor() {
    let dir = tempdir("mapbalance");
    std::fs::write(
        dir.join("Outer.java"),
        "class Outer { static class Inner {} }\n",
    )
    .unwrap();
    std::fs::write(dir.join("User.java"), "class User { Inner a; Inner b; }\n").unwrap();
    // A tight budget: Inner is referenced across files so it ranks above its
    // outer class, which is therefore printed only as context.
    let opts = MapOptions {
        budget_tokens: 20,
        ..MapOptions::default()
    };
    let map = map::run(std::slice::from_ref(&dir), &[], &opts).unwrap();
    assert!(map.text.contains("class Inner"), "{}", map.text);
    assert_eq!(
        map.text.matches('{').count(),
        map.text.matches('}').count(),
        "a context-only ancestor must be closed:\n{}",
        map.text
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn map_focus_biases_the_ranking() {
    let dir = tempdir("mapfocus");
    std::fs::create_dir_all(dir.join("one")).unwrap();
    std::fs::create_dir_all(dir.join("two")).unwrap();
    std::fs::write(dir.join("one/A.java"), "class Alpha {}\n").unwrap();
    std::fs::write(dir.join("two/B.java"), "class Beta {}\n").unwrap();
    let context = vec![dir.join("two")];
    let map = map::run(std::slice::from_ref(&dir), &context, &MapOptions::default()).unwrap();
    assert_eq!(
        map.entries.first().map(|e| e.name.as_str()),
        Some("Beta"),
        "{:?}",
        map.entries
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn lines_prefixes_each_declaration_with_its_source_row() {
    let src = "class A {\n    int f() { return 1; }\n}\n";
    let lang = Lang::from_path(Path::new("x.java"));
    let out = render::render_source_opts(src, lang, false, true).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].ends_with("class A {"), "{out}");
    assert!(lines[0].starts_with("    1  "), "{out}");
    assert!(lines[1].trim_start().starts_with("2      int f()"), "{out}");
    // The default renderer stays unnumbered.
    let plain = render::render_source(src, lang).unwrap();
    assert!(plain.starts_with("class A {"), "{plain}");
}

#[cfg(feature = "lang-java")]
#[test]
fn referrers_labels_each_hit_with_its_enclosing_definition() {
    let dir = tempdir("referrers");
    std::fs::write(
        dir.join("Thing.java"),
        "class Thing {\n    void run() {\n        helper();\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("User.java"),
        "class User {\n    void go() {\n        helper();\n        helper();\n    }\n}\n",
    )
    .unwrap();
    let report =
        map::referrers(std::slice::from_ref(&dir), "helper", &MapOptions::default()).unwrap();
    assert_eq!(report.occurrences, 3);
    assert_eq!(report.files, 2);
    assert!(
        report.entries.iter().any(|e| e.enclosing == "Thing.run"),
        "{:?}",
        report.entries
    );
    assert!(
        report.entries.iter().any(|e| e.enclosing == "User.go"),
        "{:?}",
        report.entries
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn referrers_text_finds_comments_and_strings() {
    let dir = tempdir("referrers-text");
    std::fs::write(
        dir.join("A.java"),
        "class A {\n    /** see Widget. */\n    String s = \"Widget\";\n    String t = \"Widgetry\";\n}\n",
    )
    .unwrap();
    // Default is code identifiers only, so a comment/string-only name is absent.
    let plain =
        map::referrers(std::slice::from_ref(&dir), "Widget", &MapOptions::default()).unwrap();
    assert_eq!(plain.occurrences, 0, "{:?}", plain.entries);
    assert!(!plain.text);
    // --text adds the comment and the string literal, but not the superstring.
    let opts = MapOptions {
        text: true,
        ..MapOptions::default()
    };
    let all = map::referrers(std::slice::from_ref(&dir), "Widget", &opts).unwrap();
    assert_eq!(all.occurrences, 2, "{:?}", all.entries);
    assert!(all.text);
    // --ignore-case folds ASCII across both kinds.
    let opts = MapOptions {
        text: true,
        ignore_case: true,
        ..MapOptions::default()
    };
    let folded = map::referrers(std::slice::from_ref(&dir), "widget", &opts).unwrap();
    assert_eq!(folded.occurrences, 2, "{:?}", folded.entries);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn check_origin_names_the_matched_contract() {
    let dir = tempdir("origin");
    let java: String = (0..100).map(|i| format!("int v{i};\n")).collect();
    std::fs::write(dir.join("Thing.java"), java).unwrap();
    let path = dir.join("DESIGN.md");
    std::fs::write(&path, "# Thing\n\nbody\n").unwrap();
    // No rule: the DESIGN.md sibling heuristic names itself.
    let sib = check_doc(&path, &Config::empty(dir.clone()), &opts()).unwrap();
    assert_eq!(sib.origin, "siblings");
    // A config rule with no source: skipped, and the origin names the glob.
    let rule_doc = globset::GlobBuilder::new("DESIGN.md")
        .literal_separator(true)
        .build()
        .unwrap();
    let config = Config {
        root: dir.clone(),
        pairs: vec![PairRule {
            doc: rule_doc,
            doc_pattern: "DESIGN.md".into(),
            source: vec![],
            band: None,
            comment_max: None,
            comment_min: None,
            metric: None,
            self_check: false,
            aggregate: false,
            relation: Relation::default(),
        }],
    };
    let r = check_doc(&path, &config, &opts()).unwrap();
    assert_eq!(r.status, "skipped", "{r:?}");
    assert_eq!(r.origin, "config: DESIGN.md");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn referrers_ignores_mentions_in_comments() {
    let dir = tempdir("referrers-comments");
    std::fs::write(
        dir.join("A.java"),
        "class A {\n    /** see {@link Widget}. */\n    void run() {}\n}\n",
    )
    .unwrap();
    let report =
        map::referrers(std::slice::from_ref(&dir), "Widget", &MapOptions::default()).unwrap();
    assert_eq!(report.occurrences, 0, "{:?}", report.entries);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_profile_extension_is_unique() {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut has_markdown = false;
    for p in skim::lang::PROFILES {
        has_markdown |= p.doc && p.name == "markdown";
        for ext in p.extensions {
            assert!(seen.insert(ext), "extension {ext} claimed twice");
        }
    }
    assert!(has_markdown, "markdown must always be compiled in");
    assert!(
        skim::lang::PROFILES.len() >= 2,
        "expected at least markdown + one language"
    );
}

fn grep_opts(ignore_case: bool, fixed_strings: bool) -> GrepOptions {
    GrepOptions {
        ignore_case,
        fixed_strings,
        ..GrepOptions::default()
    }
}

#[test]
fn grep_matches_the_skeleton_not_the_prose() {
    let dir = tempdir("grep-md");
    std::fs::write(
        dir.join("DESIGN.md"),
        "# Widget design\n\n> The widget holds `seats`.\n\nProse mentions seats but is not the index.\n\n## Layout\n\n> Nothing here.\n",
    )
    .unwrap();
    // The quote matches; the prose line does not.
    let hits = skim::search::grep(
        std::slice::from_ref(&dir),
        "seats",
        &grep_opts(false, false),
    )
    .unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].line, 3);
    assert!(hits[0].skeleton.contains("seats"));
    // Case folding matches both the heading and the quote.
    let folded = skim::search::grep(
        std::slice::from_ref(&dir),
        "WIDGET",
        &grep_opts(true, false),
    )
    .unwrap();
    let lines: Vec<usize> = folded.iter().map(|m| m.line).collect();
    assert_eq!(lines, vec![1, 3]);
    // A miss is empty.
    assert!(skim::search::grep(
        std::slice::from_ref(&dir),
        "absent",
        &grep_opts(false, false)
    )
    .unwrap()
    .is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_treats_the_pattern_as_a_regex_by_default() {
    let dir = tempdir("grep-regex");
    std::fs::write(
        dir.join("DESIGN.md"),
        "# Widget design\n\n> The widget holds `seats`.\n\n> A seat is a row.\n",
    )
    .unwrap();
    let regex = |pat: &str| {
        skim::search::grep(std::slice::from_ref(&dir), pat, &grep_opts(false, false)).unwrap()
    };
    // `.` is a wildcard: `s.ats` matches `seats`.
    let wild = regex("s.ats");
    assert_eq!(wild.len(), 1, "{wild:?}");
    assert_eq!(wild[0].line, 3);
    // Alternation and `^` anchoring select the heading only.
    let alt: Vec<usize> = regex("Widget|Layout").iter().map(|m| m.line).collect();
    assert_eq!(alt, vec![1]);
    let anchored = regex("^# Widget");
    assert_eq!(anchored.len(), 1, "{anchored:?}");
    assert_eq!(anchored[0].line, 1);
    // The same metacharacters are literal under --fixed-strings, so `s.ats`
    // no longer matches the quote.
    let literal =
        skim::search::grep(std::slice::from_ref(&dir), "s.ats", &grep_opts(false, true)).unwrap();
    assert!(literal.is_empty(), "{literal:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_rejects_an_invalid_regex_but_accepts_it_literally() {
    let dir = tempdir("grep-bad");
    std::fs::write(dir.join("DESIGN.md"), "# T\n\n> (a seat)\n").unwrap();
    let err =
        skim::search::grep(std::slice::from_ref(&dir), "(", &grep_opts(false, false)).unwrap_err();
    assert!(err.to_string().contains("invalid --grep pattern"), "{err}");
    // As a fixed string, `(` is just a character.
    let hits =
        skim::search::grep(std::slice::from_ref(&dir), "(", &grep_opts(false, true)).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_zero_match_hint_flags_a_bre_escape() {
    // A literal `\|` is a valid ERE, so the miss is silent by default; the
    // hint names the likely mistake.
    let hint = skim::search::zero_match_hint("lifeFrame\\|decal", false).unwrap();
    assert!(hint.contains("alternation"), "{hint}");
    // Other BRE escapes get the generic wording.
    let generic = skim::search::zero_match_hint("a\\(b", false).unwrap();
    assert!(generic.contains("metacharacter"), "{generic}");
    // A plain miss and a literal (fixed-string) miss carry no hint.
    assert!(skim::search::zero_match_hint("seat", false).is_none());
    assert!(skim::search::zero_match_hint("a\\|b", true).is_none());
}

#[test]
fn skeleton_miss_hint_points_at_body_mode_only_for_skeleton_searches() {
    // A skeleton miss nudges the reader at the body search the skeleton elides.
    let hint = skim::search::skeleton_miss_hint(false).unwrap();
    assert!(hint.contains("--body"), "{hint}");
    // Body mode is already the raw-line search: no nudge, and no second hint
    // next to the BRE one a skeleton miss may carry.
    assert!(skim::search::skeleton_miss_hint(true).is_none());
}

#[cfg(feature = "lang-rust")]
#[test]
fn referrers_marks_the_declaration_and_names_impls() {
    let dir = tempdir("referrers-def");
    std::fs::write(
        dir.join("lib.rs"),
        "struct Widget;\n\nimpl Widget {\n    fn draw(&self) { helper(); }\n    fn helper(&self) {}\n}\n\nfn helper() {}\n",
    )
    .unwrap();
    let report =
        map::referrers(std::slice::from_ref(&dir), "helper", &MapOptions::default()).unwrap();
    // The `impl` block is named after its type, so the method label is
    // `Widget.helper`, not the first method's name, and the declaration is
    // flagged rather than the call site.
    assert!(
        report
            .entries
            .iter()
            .any(|e| e.enclosing == "Widget.helper"),
        "{:?}",
        report.entries
    );
    assert!(
        report.entries.iter().any(|e| e.definition && e.line == 5),
        "{:?}",
        report.entries
    );
    assert!(
        report
            .entries
            .iter()
            .any(|e| !e.definition && e.enclosing == "Widget.draw"),
        "{:?}",
        report.entries
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(feature = "lang-java")]
#[test]
fn grep_body_labels_a_structural_pattern_the_skeleton_elides() {
    let dir = tempdir("grep-body");
    std::fs::write(
        dir.join("Thing.java"),
        "class Thing {\n    void run() {\n        Box2d b = new Box2d();\n        b.bounds = compute();\n    }\n}\n",
    )
    .unwrap();
    let body = |pat: &str| {
        skim::search::grep_body(
            std::slice::from_ref(&dir),
            pat,
            &GrepOptions {
                ..GrepOptions::default()
            },
        )
        .unwrap()
    };
    // The skeleton search cannot see a body statement ...
    let skel = skim::search::grep(
        std::slice::from_ref(&dir),
        "new Box2d",
        &grep_opts(false, false),
    )
    .unwrap();
    assert!(skel.is_empty(), "{skel:?}");
    // ... but the body search finds the call, labelled with its method.
    let ctor = body("new Box2d\\(");
    assert_eq!(ctor.len(), 1, "{ctor:?}");
    assert_eq!(ctor[0].line, 3);
    assert_eq!(ctor[0].enclosing, "Thing.run");
    // An assignment pattern, matched literally with --fixed-strings.
    let assign = skim::search::grep_body(
        std::slice::from_ref(&dir),
        "b.bounds =",
        &GrepOptions {
            fixed_strings: true,
            ..GrepOptions::default()
        },
    )
    .unwrap();
    assert_eq!(assign.len(), 1, "{assign:?}");
    assert_eq!(assign[0].line, 4);
    assert_eq!(assign[0].enclosing, "Thing.run");
    std::fs::remove_dir_all(dir).unwrap();
}
