//! A FAIL row's hint: the exact ratio its two decimals can round onto the band
//! edge, and the doc lines that bring it back inside.

use skim::check::{band_hint, CheckReport};

fn row(
    status: &str,
    relation: &str,
    doc_lines: usize,
    source: usize,
    band: [f64; 2],
) -> CheckReport {
    let ratio = match relation {
        "linear" => doc_lines as f64 / source as f64,
        _ => doc_lines as f64 / (source as f64).sqrt(),
    };
    CheckReport {
        doc: "DESIGN.md".to_string(),
        sources: Vec::new(),
        doc_lines,
        source_metric: source,
        metric: "nonblank".to_string(),
        relation: relation.to_string(),
        ratio,
        band,
        status: status.to_string(),
        self_check: false,
        origin: String::new(),
        comment: None,
    }
}

#[test]
fn an_under_row_that_rounds_onto_the_floor_names_the_exact_ratio() {
    // 9625 / 32226 = 0.29867, which the row prints as 0.30 beside a 0.30 floor.
    let hint = band_hint(&row("under", "linear", 9625, 32226, [0.30, 0.80])).unwrap();
    assert_eq!(
        hint,
        "under: 0.2987 < 0.30 - add 43 doc line(s) to reach the floor"
    );
}

#[test]
fn a_sqrt_under_row_counts_against_the_root_of_the_source() {
    // 4.0 * sqrt(1759) = 167.8, so 54 doc lines need 114 more.
    let hint = band_hint(&row("under", "sqrt", 54, 1759, [4.0, 12.5])).unwrap();
    assert_eq!(
        hint,
        "under: 1.2875 < 4.00 - add 114 doc line(s) to reach the floor"
    );
}

#[test]
fn an_over_row_names_the_lines_to_cut() {
    // 12.5 * sqrt(100) = 125, so 130 doc lines are 5 over.
    let hint = band_hint(&row("over", "sqrt", 130, 100, [4.0, 12.5])).unwrap();
    assert_eq!(
        hint,
        "over: 13.0000 > 12.50 - cut 5 doc line(s) to reach the ceiling"
    );
}

#[test]
fn a_row_inside_its_band_has_no_hint() {
    assert!(band_hint(&row("ok", "sqrt", 80, 100, [4.0, 12.5])).is_none());
}
