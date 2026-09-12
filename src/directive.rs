//! `Skim:` directives — the in-file overrides.
//!
//! Any line whose content is `Skim:` (optionally behind a comment marker such
//! as `>`, `//`, `#` or `*`) contributes to a [`Directive`]. The payload is a
//! small space/comma-separated grammar:
//!
//! ```text
//! [lo, hi]                       band override
//! against <glob>[, <glob>...]    tracked-source override
//! comments <ratio>               comment-ratio ceiling
//! metric <nonblank|code|decls>   which source count the band uses
//! <bare glob>                    shorthand for `against <glob>`
//! ```
//!
//! e.g. `> Skim: [4.0, 12.5] against src/**/*.java comments 0.5`.

use crate::discover::Relation;
use crate::metrics::Metric;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Directive {
    pub band: Option<(f64, f64)>,
    pub against: Vec<String>,
    pub comment_max: Option<f64>,
    pub comment_off: bool,
    pub metric: Option<Metric>,
    pub relation: Option<Relation>,
    /// Check the document's own skeleton against its body (the sqrt-proxy
    /// applied to a markdown file).
    pub self_check: bool,
}

impl Directive {
    pub fn is_empty(&self) -> bool {
        self.band.is_none()
            && self.against.is_empty()
            && self.comment_max.is_none()
            && !self.comment_off
            && self.metric.is_none()
            && self.relation.is_none()
            && !self.self_check
    }

    pub fn merge(&mut self, other: Directive) {
        if other.band.is_some() {
            self.band = other.band;
        }
        self.against.extend(other.against);
        if other.comment_max.is_some() {
            self.comment_max = other.comment_max;
        }
        self.comment_off |= other.comment_off;
        if other.metric.is_some() {
            self.metric = other.metric;
        }
        if other.relation.is_some() {
            self.relation = other.relation;
        }
        self.self_check |= other.self_check;
    }
}

const KEYWORDS: &[&str] = &[
    "against", "comments", "comment", "metric", "relation", "self",
];

/// Extract the raw payload of a `Skim:` line, if the line is one.
fn payload(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    for marker in [">", "//", "#", "/*", "*", "--", "<!--", ";"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            let rest = rest.trim_start();
            if let Some(p) = strip_skim(rest) {
                return Some(p);
            }
        }
    }
    strip_skim(trimmed)
}

fn strip_skim(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.find("skim:") {
        if lower[..idx]
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, '>' | '/' | '#' | '*' | '-' | '!' | ';'))
        {
            return Some(text[idx + "skim:".len()..].trim());
        }
    }
    None
}

fn parse_band(inside: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = inside
        .split([',', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return None;
    }
    let lo = parts[0].parse().ok()?;
    let hi = parts[1].parse().ok()?;
    Some((lo, hi))
}

fn parse_ratio(token: &str) -> Option<f64> {
    token.trim_start_matches(['<', '=']).trim().parse().ok()
}

fn parse_payload(text: &str, d: &mut Directive) {
    let mut rest = text.to_string();
    if let (Some(l), Some(r)) = (rest.find('['), rest.find(']')) {
        if l < r {
            if let Some(band) = parse_band(&rest[l + 1..r]) {
                d.band = Some(band);
            }
            rest.replace_range(l..=r, " ");
        }
    }
    let tokens: Vec<&str> = rest
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "against" => {
                i += 1;
                while i < tokens.len() && !KEYWORDS.contains(&tokens[i]) {
                    d.against.push(tokens[i].to_string());
                    i += 1;
                }
            }
            "comments" | "comment" => {
                i += 1;
                if i < tokens.len() {
                    match tokens[i] {
                        "off" | "none" | "disabled" => d.comment_off = true,
                        t => {
                            if let Some(r) = parse_ratio(t) {
                                d.comment_max = Some(r);
                            }
                        }
                    }
                    i += 1;
                }
            }
            "metric" => {
                i += 1;
                if i < tokens.len() {
                    if let Some(m) = Metric::from_name(tokens[i]) {
                        d.metric = Some(m);
                    }
                    i += 1;
                }
            }
            "relation" => {
                i += 1;
                if i < tokens.len() {
                    if let Some(r) = Relation::from_name(tokens[i]) {
                        d.relation = Some(r);
                    }
                    i += 1;
                }
            }
            "self" => {
                d.self_check = true;
                i += 1;
            }
            kw if KEYWORDS.contains(&kw) => i += 1,
            glob => {
                d.against.push(glob.to_string());
                i += 1;
            }
        }
    }
}

/// Scan a whole file's text for `Skim:` directives and merge them in order.
pub fn parse(text: &str) -> Directive {
    let mut d = Directive::default();
    for line in text.lines() {
        if let Some(payload) = payload(line) {
            parse_payload(payload, &mut d);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_band_only() {
        let d = parse("> Skim: [0.4, 12]");
        assert_eq!(d.band, Some((0.4, 12.0)));
        assert!(d.against.is_empty());
    }

    #[test]
    fn parses_bare_glob() {
        let d = parse("> Skim: src/abc.java");
        assert_eq!(d.against, vec!["src/abc.java"]);
        assert_eq!(d.band, None);
    }

    #[test]
    fn parses_combined() {
        let d = parse("> Skim: [4, 12.5] against src/**.java, lib/*.java comments 0.5");
        assert_eq!(d.band, Some((4.0, 12.5)));
        assert_eq!(d.against, vec!["src/**.java", "lib/*.java"]);
        assert_eq!(d.comment_max, Some(0.5));
    }

    #[test]
    fn parses_source_comment_marker() {
        let d = parse("// skim: metric code");
        assert_eq!(d.metric, Some(Metric::Code));
    }

    #[test]
    fn ignores_ordinary_prose_mentioning_skim() {
        // A mention mid-sentence is not a directive.
        let d = parse("The skin of it is thin, but skimming is fine.");
        assert!(d.is_empty());
    }

    #[test]
    fn merges_multiple_lines() {
        let d = parse("> Skim: [1.5, 4.0]\n> Skim: src/a.md");
        assert_eq!(d.band, Some((1.5, 4.0)));
        assert_eq!(d.against, vec!["src/a.md"]);
    }
}
