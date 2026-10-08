//! Library tests for the two search-miss diagnoses that were silent before:
//! `--grep --body` over markdown, and `--referrers` with call/member syntax.

use skim::search::{body_doc_hint, referrers_shape_hint};

#[test]
fn body_doc_hint_is_none_when_no_markdown_was_scanned() {
    assert!(body_doc_hint(0).is_none());
}

#[test]
fn body_doc_hint_names_source_only_search_and_the_count() {
    let hint = body_doc_hint(3).expect("a hint when markdown was scanned");
    assert!(hint.contains("source lines only"), "{hint}");
    assert!(hint.contains('3'), "{hint}");
}

#[test]
fn referrers_shape_hint_is_none_for_a_bare_identifier() {
    assert!(referrers_shape_hint("panel").is_none());
    assert!(referrers_shape_hint("Frame").is_none());
    assert!(referrers_shape_hint("_pairs1").is_none());
}

#[test]
fn referrers_shape_hint_flags_a_call_and_a_member_form() {
    let call = referrers_shape_hint("panel()").expect("call syntax is not a bare identifier");
    assert!(call.contains("bare identifier"), "{call}");
    assert!(call.contains("--grep"), "{call}");
    let member = referrers_shape_hint("Frame.pairs").expect("member syntax is not bare");
    assert!(member.contains("bare identifier"), "{member}");
    assert!(member.contains("--grep"), "{member}");
}
