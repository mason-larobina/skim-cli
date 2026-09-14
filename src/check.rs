//! The budget gate: sqrt/linear band and comment-ceiling checks over resolved
//! pairs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::directive::{self, Directive};
use crate::discover::{self, Config, Pair};
use crate::lang::Lang;
use crate::metrics::{self, FileMetrics, Metric};

/// One source file whose own comment share exceeds the ceiling.
#[derive(Serialize, Debug)]
pub struct OverFile {
    pub file: String,
    pub ratio: f64,
    pub comment_lines: usize,
    pub nonblank: usize,
}

/// Aggregate comment share for a pair's source set, plus the per-file offenders
/// that actually trip the ceiling.
#[derive(Serialize, Debug)]
pub struct CommentReport {
    pub ratio: f64,
    pub max: f64,
    pub comment_lines: usize,
    pub nonblank: usize,
    pub worst_file: Option<String>,
    pub worst_ratio: f64,
    /// Files over `max` (each with `nonblank >= comment_min`), worst first.
    pub over: Vec<OverFile>,
}

#[derive(Serialize, Debug)]
pub struct CheckReport {
    pub doc: String,
    pub sources: Vec<String>,
    pub doc_lines: usize,
    pub source_metric: usize,
    pub metric: String,
    pub relation: String,
    pub ratio: f64,
    pub band: [f64; 2],
    /// ok | under | over | comments | skipped
    pub status: String,
    pub self_check: bool,
    /// Where the contract came from (`config: <glob>`, `directive`, `cli`,
    /// `siblings`, `self`, `default`), so a mis-scoped run is self-diagnosing.
    pub origin: String,
    pub comment: Option<CommentReport>,
}

impl CheckReport {
    pub fn failed(&self) -> bool {
        matches!(self.status.as_str(), "under" | "over" | "comments")
    }
}

#[derive(Serialize, Debug)]
pub struct Report {
    pub checks: Vec<CheckReport>,
    pub violations: usize,
}

pub struct CheckOptions {
    pub cli_against: Vec<String>,
    pub cli_band: Option<(f64, f64)>,
    pub cli_comment_max: Option<f64>,
    pub cli_comment_min: Option<usize>,
    pub metric: Option<Metric>,
}

pub fn check_doc(doc: &Path, config: &Config, opts: &CheckOptions) -> Result<CheckReport> {
    let text =
        std::fs::read_to_string(doc).with_context(|| format!("reading {}", doc.display()))?;
    let mut direct: Directive = directive::parse(&text);
    if opts.metric.is_some() {
        direct.metric = opts.metric;
    }
    let pair = discover::resolve(
        doc,
        &direct,
        config,
        &opts.cli_against,
        opts.cli_band,
        opts.cli_comment_max,
        opts.cli_comment_min,
    )?;
    Ok(evaluate(&text, &pair))
}

enum Status {
    Ok,
    Under,
    Over,
    Comments,
    Skipped,
}

fn classify(ratio: f64, band: (f64, f64), comment_over: bool) -> Status {
    if ratio < band.0 {
        Status::Under
    } else if ratio > band.1 {
        Status::Over
    } else if comment_over {
        Status::Comments
    } else {
        Status::Ok
    }
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Ok => "ok",
        Status::Under => "under",
        Status::Over => "over",
        Status::Comments => "comments",
        Status::Skipped => "skipped",
    }
}

