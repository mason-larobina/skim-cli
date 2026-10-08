//! `skim` — syntax-aware source skimming and documentation-budget checks.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use skim::lang::Lang;
use skim::map;
use skim::metrics::Metric;
use skim::{check, discover, metrics, render, search};

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

    /// Print raw metrics only. `decls` counts each declarator, not each
    /// declaration, so one field holding N constants counts N.
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

    /// Minimum non-blank source lines before the comment ceiling applies.
    #[arg(long, value_name = "N")]
    comment_min: Option<usize>,

    /// Source metric the band is measured against: nonblank, code, or decls.
    /// `decls` counts each declarator, not each declaration, so one field
    /// holding N constants counts N.
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

    /// Keep each declaration's leading doc comment when rendering.
    #[arg(long)]
    docs: bool,

    /// Prefix each declaration (or markdown skeleton line) with its 1-based
    /// source line, so `read` can jump straight to the body skim elided.
    #[arg(long)]
    lines: bool,

    /// Print a ranked repo map: the definitions the scanned files reference
    /// most, within a token budget.
    #[arg(long)]
    map: bool,

    /// Approximate token budget for --map (about four bytes per token).
    #[arg(long, value_name = "N", default_value_t = 4096)]
    map_budget: usize,

    /// Bias --map toward these files or directories (repeatable). `--deps` is
    /// an alias, since the bias reveals what these paths depend on.
    #[arg(long = "focus", visible_alias = "deps", value_name = "PATH")]
    map_focus: Vec<PathBuf>,

    /// List every occurrence of NAME with its enclosing Type.method — a
    /// structural grep for finding the call sites before a rename.
    #[arg(long, value_name = "NAME")]
    referrers: Option<String>,

    /// Search the rendered skeletons for regex PATTERN and print each hit
    /// with its file and source line. This greps the *index* — markdown
    /// headings and quote lines, code declaration signatures — not the prose,
    /// so it is the scouting pass for the concepts and modules a task touches.
    /// PATTERN is a regular expression, like `grep -E`; see --fixed-strings.
    /// Accepted forms: `--grep PATTERN`, the match flags before `--grep` or
    /// after the pattern (`-iE --grep PAT`), and `--grep=<pattern>` for a
    /// pattern that itself begins with `-`.
    #[arg(long, value_name = "PATTERN")]
    grep: Option<String>,

    /// With --grep, search raw source lines (comments and strings included)
    /// instead of the skeleton, and label each hit with its enclosing
    /// `Type.method`. This is the structural pass the skeleton elides: an
    /// assignment (`x.y =`), a call shape (`new Box2d(`), a literal. Markdown
    /// prose is not searched. Body search belongs to `--grep`: write
    /// `--grep PATTERN --body`, with the match flags before `--grep` or after
    /// the pattern.
    #[arg(long, requires = "grep", conflicts_with = "referrers")]
    body: bool,

    /// Treat --grep PATTERN as a literal string, not a regular expression, so
    /// `.` and `*` match themselves (like `grep -F`); write `--grep PATTERN -F`
    /// or put `-F` before `--grep`.
    #[arg(short = 'F', long = "fixed-strings")]
    fixed_strings: bool,

    /// Accepted no-op: PATTERN is already an ERE, so this changes nothing. It
    /// exists so `-iE`/`-E` parse like the `grep` habit they come from.
    #[arg(short = 'E', long = "extended-regexp")]
    extended_regexp: bool,

    /// With --referrers, also match comments and string literals (a rename's
    /// `"Found"` label and its javadoc), not just code identifiers.
    #[arg(long)]
    text: bool,

    /// Fold case when matching --grep patterns and --referrers names.
    #[arg(short = 'i', long = "ignore-case")]
    ignore_case: bool,
}

