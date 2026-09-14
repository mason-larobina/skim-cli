//! `--grep`: regular-expression search over the rendered skeletons. Unlike
//! `--referrers` (identifiers labelled with their enclosing scope) this matches
//! the *index* — markdown headings and quote lines, code declaration signatures
//! (and doc comments with `docs`) — so a hit names a concept or a definition
//! rather than an arbitrary line of prose. It is the scouting pass for finding
//! what a task touches before reading anything: which design docs carry a
//! concept, which module declares a tool, where a rule is written down.
//!
//! `PATTERN` is a regular expression by default, like `grep -E`; use
//! `fixed_strings` (`--fixed-strings`) to match the pattern literally.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::discover;
use crate::lang::Lang;
use crate::render;

/// How `--grep` should select and render the skeletons it searches.
#[derive(Clone, Copy, Debug, Default)]
pub struct GrepOptions {
    /// Force a grammar instead of inferring it from the path.
    pub force: Option<Lang>,
    /// Keep each declaration's leading doc comment in the skeleton.
    pub docs: bool,
    /// Fold case when matching (`--ignore-case`).
    pub ignore_case: bool,
    /// Treat `pattern` as a literal string (`--fixed-strings`) instead of a
    /// regular expression, so `.` and `*` match themselves.
    pub fixed_strings: bool,
}

/// One matching skeleton line.
#[derive(Serialize, Debug)]
pub struct Match {
    pub file: String,
    /// The 1-based source line the skeleton line maps to.
    pub line: usize,
    /// The skeleton line itself (heading, quote, or declaration signature).
    pub skeleton: String,
}

/// A compiled pattern, either a regex or a literal needle.
enum Matcher {
    Literal { needle: String, ignore_case: bool },
    Regex(regex::Regex),
}

impl Matcher {
    fn new(pattern: &str, ignore_case: bool, fixed_strings: bool) -> Result<Self> {
        if fixed_strings {
            // Keep the original literal fast path: no metacharacter parsing,
            // no compiled-program size limit, and byte-for-byte the behaviour
            // `--grep` had before regex support.
            let needle = if ignore_case {
                pattern.to_lowercase()
            } else {
                pattern.to_string()
            };
            return Ok(Matcher::Literal {
                needle,
                ignore_case,
            });
        }
        let re = regex::RegexBuilder::new(pattern)
            .case_insensitive(ignore_case)
            .build()
            .with_context(|| format!("invalid --grep pattern {pattern:?}"))?;
        Ok(Matcher::Regex(re))
    }

    fn is_match(&self, line: &str) -> bool {
        match self {
            Matcher::Literal {
                needle,
                ignore_case,
            } => {
                if *ignore_case {
                    line.to_lowercase().contains(needle)
                } else {
                    line.contains(needle)
                }
            }
            Matcher::Regex(re) => re.is_match(line),
        }
    }
}

/// Every skeleton line under `paths` matching `pattern`.
pub fn grep(paths: &[PathBuf], pattern: &str, opts: &GrepOptions) -> Result<Vec<Match>> {
    let files: Vec<PathBuf> = discover::expand(paths)
        .into_iter()
        .filter(|p| opts.force.is_some() || Lang::from_path(p).is_parseable())
        .collect();
    // Compile once, not per line; an invalid pattern fails before any I/O.
    let matcher = Matcher::new(pattern, opts.ignore_case, opts.fixed_strings)?;
    let mut out = Vec::new();
    for p in &files {
        let text =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let lang = opts.force.unwrap_or_else(|| Lang::from_path(p));
        // Always number the skeleton so a hit opens straight in `read`; split
        // the prefix off so matching sees the content and JSON still carries
        // the number.
        let rendered = if lang.is_doc() {
            render::render_markdown_opts(&text, lang, true)?
        } else {
            render::render_source_opts(&text, lang, opts.docs, true)?
        };
        for raw in rendered.lines() {
            // Match the skeleton content, not the `   13  ` line-number prefix
            // that `--lines` writes, so `^`/`$` anchor to what the user sees.
            let (line, skeleton) = split_line(raw);
            if matcher.is_match(&skeleton) {
                out.push(Match {
                    file: p.display().to_string(),
                    line,
                    skeleton,
                });
            }
        }
    }
    Ok(out)
}

/// Split the `   13  text` prefix `--lines` writes back into `(13, "text")`.
/// A line with no numeric prefix (a doc-comment continuation) reports line 0.
fn split_line(raw: &str) -> (usize, String) {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.split_once("  ") {
        let (num, text) = rest;
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return (num.parse().unwrap_or(0), text.to_string());
        }
    }
    (0, raw.trim_end().to_string())
}
