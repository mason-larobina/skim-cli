//! End-to-end tests over the public API: rendering, skeleton counting, and the
//! band/comment gate.

use std::path::{Path, PathBuf};

use skim::check::{self, check_doc, CheckOptions};
use skim::discover::{self, Config, PairRule, Relation};
use skim::lang::Lang;
use skim::metrics::{self, Metric};
use skim::render;

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
        cli_band: Some((0.0, 1000.0)),
        ..opts()
    };
    let config = Config::empty(dir.clone());
    let r = check_doc(&path, &config, &o).unwrap();
    assert_eq!(r.status, "comments", "{r:?}");
    assert!(r.comment.unwrap().ratio > 0.3);
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
