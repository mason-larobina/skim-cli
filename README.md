# skim

Syntax-aware source skimming and documentation-budget checks.

`skim` does two things with one tree-sitter parse:

- **Render a skeleton.** Given a source file, it prints the declarations —
  class, method and field signatures — with the bodies elided. Given a markdown
  file, it prints the heading/quote skeleton. This is the aider-style "show the
  signatures, hide the implementation" view.
- **Check a budget.** Given a document, it finds the source that document
  tracks and asserts that the document's size stays proportional to that source
  — on a square-root curve for a scouting index, or a straight ratio for a
  reference manual — and that comments do not exceed a ceiling.

Comments and code are counted from the parse, not from a regex: a `//` inside a
string is code, a block comment is one region rather than one line per `*`, and
a markdown fenced code block is not mistaken for prose.

## Install

```sh
cargo install --path .
# or, once published:
cargo install skim-cli    # installs the `skim` binary
```

> **Name note.** The crate is `skim-cli` because `skim` on crates.io is the Rust
> fuzzy finder. The binary is still `skim`. If you have the fuzzy finder
> installed, the two binaries collide on `PATH`.

## Usage

```sh
skim [PATHS]...            # render skeletons (default)
skim --check [PATHS]...    # run the budget gate; exit 1 on any violation
skim --count [PATHS]...    # print raw metrics, no verdict
```

`PATHS` may be files, directories (walked, respecting ignore files), or quoted
globs. With no paths, `.` is used.

```sh
skim src/                          # every source file's skeleton
skim 'src/**/*.java' | less        # one file's outline
skim --check docs/                 # check every DESIGN.md under docs/
skim --check --band 4,12 src/
skim --count --json src/ | jq .
```

Useful flags:

| Flag | Meaning |
| --- | --- |
| `--against <GLOB>` | Track this source for every doc (repeatable) |
| `--band LO,HI` | Override the band for every doc |
| `--comment-max R` | Override the comment ceiling (0.0–1.0) |
| `--metric nonblank\|code\|decls` | What the band measures against |
| `--config <FILE>` | Use this `.skim.toml` |
| `--lang <LANG>` | Force a grammar instead of inferring it |
| `--json` | Machine-readable output |

## Directives

Any file may carry a `Skim:` line — in a markdown blockquote or a source
comment. Multiple lines accumulate.

```markdown
> Skim: [4.0, 12.5]                          # band
> Skim: src/abc.java, src/def/**             # tracked source
> Skim: [0.3, 0.8] relation linear           # a reference manual, not an index
> Skim: comments 0.4                         # comment ceiling
> Skim: comments off                         # disable the comment check
> Skim: metric code                          # count code lines, not non-blank
> Skim: self                                 # check this file's own skeleton
```

```java
// skim: comments 0.6
```

`skim: self` is the todo-item check: the markdown file's heading and `>` lines
are treated as the index and the rest of the file as the body, and the same
sqrt band applies.

## Configuration

A `.skim.toml` at (or above) the working directory defines the contracts that no
heuristic can guess — the platform docs that track three directories, the
aggregate pool, the per-file exceptions. All paths are relative to the config's
`root` (default: the config file's directory). `source` patterns are relative to
each document's own directory unless they start with `/` (then `root`).

```toml
root = "/repo"

# A DESIGN.md tracks the source siblings by default; this makes it explicit.
[[pair]]
doc = "src/**/DESIGN.md"
source = ["**/*.java"]
band = [4.0, 12.5]

# Many docs measured as one summed pool, linearly against a source set.
[[pair]]
doc = "docs/*.md"
source = ["/src/core/**/*.java", "/src/platform/**/*.java"]
band = [0.3, 0.8]
relation = "linear"
aggregate = true

# A file whose docs are its own skeleton.
[[pair]]
doc = "todo/*_*.md"
source = []
self_check = true
band = [1.5, 4.0]
```

Without a config, a `DESIGN.md` or `README.md` tracks the parseable source
siblings under its own directory, and other markdown files are skipped unless
they opt in with `skim: self` or `--against`.

## Semantics

For a document with `D` non-blank lines and a source set measured at `S`:

- `relation = "sqrt"` (default): `D / sqrt(S)` must lie in `[lo, hi]`. This is
  the scouting-index curve — the bigger the code, the cheaper the doc should be
  in relative terms.
- `relation = "linear"`: `D / S` must lie in `[lo, hi]`. A reference manual
  that grows with what it documents.
- `aggregate = true`: every matching document is summed into one `D` and the
  source set is one `S`, producing a single result.

`S` is `--metric`:
`nonblank` (default, every non-blank line), `code` (non-blank lines that are not
comment-only), or `decls` (declaration count).

The comment ceiling is the aggregate share of non-blank source lines that are
comment-only. The default is `0.5`, a gross-drift backstop rather than a style
rule; override it per pair. A band failure is reported before a comment failure.

## Supported languages

tree-sitter grammars are compiled in for **Java** and **Markdown**. Every other
extension is treated as plain text (non-blank line counting only). Adding a
language means one profile in `src/lang.rs`: the grammar, its declaration node
kinds, its type node kinds, and its comment node kinds.

## Development

```sh
cargo test
cargo clippy --all-targets
cargo fmt
```

The test suite pins the contracts against real fixtures: body elision, the
markdown skeleton (including that lazy continuation and blank lines are not
index lines), sqrt and linear bands, directive overrides, and the comment gate.
