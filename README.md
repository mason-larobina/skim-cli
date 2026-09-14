# skim

Syntax-aware source skimming and documentation-budget checks.

`skim` does two things with one tree-sitter parse:

- **Render a skeleton.** Given a source file, it prints the declarations —
  class, method and field signatures — with the bodies elided. Given a markdown
  file, it prints the heading/quote skeleton. This is the aider-style "show the
  signatures, hide the implementation" view. `--docs` keeps each declaration's
  doc comment as well.
- **Map what matters.** `--map` ranks the types and top-level functions under a
  path by how much the code references them, then prints the top of that
  ranking within a token budget — a small, relevance-ranked skeleton of the
  repo. `--referrers` answers the reverse question: every use of one name,
  labelled with the scope that contains it.
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
skim --lines [PATHS]...    # …with each declaration's source line number
skim --check [PATHS]...    # run the budget gate; exit 1 on any violation
skim --count [PATHS]...    # print raw metrics, no verdict
skim --map [PATHS]...      # a ranked repo map, within a token budget
skim --referrers NAME [PATHS]...  # every use of a name, with its enclosing scope
skim --grep PATTERN [PATHS]...    # skeleton lines matching regex PATTERN (the index, not prose)
```

`PATHS` may be files, directories (walked, respecting ignore files), or quoted
globs. With no paths, `.` is used.

```sh
skim src/                          # every source file's skeleton
skim --docs src/foo.java           # …with each declaration's doc comment
skim --lines --docs src/foo.java   # …and each declaration's source line
skim 'src/**/*.java' | less        # one file's outline
skim --map src/                    # the repo's most-referenced types
skim --map --deps src/core src/    # …biased toward what src/core depends on
skim --referrers FOOT_BAND src/    # every use of a name, with its scope
skim --referrers '"Found"' --text src/   # …including comments and strings
skim --check docs/                 # check every DESIGN.md under docs/
skim --check --band 4,12 src/
skim --count --json src/ | jq .
```

Useful flags:

| Flag | Meaning |
| --- | --- |
| `--docs` | Keep each declaration's leading doc comment |
| `--lines` | Prefix each declaration (or markdown skeleton line) with its source line |
| `--map` | Print a ranked repo map instead of a full skeleton |
| `--map-budget N` | Approximate token budget for `--map` (default 4096) |
| `--focus PATH` | Bias `--map` toward this file/dir (`--deps` is an alias; repeatable) |
| `--referrers NAME` | List every use of a name with its enclosing scope |
| `--grep PATTERN` | Search the rendered skeletons (headings, quotes, signatures) for regex `PATTERN` |
| `--fixed-strings`, `-F` | Treat `--grep PATTERN` as a literal string, not a regular expression |
| `--text` | With `--referrers`, also match comments and string literals |
| `--ignore-case`, `-i` | Fold case for `--grep` and `--referrers` |
| `--against <GLOB>` | Track this source for every doc (repeatable) |
| `--band LO,HI` | Override the band for every doc |
| `--comment-max R` | Override the comment ceiling (0.0–1.0) |
| `--comment-min N` | Only assert the comment ceiling above N non-blank source lines |
| `--metric nonblank\|code\|decls` | What the band measures against |
| `--config <FILE>` | Use this `.skim.toml` |
| `--lang <LANG>` | Force a grammar instead of inferring it |
| `--list-languages` | List the compiled-in languages (with `--json`) |
| `--json` | Machine-readable output |
| `-q, --quiet` | Only failing checks and the summary |

## Doc comments (`--docs`)

A skeleton is signatures only. `--docs` also keeps the comment block immediately
above each declaration — the javadoc, `#` block, or docstring header that carries
the contract a signature cannot:

```sh
skim --docs src/core/Program.java
```

A comment that shares a line with the statement before it is a trailing comment,
not a doc comment, and is left out. This is the flag to reach for when you need
to know not just *what* a method is but how to call it.

## Line numbers (`--lines`)

A skeleton elides the bodies, so a reader who wants to open one needs its line.
`--lines` prefixes each declaration (and each markdown skeleton line) with its
1-based source line, so the skeleton doubles as an index into `read`:

```sh
skim --lines --docs src/core/Node.java
```

```text
   13  /**
       * One node of the draw list: an extent, an opacity flag, …
       */
   13  public abstract class Node {
   59      public abstract Object token();
   78      public abstract void draw(ImageBuffer target, Box2d damage, Stats stats);
```

The line is the declaration's own `start_position`, so `--lines` composes with
`--docs` without numbering the doc block.

## Repo map (`--map`)

