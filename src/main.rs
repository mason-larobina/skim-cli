//! `skim` — syntax-aware source skimming and documentation-budget checks.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use skim::lang::Lang;
use skim::metrics::Metric;
use skim::{check, discover, metrics, render};

/// Print a compressed skeleton of source files, or check that docs and comments
/// stay within their budget.
#[derive(Parser, Debug)]
#[command(name = "skim", version, about, long_about = None)]
struct Cli {
    /// Files, directories, or globs. Directories are walked.
    paths: Vec<PathBuf>,

    /// Check doc/comment budgets instead of rendering.
    #[arg(long)]
    check: bool,

    /// Print raw metrics only.
    #[arg(long)]
    count: bool,

    /// Override the tracked source for every checked doc (repeatable).
    #[arg(long, value_name = "GLOB")]
    against: Vec<String>,

    /// Override the sqrt band for every checked doc, e.g. `4.0,12.5`.
    #[arg(long, value_name = "LO,HI", value_parser = parse_band)]
    band: Option<(f64, f64)>,

    /// Override the comment-ratio ceiling (0.0..1.0).
    #[arg(long, value_name = "RATIO")]
    comment_max: Option<f64>,

    /// Source metric the band is measured against: nonblank, code, or decls.
    #[arg(long, value_name = "METRIC")]
    metric: Option<String>,

    /// Path to `.skim.toml` (searched upwards from the cwd by default).
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,

    /// Only print failing checks (and the summary).
    #[arg(short, long)]
    quiet: bool,

    /// Force a grammar instead of inferring it from the extension.
    #[arg(long, value_name = "LANG")]
    lang: Option<String>,

    /// List the languages compiled into this binary and exit.
    #[arg(long)]
    list_languages: bool,
}

fn parse_band(s: &str) -> Result<(f64, f64), String> {
    let (lo, hi) = s
        .split_once(',')
        .ok_or_else(|| "expected LO,HI".to_string())?;
    let lo: f64 = lo.trim().parse().map_err(|_| "bad LO".to_string())?;
    let hi: f64 = hi.trim().parse().map_err(|_| "bad HI".to_string())?;
    Ok((lo, hi))
}

fn main() -> ExitCode {
    // Restore default SIGPIPE handling so `skim … | head` exits quietly
    // instead of panicking on a broken pipe.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("skim: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    if cli.list_languages {
        print_languages(cli.json)?;
        return Ok(ExitCode::SUCCESS);
    }
    let forced = match &cli.lang {
        Some(name) => Some(
            Lang::from_name(name).ok_or_else(|| anyhow::anyhow!("unknown language {name:?}"))?,
        ),
        None => None,
    };
    let metric = match &cli.metric {
        Some(name) => Some(
            Metric::from_name(name).ok_or_else(|| anyhow::anyhow!("unknown metric {name:?}"))?,
        ),
        None => None,
    };

    let paths: Vec<PathBuf> = if cli.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        cli.paths.clone()
    };

    if cli.check {
        let config = discover::Config::load(cli.config.as_deref(), &cwd)?;
        let opts = check::CheckOptions {
            cli_against: cli.against.clone(),
            cli_band: cli.band,
            cli_comment_max: cli.comment_max,
            metric,
        };
        let report = check::run(&paths, &config, &opts)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            check::print_human(&report, cli.quiet);
        }
        return Ok(if report.violations > 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        });
    }

    if cli.count {
        let files = collect_files(&paths, forced);
        if cli.json {
            let rows: Vec<_> = files
                .iter()
                .map(|p| count_row(p, forced))
                .collect::<Result<Vec<_>>>()?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        } else {
            println!(
                "{:<52} {:>6} {:>6} {:>6} {:>6} {:>7}",
                "file", "lines", "blank", "code", "comment", "c-ratio"
            );
            for p in &files {
                let m = measure_path(p, forced)?;
                println!(
                    "{:<52} {:>6} {:>6} {:>6} {:>6} {:>7.2}",
                    truncate(&p.display().to_string(), 52),
                    m.total,
                    m.total - m.nonblank,
                    m.code,
                    m.comment,
                    m.comment_ratio()
                );
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    // Default: render skims.
    let files = collect_files(&paths, forced);
    let multiple = files.len() > 1;
    for p in &files {
        let text =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let lang = forced.unwrap_or_else(|| Lang::from_path(p));
        let rendered = if lang.is_doc() {
            render::render_markdown(&text, lang)?
        } else {
            render::render_source(&text, lang)?
        };
        if multiple {
            println!("==> {} <==", p.display());
        }
        if rendered.is_empty() {
            println!("(no declarations)");
        } else {
            print!("{rendered}");
        }
        if multiple {
            println!();
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(serde::Serialize)]
struct CountRow {
    file: String,
    total: usize,
    nonblank: usize,
    code: usize,
    comment: usize,
    decls: usize,
    comment_ratio: f64,
}

fn count_row(p: &Path, forced: Option<Lang>) -> Result<CountRow> {
    let m = measure_path(p, forced)?;
    Ok(CountRow {
        file: p.display().to_string(),
        total: m.total,
        nonblank: m.nonblank,
        code: m.code,
        comment: m.comment,
        decls: m.decls,
        comment_ratio: m.comment_ratio(),
    })
}

fn measure_path(p: &Path, forced: Option<Lang>) -> Result<metrics::FileMetrics> {
    let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
    let lang = forced.unwrap_or_else(|| Lang::from_path(p));
    if lang.is_doc() {
        Ok(metrics::measure_doc(&text, lang)?.0)
    } else {
        metrics::measure_source(&text, lang)
    }
}

#[derive(serde::Serialize)]
struct LanguageRow {
    name: &'static str,
    feature: &'static str,
    extensions: &'static [&'static str],
    document: bool,
}

fn print_languages(json: bool) -> Result<()> {
    let rows: Vec<LanguageRow> = skim::lang::PROFILES
        .iter()
        .map(|p| LanguageRow {
            name: p.name,
            feature: p.feature,
            extensions: p.extensions,
            document: p.doc,
        })
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "skim {} — {} languages compiled in\n",
        env!("CARGO_PKG_VERSION"),
        rows.len()
    );
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let ext_w = rows
        .iter()
        .map(|r| r.extensions.join(", ").len())
        .max()
        .unwrap_or(0);
    for r in &rows {
        let exts = r.extensions.join(", ");
        let kind = if r.document { "document" } else { "source" };
        println!(
            "{:<name_w$}  {:<ext_w$}  {:<8}  {}",
            r.name,
            exts,
            kind,
            r.feature,
            name_w = name_w,
            ext_w = ext_w,
        );
    }
    Ok(())
}

fn collect_files(paths: &[PathBuf], forced: Option<Lang>) -> Vec<PathBuf> {
    discover::expand(paths)
        .into_iter()
        .filter(|p| forced.is_some() || Lang::from_path(p).is_parseable())
        .collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