fn comment_report(
    metrics: &[(PathBuf, FileMetrics)],
    max: f64,
    min_source: usize,
) -> CommentReport {
    let mut comment_lines = 0;
    let mut nonblank = 0;
    let mut worst: Option<(&PathBuf, f64)> = None;
    let mut over: Vec<OverFile> = Vec::new();
    for (path, m) in metrics {
        comment_lines += m.comment;
        nonblank += m.nonblank;
        let ratio = m.comment_ratio();
        if worst.as_ref().map(|(_, w)| ratio > *w).unwrap_or(true) {
            worst = Some((path, ratio));
        }
        if m.nonblank >= min_source && ratio > max {
            over.push(OverFile {
                file: path.display().to_string(),
                ratio,
                comment_lines: m.comment,
                nonblank: m.nonblank,
            });
        }
    }
    over.sort_by(|a, b| {
        b.ratio
            .partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let ratio = if nonblank == 0 {
        0.0
    } else {
        comment_lines as f64 / nonblank as f64
    };
    CommentReport {
        ratio,
        max,
        comment_lines,
        nonblank,
        worst_file: worst.map(|(p, _)| p.display().to_string()),
        worst_ratio: worst.map(|(_, r)| r).unwrap_or(0.0),
        over,
    }
}

fn evaluate(text: &str, pair: &Pair) -> CheckReport {
    let doc_name = pair.doc.display().to_string();

    if pair.self_check && pair.sources.is_empty() {
        let (m, skeleton) =
            metrics::measure_doc(text, Lang::markdown()).unwrap_or((FileMetrics::default(), 0));
        let (lo, hi) = pair.band;
        let body = m.nonblank.max(1);
        let ratio = skeleton as f64 / (body as f64).sqrt();
        let status = classify(ratio, (lo, hi), false);
        return CheckReport {
            doc: doc_name,
            sources: vec![],
            doc_lines: skeleton,
            source_metric: m.nonblank,
            metric: "skeleton".to_string(),
            relation: "sqrt".to_string(),
            ratio,
            band: [lo, hi],
            status: status_name(status).to_string(),
            self_check: true,
            origin: pair.origin.clone(),
            comment: None,
        };
    }

    let doc_lines = metrics::nonblank(text);
    let metric = pair.metric;

    let mut source_metric = 0usize;
    let mut source_names = Vec::new();
    let mut measured: Vec<(PathBuf, FileMetrics)> = Vec::new();
    for src in &pair.sources {
        let Ok(body) = std::fs::read_to_string(src) else {
            continue;
        };
        let lang = Lang::from_path(src);
        let m = if lang.is_doc() {
            metrics::measure_doc(&body, lang)
                .map(|(m, _)| m)
                .unwrap_or_default()
        } else {
            metrics::measure_source(&body, lang).unwrap_or_default()
        };
        source_metric += m.metric(metric);
        source_names.push(src.display().to_string());
        measured.push((src.clone(), m));
    }

    if source_metric == 0 {
        return CheckReport {
            doc: doc_name,
            sources: source_names,
            doc_lines,
            source_metric: 0,
            metric: metric.name().to_string(),
            relation: pair.relation.name().to_string(),
            ratio: 0.0,
            band: [pair.band.0, pair.band.1],
            status: status_name(Status::Skipped).to_string(),
            self_check: false,
            origin: pair.origin.clone(),
            comment: None,
        };
    }

    let (lo, hi) = pair.band;
    let ratio = pair.relation.ratio(doc_lines, source_metric);
    let comment = pair
        .comment_max
        .map(|max| comment_report(&measured, max, pair.comment_min));
    let comment_over = comment
        .as_ref()
        .map(|c| !c.over.is_empty())
        .unwrap_or(false);
    let status = classify(ratio, (lo, hi), comment_over);

    CheckReport {
        doc: doc_name,
        sources: source_names,
        doc_lines,
        source_metric,
        metric: metric.name().to_string(),
        relation: pair.relation.name().to_string(),
        ratio,
        band: [lo, hi],
        status: status_name(status).to_string(),
        self_check: false,
        origin: pair.origin.clone(),
        comment,
    }
}

pub fn run(paths: &[PathBuf], config: &Config, opts: &CheckOptions) -> Result<Report> {
    let docs = collect_docs(paths);
    let mut checks = Vec::new();
    let mut consumed = std::collections::HashSet::new();

    // Aggregate rules own their docs and produce one summed report each.
    for rule in config.pairs.iter().filter(|r| r.aggregate) {
        let matched: Vec<&PathBuf> = docs
            .iter()
            .filter(|d| config.rule_matches(rule, d))
            .collect();
        if matched.is_empty() {
            continue;
        }
        for d in &matched {
            consumed.insert((*d).clone());
        }
        if let Some(report) = evaluate_aggregate(rule, &matched, config, opts) {
            checks.push(report);
        }
    }

    for doc in docs {
        if consumed.contains(&doc) {
            continue;
        }
        match check_doc(&doc, config, opts) {
            Ok(report) => checks.push(report),
            Err(e) => eprintln!("skim: {e}"),
        }
    }
    checks.sort_by(|a, b| {
        b.ratio
            .partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let violations = checks.iter().filter(|c| c.failed()).count();
    Ok(Report { checks, violations })
}

fn evaluate_aggregate(
    rule: &crate::discover::PairRule,
    docs: &[&PathBuf],
    config: &Config,
    opts: &CheckOptions,
) -> Option<CheckReport> {
    let mut doc_lines = 0usize;
    for d in docs {
        if let Ok(text) = std::fs::read_to_string(d) {
            doc_lines += metrics::nonblank(&text);
        }
    }
    let sources = discover::glob_files(&config.root, &config.root, &rule.source);
    let metric = rule.metric.unwrap_or_default();
    let mut source_metric = 0usize;
    let mut source_names = Vec::new();
    let mut measured: Vec<(PathBuf, FileMetrics)> = Vec::new();
    for src in &sources {
        let Ok(body) = std::fs::read_to_string(src) else {
            continue;
        };
        let lang = Lang::from_path(src);
        let m = if lang.is_doc() {
            metrics::measure_doc(&body, lang)
                .map(|(m, _)| m)
                .unwrap_or_default()
        } else {
            metrics::measure_source(&body, lang).unwrap_or_default()
        };
        source_metric += m.metric(metric);
        source_names.push(src.display().to_string());
        measured.push((src.clone(), m));
    }
    if source_metric == 0 {
        return None;
    }
    let (lo, hi) = opts
        .cli_band
        .or(rule.band)
        .unwrap_or(discover::DEFAULT_BAND);
    let ratio = rule.relation.ratio(doc_lines, source_metric);
    let comment = comment_report(
        &measured,
        opts.cli_comment_max
            .or(rule.comment_max)
            .unwrap_or(discover::DEFAULT_COMMENT_MAX),
        opts.cli_comment_min
            .or(rule.comment_min)
            .unwrap_or(discover::DEFAULT_COMMENT_MIN_SOURCE),
    );
    let comment_over = !comment.over.is_empty();
    let status = classify(ratio, (lo, hi), comment_over);
    Some(CheckReport {
        doc: format!("{} ({} docs, aggregate)", rule.doc_pattern, docs.len()),
        sources: source_names,
        doc_lines,
        source_metric,
        metric: metric.name().to_string(),
        relation: rule.relation.name().to_string(),
        ratio,
        band: [lo, hi],
        status: status_name(status).to_string(),
        self_check: false,
        origin: format!("config: {} (aggregate)", rule.doc_pattern),
        comment: Some(comment),
    })
}

/// Every markdown file reachable from the given paths (dirs and globs are
/// expanded, respecting ignore files).
pub fn collect_docs(paths: &[PathBuf]) -> Vec<PathBuf> {
    discover::expand(paths)
        .into_iter()
        .filter(|p| Lang::from_path(p).is_doc())
        .collect()
}

pub fn print_human(report: &Report, quiet: bool) {
    let mut printed = false;
    let mut skipped = 0usize;
    for c in &report.checks {
        // A skipped doc has no tracked source; listing 170-odd of them buries
        // the real verdicts, so they are counted and reported once below.
        if c.status == "skipped" {
            skipped += 1;
            continue;
        }
        if quiet && !c.failed() {
            continue;
        }
        printed = true;
        let mark = if c.status == "ok" { "ok  " } else { "FAIL" };
        let origin = if c.origin.is_empty() {
            String::new()
        } else {
            format!(" [{}]", c.origin)
        };
        if c.self_check {
            println!(
                "{mark} {:>7.2}  {}  (skeleton={} body={} band {:.2}..{:.2}){origin}",
                c.ratio, c.doc, c.doc_lines, c.source_metric, c.band[0], c.band[1]
            );
        } else {
            println!(
                "{mark} {:>7.2}  {}  (doc={} {}={} {} band {:.2}..{:.2}){origin}",
                c.ratio,
                c.doc,
                c.doc_lines,
                c.metric,
                c.source_metric,
                c.relation,
                c.band[0],
                c.band[1]
            );
        }
        if let Some(cm) = &c.comment {
            if !cm.over.is_empty() {
                println!(
                    "       comment ceiling {:.2}: {} file(s) over ({}/{} nonblank overall)",
                    cm.max,
                    cm.over.len(),
                    cm.comment_lines,
                    cm.nonblank
                );
                for o in cm.over.iter().take(10) {
                    println!(
                        "         {:.2}  {}/{}  {}",
                        o.ratio, o.comment_lines, o.nonblank, o.file
                    );
                }
                if cm.over.len() > 10 {
                    println!("         ... {} more", cm.over.len() - 10);
                }
            }
        }
    }
    if printed {
        println!();
    }
    if skipped > 0 && !quiet {
        println!("({skipped} doc(s) skipped: no tracked source)");
    }
    println!(
        "{} docs, {} violation(s)",
        report.checks.len(),
        report.violations
    );
}