fn parse_band(s: &str) -> Result<(f64, f64), String> {
    let (lo, hi) = s
        .split_once(',')
        .ok_or_else(|| "expected LO,HI".to_string())?;
    let lo: f64 = lo.trim().parse().map_err(|_| "bad LO".to_string())?;
    let hi: f64 = hi.trim().parse().map_err(|_| "bad HI".to_string())?;
    Ok((lo, hi))
}

/// Parse the CLI, first normalizing the `--grep -iE PATTERN` wedge. Falls back
/// to a plain parse when any argument is not valid UTF-8, so a non-UTF-8 argv
/// can never panic the normalizer.
fn parse_cli() -> Cli {
    let args: Option<Vec<String>> = std::env::args_os().map(|a| a.into_string().ok()).collect();
    match args {
        Some(argv) => Cli::parse_from(normalize_grep_args(argv)),
        None => Cli::parse(),
    }
}

/// Rewrite `--grep -iE PATTERN` into the form clap accepts, `-iE --grep PATTERN`.
/// clap treats any short-flag token directly after `--grep` as a new argument
/// (`is_new_arg`), so the pattern must not sit behind the flags: the command
/// exits 2 today even though `-E` parses. Only `--grep` is rewritten and only
/// when the token right after it is a known match flag followed by a real
/// pattern; everything else, including `--grep=PATTERN`, passes through.
fn normalize_grep_args(argv: Vec<String>) -> Vec<String> {
    let Some(i) = argv.iter().position(|a| a == "--grep") else {
        return argv;
    };
    let Some(next) = argv.get(i + 1) else {
        return argv;
    };
    if !is_match_flag(next) {
        return argv;
    }
    // First token after the flag run that is not a match flag: the pattern.
    let Some(j) = (i + 1..argv.len()).find(|&k| !is_match_flag(&argv[k])) else {
        return argv;
    };
    let mut out = Vec::with_capacity(argv.len());
    out.extend_from_slice(&argv[..i]);
    out.extend_from_slice(&argv[i + 1..j]);
    out.push("--grep".to_string());
    out.push(argv[j].clone());
    out.extend_from_slice(&argv[j + 1..]);
    out
}

/// A `--grep` match flag that may be wedged between `--grep` and its pattern:
/// the long forms, or a short cluster of `-i`/`-E`/`-F`/`-q` (`-iE`, `-Fq`).
fn is_match_flag(t: &str) -> bool {
    matches!(
        t,
        "--ignore-case"
            | "--extended-regexp"
            | "--fixed-strings"
            | "--body"
            | "--text"
            | "--docs"
            | "--json"
            | "--quiet"
            | "--lines"
    ) || is_short_flag_cluster(t)
}