`--map` answers "what should I look at first". It parses every source file under
the paths, builds a reference graph (a node is a type or a top-level function;
 an edge is an identifier reference), runs PageRank over it, and prints the
 highest-ranked definitions within a token budget, grouped by file.

```sh
skim --map src/                                       # the repo's central types
skim --map --focus src/core/GameProgram.java src/     # what GameProgram relies on
skim --map --map-budget 2048 src/ --json
```

- `--focus <PATH>` (alias `--deps`) biases the ranking toward those files.
  Without it the map is the repo's global shape; with it, the map is the API
  surface relevant to the files you named. The bias flows *forward*: the result
  is what those paths depend on, not what depends on them — use `--referrers`
  for the other direction.
- `--map-budget N` sets an approximate token ceiling (about four bytes per
  token).
- Ranking is at the **type** level. Nodes are types and top-level functions;
  methods and fields are not, so an accessor never competes with its class, and
  a reference from inside a type attributes to that type.
- References resolve loosely: a name defined in the same file shadows a
  same-named definition elsewhere, and cross-file references weigh more than
  local ones. There is no type or scope resolution — the map is a heuristic for
  orientation, not a compiler.

## Referrers (`--referrers`)

`--map` shows what a file *depends on*. `--referrers` answers the other
direction: where a name is *used*. It matches every identifier equal to the name
and prints each hit labelled with the definition that contains it — `Type.method`,
`Type`, or file scope:

```sh
skim --referrers AnchoredLayout src/      # which scopes reference the type
skim --referrers FOOT_BAND src/           # every use of a constant
skim --referrers hasCabinet --json src/   # machine-readable
```

```
src/bobble/Bobble.java:
  217    Bobble.anchored
  335    Bobble.layout
...
76 occurrence(s) of "AnchoredLayout" in 17 file(s)
```

It exits 1 when the name is absent, so it drops into a shell test like `grep`.

The match is lexical — the same thing `grep` does — so a same-named local is
reported too, and by default it only sees **code**, not comments or strings: a
javadoc `{@link Widget}` is not a hit and neither is the `"Found"` label a
rename would have to change. What it adds over `grep` is the enclosing
`Type.method`, so the result is the list of scopes to change before a rename
rather than a pile of line numbers. Methods, fields and constants work the same
as types, which the `--map` graph — whole types and top-level functions only —
cannot reach.

`--text` also matches comments and string literals (as whole words), which is
what closes the rename gap; `--ignore-case` folds ASCII case on either kind:

```sh
skim --referrers Found --text src/           # the identifiers, the "Found" label and its docs
skim --referrers foot_band --ignore-case src/
```

## Skeleton search (`--grep`)

`--map` ranks what a tree *depends on*; `--referrers` finds where a name is
*used*; `--grep` searches the **skeleton itself** — the headings and quote lines
of every markdown file, and every code declaration signature (plus doc comments
with `--docs`). It is `grep` over the index, not the prose: a hit names a
concept, a design decision, or a definition, so it is the first pass for finding
what a task touches before opening a single body.

```sh
skim --grep seat todo/ pico/          # which docs carry the seat model?
skim --grep 'FOOT_BAND' src/          # docs and code that name a constant
skim --grep 'seat|chair' todo/        # regex: either word
skim -F --grep 'a.b' src/             # literal dot, not a wildcard
skim --grep centre-map -i pico/*.md
```

`PATTERN` is a regular expression by default — what `grep -E` accepts. `.`, `*`,
`+`, `?`, character classes, `(...)` groups, and `|` alternation all work, and
`^`/`$` anchor to the start and end of the skeleton line. A pattern containing
these metacharacters matches differently than it did before regex support; use
`--fixed-strings` (`-F`) when you mean them literally, so a `.` in a dotted name
matches only `.`. `--ignore-case` (`-i`) folds case, and an invalid pattern
fails with exit 2.

Each hit prints the file, its 1-based source line, and the skeleton line, so it
opens straight in `read`; `--json` emits `{file, line, skeleton}` objects. Like
grep, it exits 1 when nothing matches.

```
pico/BEST_PRACTICES.md:
  140  > `core.Seats` is the lock: seat 0 is p1, pre-seated by `openedBy` and logging

10 match(es) in 3 file(s)
```

## Directives

Any file may carry a `Skim:` line — in a markdown blockquote or a source
comment. Multiple lines accumulate.

