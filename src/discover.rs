//! Pair discovery: which source files a document tracks, from its `Skim:`
//! directives, a `.skim.toml` rule, or the `DESIGN.md`/`README.md` sibling
//! heuristic.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobSetBuilder};
use ignore::WalkBuilder;
use serde::Deserialize;

use crate::directive::Directive;
use crate::lang::Lang;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Relation {
    /// `doc / sqrt(source)` — a scouting index of code.
    #[default]
    Sqrt,
    /// `doc / source` — a reference manual that grows with the code.
    Linear,
}

impl Relation {
    pub fn from_name(name: &str) -> Option<Relation> {
        match name.to_ascii_lowercase().as_str() {
            "sqrt" => Some(Relation::Sqrt),
            "linear" | "ratio" => Some(Relation::Linear),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Relation::Sqrt => "sqrt",
            Relation::Linear => "linear",
        }
    }

    pub fn ratio(self, doc_lines: usize, source_metric: usize) -> f64 {
        match self {
            Relation::Sqrt => doc_lines as f64 / (source_metric as f64).sqrt(),
            Relation::Linear => doc_lines as f64 / source_metric as f64,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    /// Repository root the `doc` globs and leading-`/` source globs resolve
    /// against. Defaults to the config file's directory.
    root: Option<String>,
    #[serde(default)]
    pair: Vec<RawPair>,
}

#[derive(Debug, Deserialize)]
struct RawPair {
    doc: String,
    #[serde(default)]
    source: Vec<String>,
    band: Option<[f64; 2]>,
    comment_max: Option<f64>,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    self_check: bool,
    /// Sum every matching doc into one ratio instead of checking each alone.
    #[serde(default)]
    aggregate: bool,
    #[serde(default)]
    relation: Option<String>,
}

#[derive(Debug)]
pub struct PairRule {
    pub doc: Glob,
    pub doc_pattern: String,
    pub source: Vec<String>,
    pub band: Option<(f64, f64)>,
    pub comment_max: Option<f64>,
    pub metric: Option<crate::metrics::Metric>,
    pub self_check: bool,
    pub aggregate: bool,
    pub relation: Relation,
}

#[derive(Debug, Default)]
pub struct Config {
    pub root: PathBuf,
    pub pairs: Vec<PairRule>,
}

impl Config {
    pub fn empty(root: PathBuf) -> Config {
        Config {
            root,
            pairs: Vec::new(),
        }
    }

    /// Find `.skim.toml` from `explicit`, else by walking up from `start`.
    pub fn load(explicit: Option<&Path>, start: &Path) -> Result<Config> {
        let found = match explicit {
            Some(p) => Some(p.to_path_buf()),
            None => find_upwards(start, ".skim.toml"),
        };
        let Some(path) = found else {
            return Ok(Config::empty(start.to_path_buf()));
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let raw: RawConfig =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        let root = match raw.root {
            Some(r) => {
                let p = PathBuf::from(r);
                if p.is_absolute() {
                    p
                } else {
                    path.parent().unwrap_or(Path::new(".")).join(p)
                }
            }
            None => path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        let mut pairs = Vec::new();
        for p in raw.pair {
            let doc = GlobBuilder::new(&p.doc)
                .literal_separator(true)
                .build()
                .with_context(|| format!("bad doc glob {:?}", p.doc))?;
            pairs.push(PairRule {
                doc,
                doc_pattern: p.doc.clone(),
                source: p.source,
                band: p.band.map(|b| (b[0], b[1])),
                comment_max: p.comment_max,
                metric: p
                    .metric
                    .as_deref()
                    .and_then(crate::metrics::Metric::from_name),
                self_check: p.self_check,
                aggregate: p.aggregate,
                relation: p
                    .relation
                    .as_deref()
                    .and_then(Relation::from_name)
                    .unwrap_or_default(),
            });
        }
        Ok(Config { root, pairs })
    }

    fn match_rule(&self, doc: &Path) -> Option<&PairRule> {
        let rel = doc.strip_prefix(&self.root).unwrap_or(doc);
        let rel = rel.to_string_lossy().replace('\\', "/");
        self.pairs
            .iter()
            .find(|r| r.doc.compile_matcher().is_match(&rel))
    }

    pub fn rule_matches(&self, rule: &PairRule, doc: &Path) -> bool {
        let rel = doc.strip_prefix(&self.root).unwrap_or(doc);
        let rel = rel.to_string_lossy().replace('\\', "/");
        rule.doc.compile_matcher().is_match(&rel)
    }
}

fn find_upwards(start: &Path, name: &str) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    loop {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Expand glob patterns into files. Relative patterns resolve against `base`;
/// a leading `/` resolves against `root`. Results are sorted and deduped.
pub fn glob_files(base: &Path, root: &Path, patterns: &[String]) -> Vec<PathBuf> {
    let mut set = GlobSetBuilder::new();
    let mut bases: Vec<PathBuf> = Vec::new();
    for pat in patterns {
        let (b, pat) = if let Some(stripped) = pat.strip_prefix('/') {
            (root.to_path_buf(), stripped.to_string())
        } else {
            (base.to_path_buf(), pat.clone())
        };
        if let Ok(g) = GlobBuilder::new(&pat).literal_separator(true).build() {
            set.add(g);
            if !bases.contains(&b) {
                bases.push(b);
            }
        }
    }
    let Ok(set) = set.build() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for b in bases {
        if !b.exists() {
            continue;
        }
        let walker = WalkBuilder::new(&b)
            .standard_filters(true)
            .hidden(false)
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(&b).unwrap_or(path);
            if set.is_match(rel) {
                out.push(path.to_path_buf());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Expand a single glob argument. An absolute pattern walks from the deepest
/// non-glob directory prefix; a relative pattern walks from `cwd`.
fn glob_arg(pattern: &str, cwd: &Path) -> Vec<PathBuf> {
    let (base, rel) = if Path::new(pattern).is_absolute() {
        let first = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
        let slash = pattern[..first].rfind('/').unwrap_or(0);
        let base = PathBuf::from(&pattern[..slash.max(1)]);
        let rel = pattern[slash + 1..].to_string();
        (base, rel)
    } else {
        (cwd.to_path_buf(), pattern.to_string())
    };
    if !base.exists() {
        return Vec::new();
    }
    let Ok(glob) = GlobBuilder::new(&rel).literal_separator(true).build() else {
        return Vec::new();
    };
    let matcher = glob.compile_matcher();
    let mut out = Vec::new();
    for entry in WalkBuilder::new(&base)
        .standard_filters(true)
        .hidden(false)
        .build()
        .flatten()
    {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel_path = path.strip_prefix(&base).unwrap_or(path);
        if matcher.is_match(rel_path) {
            out.push(path.to_path_buf());
        }
    }
    out
}

/// Expand CLI path arguments: a glob is matched against the cwd, a directory
/// is walked (respecting ignore files), and a plain path is taken as-is.
pub fn expand(paths: &[PathBuf]) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut out = Vec::new();
    for p in paths {
        let s = p.to_string_lossy().to_string();
        if s.contains(['*', '?', '[', '{']) {
            out.extend(glob_arg(&s, &cwd));
        } else if p.is_dir() {
            for entry in WalkBuilder::new(p)
                .standard_filters(true)
                .hidden(false)
                .build()
                .flatten()
            {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    out.push(entry.path().to_path_buf());
                }
            }
        } else {
            out.push(p.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn source_siblings(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in WalkBuilder::new(dir)
        .standard_filters(true)
        .hidden(false)
        .build()
        .flatten()
    {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let is_source = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| Lang::source_extensions().contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
        if is_source {
            out.push(entry.path().to_path_buf());
        }
    }
    out.sort();
    out
}

/// The resolved contract for one document: what it tracks, and the band and
/// comment ceiling it is checked against.
#[derive(Debug)]
pub struct Pair {
    pub doc: PathBuf,
    pub sources: Vec<PathBuf>,
    pub band: (f64, f64),
    pub comment_max: Option<f64>,
    pub metric: crate::metrics::Metric,
    pub self_check: bool,
    pub relation: Relation,
}

pub const DEFAULT_BAND: (f64, f64) = (4.0, 12.5);
pub const DEFAULT_SELF_BAND: (f64, f64) = (1.5, 4.0);
/// Gross-drift ceiling on comment share of non-blank source lines.
pub const DEFAULT_COMMENT_MAX: f64 = 0.5;

/// Resolve one doc path into a [`Pair`]. `cli_against` and `cli_band` override
/// everything; directives override the config; the config overrides the
/// `DESIGN.md`/`README.md` sibling heuristic.
pub fn resolve(
    doc: &Path,
    directive: &Directive,
    config: &Config,
    cli_against: &[String],
    cli_band: Option<(f64, f64)>,
    cli_comment_max: Option<f64>,
) -> Result<Pair> {
    let dir = doc.parent().unwrap_or_else(|| Path::new("."));
    let rule = config.match_rule(doc);

    let patterns: Vec<String> = if !cli_against.is_empty() {
        cli_against.to_vec()
    } else if !directive.against.is_empty() {
        directive.against.clone()
    } else if let Some(r) = rule {
        r.source.clone()
    } else {
        Vec::new()
    };

    let mut sources = if patterns.is_empty() {
        Vec::new()
    } else {
        glob_files(dir, &config.root, &patterns)
    };

    // Sibling heuristic: a DESIGN.md/README.md with no explicit target tracks
    // the parseable sources beside it.
    if sources.is_empty()
        && rule.is_none()
        && cli_against.is_empty()
        && directive.against.is_empty()
    {
        let name = doc.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_index_doc = matches!(name, "DESIGN.md" | "README.md");
        if is_index_doc {
            sources = source_siblings(dir);
        }
    }

    let self_check =
        sources.is_empty() && (rule.map(|r| r.self_check).unwrap_or(false) || directive.self_check);

    let band = cli_band
        .or(directive.band)
        .or_else(|| rule.and_then(|r| r.band))
        .unwrap_or(if self_check && sources.is_empty() {
            DEFAULT_SELF_BAND
        } else {
            DEFAULT_BAND
        });

    let comment_max = if let Some(c) = cli_comment_max {
        Some(c)
    } else if directive.comment_off {
        None
    } else {
        let configured = directive
            .comment_max
            .or_else(|| rule.and_then(|r| r.comment_max));
        configured.or(if self_check || sources.is_empty() {
            None
        } else {
            Some(DEFAULT_COMMENT_MAX)
        })
    };

    let metric = directive
        .metric
        .or_else(|| rule.and_then(|r| r.metric))
        .unwrap_or_default();

    let relation = directive
        .relation
        .or_else(|| rule.map(|r| r.relation))
        .unwrap_or_default();

    Ok(Pair {
        doc: doc.to_path_buf(),
        sources,
        band,
        comment_max,
        metric,
        self_check,
        relation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_config_upwards() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join(".skim.toml"), "[[pair]]\ndoc='x'\n").unwrap();
        let found = find_upwards(&dir.join("a/b"), ".skim.toml").unwrap();
        assert_eq!(found, dir.join(".skim.toml"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!("skim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }
}
