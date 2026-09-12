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
skim --check [PATHS]...    # run the budget gate; exit 1 on any violation
skim --count [PATHS]...    # print raw metrics, no verdict
skim --map [PATHS]...      # a ranked repo map, within a token budget
skim --referrers NAME [PATHS]...  # every use of a name, with its enclosing scope
```

`PATHS` may be files, directories (walked, respecting ignore files), or quoted
globs. With no paths, `.` is used.

```sh
skim src/                          # every source file's skeleton
skim --docs src/foo.java           # …with each declaration's doc comment
skim 'src/**/*.java' | less        # one file's outline
skim --map src/                    # the repo's most-referenced types
skim --map --deps src/core src/    # …biased toward what src/core depends on
skim --referrers FOOT_BAND src/    # every use of a name, with its scope
skim --check docs/                 # check every DESIGN.md under docs/
skim --check --band 4,12 src/
skim --count --json src/ | jq .
```

Useful flags:

| Flag | Meaning |
| --- | --- |
| `--docs` | Keep each declaration's leading doc comment |
| `--map` | Print a ranked repo map instead of a full skeleton |
| `--map-budget N` | Approximate token budget for `--map` (default 1024) |
| `--focus PATH` | Bias `--map` toward this file/dir (`--deps` is an alias; repeatable) |
| `--referrers NAME` | List every use of a name with its enclosing scope |
| `--against <GLOB>` | Track this source for every doc (repeatable) |
| `--band LO,HI` | Override the band for every doc |
| `--comment-max R` | Override the comment ceiling (0.0–1.0) |
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
reported too, and it only sees **code**, not comments or strings: a javadoc
`{@link Widget}` is not a hit. What it adds over `grep` is the enclosing
`Type.method`, so the result is the list of scopes to change before a rename
rather than a pile of line numbers. Methods, fields and constants work the same
as types, which the `--map` graph — whole types and top-level functions only —
cannot reach.

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
comments, and the map's ranking and focus.