/// `-iE`, `-iF`, `-Fq`, …: a leading `-`, then one or more short flag chars. A
/// bare `-`, `--`, `-1` or `-x` is not a cluster and so is not a match flag.
fn is_short_flag_cluster(t: &str) -> bool {
    let Some(rest) = t.strip_prefix('-') else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| matches!(c, 'i' | 'E' | 'F' | 'q'))
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
    let cli = parse_cli();
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

    // `--grep`, `--referrers` and `--map` walk the same paths: expand once so a
    // typo exits 2 naming the requested paths, not a "0 results" report, and
    // `--grep`'s summary can say how many files were scanned.
    let scan = if cli.referrers.is_some() || cli.grep.is_some() || cli.map {
        let expanded = discover::expand(&paths);
        if !expanded.iter().any(|p| p.is_file()) {
            let requested: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            eprintln!("skim: no files matched {}", requested.join(" "));
            return Ok(ExitCode::from(2));
        }
        Some(expanded)
    } else {
        None
    };
    let scanned = scan
        .as_ref()
        .map(|e| e.iter().filter(|p| p.is_file()).count())
        .unwrap_or(0);

    if let Some(symbol) = &cli.referrers {
        let opts = map::MapOptions {
            force: forced,
            text: cli.text,
            ignore_case: cli.ignore_case,
            ..map::MapOptions::default()
        };
        let report = map::referrers(&paths, symbol, &opts)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", report.text());
        }
        if report.occurrences == 0 {
            if let Some(hint) = search::referrers_shape_hint(symbol) {
                eprintln!("{hint}");
            }
        }
        return Ok(if report.occurrences == 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        });
    }

    if let Some(pattern) = &cli.grep {
        let opts = search::GrepOptions {
            force: forced,
            docs: cli.docs,
            ignore_case: cli.ignore_case,
            fixed_strings: cli.fixed_strings,
        };
        // The markdown half of the scan set, so a `--body` miss over prose a
        // doc carries can be diagnosed instead of read as an absent concept.
        let scanned_markdown = if cli.body {
            scan.as_ref()
                .map(|e| e.iter().filter(|p| Lang::from_path(p).is_doc()).count())
                .unwrap_or(0)
        } else {
            0
        };
        let empty = if cli.body {
            let hits = search::grep_body(&paths, pattern, &opts)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                print!("{}", body_report(&hits, scanned));
            }
            hits.is_empty()
        } else {
            let hits = search::grep(&paths, pattern, &opts)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                let mut out = String::new();
                let mut last: Option<&str> = None;
                let mut files = 0usize;
                for m in &hits {
                    if last != Some(m.file.as_str()) {
                        out.push_str(&format!("{}:\n", m.file));
                        last = Some(m.file.as_str());
                        files += 1;
                    }
                    out.push_str(&format!("{:>5}  {}\n", m.line, m.skeleton));
                }
                out.push_str(&format!(
                    "\n{} match(es) in {} file(s) (scanned {})\n",
                    hits.len(),
                    files,
                    scanned
                ));
                print!("{out}");
            }
            hits.is_empty()
        };
        if empty {
            if let Some(hint) = search::zero_match_hint(pattern, cli.fixed_strings) {
                eprintln!("{hint}");
            } else if let Some(hint) = search::skeleton_miss_hint(cli.body) {
                eprintln!("{hint}");
            }
            if let Some(hint) = search::body_doc_hint(scanned_markdown) {
                eprintln!("{hint}");
            }
        }
        return Ok(if empty {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        });
    }

    if cli.map {
        let context = if !cli.map_focus.is_empty() {
            cli.map_focus.clone()
        } else if !cli.paths.is_empty() {
            cli.paths.clone()
        } else {
            Vec::new()
        };
        let opts = map::MapOptions {
            force: forced,
            docs: cli.docs,
            budget_tokens: cli.map_budget,
            ..map::MapOptions::default()
        };
        let result = map::run(&paths, &context, &opts)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&result.entries)?);
        } else {
            print!("{}", result.text);
        }
        return Ok(ExitCode::SUCCESS);
    }

    if cli.check {
        let config = discover::Config::load(cli.config.as_deref(), &cwd)?;
        let opts = check::CheckOptions {
            cli_against: cli.against.clone(),
            cli_band: cli.band,
            cli_comment_max: cli.comment_max,
            cli_comment_min: cli.comment_min,
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
            render::render_markdown_opts(&text, lang, cli.lines)?
        } else {
            render::render_source_opts(&text, lang, cli.docs, cli.lines)?
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

/// Human-readable `--grep --body` output: the line, its enclosing scope, and
/// the matched source line, grouped by file.
fn body_report(hits: &[search::BodyMatch], scanned: usize) -> String {
    let mut out = String::new();
    let mut last: Option<&str> = None;
    let mut files = 0usize;
    for m in hits {
        if last != Some(m.file.as_str()) {
            out.push_str(&format!("{}:\n", m.file));
            last = Some(m.file.as_str());
            files += 1;
        }
        let enc = if m.enclosing.is_empty() {
            "(file scope)"
        } else {
            &m.enclosing
        };
        out.push_str(&format!("  {:<6} {:<24} {}\n", m.line, enc, m.text));
    }
    out.push_str(&format!(
        "\n{} match(es) in {} file(s) (scanned {})\n",
        hits.len(),
        files,
        scanned
    ));
    out
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
