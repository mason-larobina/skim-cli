//! End-to-end diagnostics for the built `skim` binary: the no-file and
//! miss-hint messages, and the `--grep -iE PATTERN` argv wedge.

#![cfg(feature = "lang-java")]

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("skim-it-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// Run the built binary and return `(exit code, stdout, stderr)`.
fn skim(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_skim"))
        .args(args)
        .output()
        .expect("run skim");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A tempdir with one Java file whose skeleton and body both carry `alpha` and
/// `beta.value`.
fn java_fixture(tag: &str) -> PathBuf {
    let dir = tempdir(tag);
    std::fs::write(
        dir.join("Alpha.java"),
        "class Alpha {\n    Object beta = beta.value;\n    void run() {\n        \
         System.out.println(\"beta.value\");\n    }\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn map_on_a_missing_path_is_an_error() {
    let dir = tempdir("map-missing");
    let missing = dir.join("missing-dir");
    let (code, _out, err) = skim(&["--map", &missing.to_string_lossy()]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("no files matched"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_on_a_missing_path_is_an_error() {
    let dir = tempdir("grep-missing");
    let missing = dir.join("missing-dir");
    let (code, _out, err) = skim(&["--grep", "zz", &missing.to_string_lossy()]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("no files matched"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn body_conflicts_with_referrers_by_name() {
    let dir = tempdir("body-referrers");
    let (code, _out, err) = skim(&["--referrers", "panel", "--body", &dir.to_string_lossy()]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("--body"), "stderr: {err}");
    assert!(err.contains("--referrers"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_ie_is_the_audit_wedge() {
    let dir = java_fixture("grep-ie");
    let (code, out, err) = skim(&["--grep", "-iE", "alpha|beta", &dir.to_string_lossy()]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("Alpha.java"), "stdout: {out}");
    assert!(
        !err.contains("a value is required for '--grep"),
        "stderr: {err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_if_literal_is_accepted() {
    let dir = java_fixture("grep-if");
    let (code, out, err) = skim(&["--grep", "-iF", "beta.value", &dir.to_string_lossy()]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("beta.value"), "stdout: {out}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_short_i_is_accepted() {
    let dir = java_fixture("grep-i");
    let (code, out, err) = skim(&["--grep", "-i", "alpha", &dir.to_string_lossy()]);
    assert_eq!(code, 0, "stdout: {out}\nstderr: {err}");
    assert!(out.contains("Alpha.java"), "stdout: {out}");
    assert!(out.contains("(scanned 1)"), "stdout: {out}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_body_wedge_resolves_rather_than_erroring() {
    let dir = java_fixture("grep-body-wedge");
    let (code, out, err) = skim(&["--grep", "--body", "beta.value", &dir.to_string_lossy()]);
    assert_ne!(code, 2, "stderr: {err}");
    assert!(out.contains("beta.value"), "stdout: {out}");
    assert!(
        !err.contains("a value is required for '--grep"),
        "stderr: {err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_canonical_order_still_reaches_bodies() {
    // Regression guard for the canonical order: `--grep PATTERN --body` must
    // stay a body search. `println` lives only in the elided method body, so
    // the skeleton misses it and the body search finds it with its scope
    // label - a pattern the skeleton also carries would not discriminate.
    let dir = java_fixture("grep-canonical");
    let (body_code, body_out, body_err) =
        skim(&["--grep", "println", "--body", &dir.to_string_lossy()]);
    assert_eq!(body_code, 0, "stdout: {body_out}\nstderr: {body_err}");
    assert!(body_out.contains("Alpha.run"), "stdout: {body_out}");
    let (skel_code, skel_out, _) = skim(&["--grep", "println", &dir.to_string_lossy()]);
    assert_eq!(skel_code, 1, "stdout: {skel_out}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grep_flag_with_no_token_after_it_still_exits_2() {
    // No token at all after the flag run: the normalizer leaves argv alone and
    // clap reports the missing value before any grep runs.
    let (code, out, err) = skim(&["--grep", "-i"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(!out.contains("match(es)"), "stdout: {out}");
}

#[test]
fn a_flag_before_the_pattern_takes_the_next_token_as_the_pattern() {
    // A non-flag token right after the flag run is the pattern (the paths
    // default to the cwd), exactly as `grep -i foo` reads `foo` as the pattern.
    let (code, _out, err) = skim(&["--grep", "-i", "zzz-no-such-skeleton-token-zzz"]);
    assert_eq!(code, 1, "stderr: {err}");
}

#[test]
fn body_miss_over_markdown_names_source_only_search() {
    let dir = java_fixture("body-markdown");
    std::fs::write(
        dir.join("NOTES.md"),
        "# alpha\n\n> prose-only-token lives here\n",
    )
    .unwrap();
    let (code, _out, err) = skim(&[
        "--grep",
        "prose-only-token",
        "--body",
        &dir.to_string_lossy(),
    ]);
    assert_eq!(code, 1, "body skips markdown: {err}");
    assert!(err.contains("source lines only"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn skeleton_miss_does_not_emit_the_body_markdown_hint() {
    let dir = java_fixture("skeleton-markdown");
    std::fs::write(
        dir.join("NOTES.md"),
        "# alpha\n\n> prose-only-token lives here\n",
    )
    .unwrap();
    let (code, _out, err) = skim(&["--grep", "zzz-no-such-token", &dir.to_string_lossy()]);
    assert_eq!(code, 1, "stderr: {err}");
    assert!(!err.contains("source lines only"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn check_on_a_non_doc_says_nothing_was_checked() {
    let dir = java_fixture("check-nothing");
    let (code, _out, err) = skim(&["--check", &dir.join("Alpha.java").to_string_lossy()]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(err.contains("no documents matched"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn help_documents_decls_as_per_declarator() {
    let (code, out, _err) = skim(&["--help"]);
    assert_eq!(code, 0, "stdout: {out}");
    assert!(out.contains("each declarator"), "stdout: {out}");
}

#[test]
fn grep_equals_pattern_passes_through_without_panicking() {
    let dir = java_fixture("grep-equals");
    let (code, _out, err) = skim(&["--grep=-x", &dir.to_string_lossy()]);
    assert!(code == 0 || code == 1, "code {code}, stderr: {err}");
    assert!(!err.contains("panic"), "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn render_docs_and_count_on_a_missing_path_are_errors() {
    // The default render and `--docs` were silent exit-0 blanks on a typo, the
    // same failure `--map` had; they share the guard now.
    let dir = tempdir("render-missing");
    let missing = dir.join("missing-dir");
    let missing = missing.to_string_lossy();
    for args in [
        vec![&*missing],
        vec!["--docs", &missing],
        vec!["--count", &missing],
    ] {
        let (code, out, err) = skim(&args);
        assert_eq!(code, 2, "{args:?} stdout: {out}\nstderr: {err}");
        assert!(err.contains("no files matched"), "{args:?} stderr: {err}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_directory_of_nothing_parseable_is_an_error_not_an_empty_map() {
    let dir = tempdir("unparseable");
    std::fs::write(dir.join("sprite.png"), b"\x89PNG").unwrap();
    for mode in ["--map", "--docs"] {
        let (code, out, err) = skim(&[mode, &dir.to_string_lossy()]);
        assert_eq!(code, 2, "{mode} stdout: {out}\nstderr: {err}");
        assert!(
            err.contains("no source files matched"),
            "{mode} stderr: {err}"
        );
    }
    let (code, _out, err) = skim(&["--grep", "x", &dir.to_string_lossy()]);
    assert_eq!(code, 2, "stderr: {err}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn referrers_summary_names_the_scanned_count() {
    let dir = java_fixture("referrers-scanned");
    std::fs::write(dir.join("sprite.png"), b"\x89PNG").unwrap();
    let (code, out, err) = skim(&["--referrers", "NoSuchName", &dir.to_string_lossy()]);
    assert_eq!(code, 1, "stderr: {err}");
    assert!(out.contains("in 0 file(s) (scanned 1;"), "stdout: {out}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn body_scanned_count_excludes_the_markdown_it_skips() {
    let dir = java_fixture("body-scanned");
    std::fs::write(dir.join("NOTES.md"), "# alpha\n").unwrap();
    let (_code, out, err) = skim(&["--grep", "zzz-none", "--body", &dir.to_string_lossy()]);
    assert!(out.contains("(scanned 1)"), "stdout: {out}");
    assert!(
        err.contains("1 markdown file(s) were skipped"),
        "stderr: {err}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