```markdown
> Skim: [4.0, 12.5]                          # band
> Skim: src/abc.java, src/def/**             # tracked source
> Skim: [0.3, 0.8] relation linear           # a reference manual, not an index
> Skim: comments 0.4                         # comment ceiling
> Skim: comment-min 200                      # …asserted only above 200 non-blank lines
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

A `.skim.toml` at or above the working directory, up to the repository root
(the directory holding `.git`, which is a file in a linked worktree), defines
the contracts that no heuristic can guess — the platform docs that track three
directories, the aggregate pool, the per-file exceptions. The search stops at
the repository root, so a config above the checkout is never picked up. All
paths are relative to the config's `root` (default: the config file's
directory). `source` patterns are relative to each document's own directory
unless they start with `/` (then `root`).

```toml
root = "/repo"

# A DESIGN.md tracks the source siblings by default; this makes it explicit.
[[pair]]
doc = "src/**/DESIGN.md"
source = ["**/*.java"]
band = [4.0, 12.5]
comment_max = 0.5
comment_min = 100

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

The comment ceiling is per **file**, not the pair aggregate: a source file whose
own comment-only lines exceed `comment_max` of its non-blank lines (a linear
`C/S`) trips it. A small file's share is dominated by its header and the parse's
comment kinds, so a file below `comment_min` non-blank lines is not asserted; the
defaults are `0.5` and `100`, and a float `C/S` is the only relation that fits
the comment data (a sqrt normalizer tracks codebase size, not comment density).
The pair aggregate is still reported for context. A band failure is reported
before a comment failure.

Each result line names the contract it was checked against in brackets —
`[config: <glob>]`, `[directive]`, `[siblings]`, `[self]`, `[cli]` — so a run
that unexpectedly fell back to the sibling heuristic is visible instead of
silent. A doc with no tracked source is counted in one summary line rather than
listed; `--json` keeps the per-doc detail either way.

## Supported languages

Every grammar is an optional Cargo feature, so the installed binary carries only
the languages you ask for. The **default** enables all of them — an unfamiliar
extension then gets real syntax-aware treatment rather than silently falling
back to plain text.

| Language | Feature | Extensions |
| --- | --- | --- |
| Java | `lang-java` | `.java` |
| Python | `lang-python` | `.py`, `.pyi` |
| JavaScript | `lang-javascript` | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `lang-typescript` | `.ts`, `.mts`, `.cts`, `.tsx` |
| Rust | `lang-rust` | `.rs` |
| Go | `lang-go` | `.go` |
| C | `lang-c` | `.c`, `.h` |
| C++ | `lang-cpp` | `.cc`, `.cpp`, `.cxx`, `.hpp`, `.hh`, `.hxx` |
| C# | `lang-csharp` | `.cs` |
| Ruby | `lang-ruby` | `.rb`, `.rake`, `.gemspec` |
| PHP | `lang-php` | `.php` |
| Kotlin | `lang-kotlin` | `.kt`, `.kts` |
| Swift | `lang-swift` | `.swift` |
| Scala | `lang-scala` | `.scala`, `.sc`, `.sbt` |
| Lua | `lang-lua` | `.lua` |
| Bash | `lang-bash` | `.sh`, `.bash`, `.zsh` |
| HTML | `lang-html` | `.html`, `.htm` |
| CSS | `lang-css` | `.css` |
| JSON | `lang-json` | `.json`, `.jsonc` |
| YAML | `lang-yaml` | `.yaml`, `.yml` |
| TOML | `lang-toml` | `.toml` |
| Markdown | always | `.md`, `.markdown` |

Markdown is always compiled in — it is the document renderer and the language of
the band checks. HTML/JSON/YAML/TOML carry comments but no declaration skeleton,
so they participate in the comment ceiling and the band denominator only.

`skim --list-languages` prints the build's actual set, its extensions and the
Cargo feature that provides each.

To slim the binary (markdown-only is ~3.7 MB, all languages ~34 MB):

```sh
# A Python- and TypeScript-only build.
cargo install --path . --no-default-features --features lang-python,lang-typescript
```

### Adding a language

One entry in `src/lang.rs`: the optional dependency and feature in `Cargo.toml`,
a `fn` returning the grammar, a `static Profile` (declaration kinds, type kinds,
body kinds, comment kinds, braces, elide suffix), and one `PROFILES` entry. The
node kinds come from the grammar's `node-types.json`.

## Development

```sh
cargo test
cargo clippy --all-targets
cargo fmt
```

The test suite pins the contracts against real fixtures: body elision, the
markdown skeleton (including that lazy continuation and blank lines are not
index lines), sqrt and linear bands, directive overrides, the comment gate, doc
comments, line-number prefixes, the referrer's text/case flags, the check's
origin labels, config-rule path-spelling normalization, and the map's ranking
and focus.
