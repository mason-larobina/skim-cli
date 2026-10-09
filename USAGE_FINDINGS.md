# What agent sessions actually do with skim

> **Current.** An audit of 1,479 `skim` invocations across 121 Pi session
> transcripts, with the failure modes it exposed and the fixes this branch
> lands.

This is a field report, not a design doc. It records what real agent sessions
did with the tool, where the output was consumed, where it was thrown away, and
which of those losses the binary can prevent itself rather than leaving to a
prompt-side recipe.

## Method

The corpus is the Pi session store (`~/.pi/agent/sessions`, 976 transcripts,
1.4 GB). 395 transcripts mention `skim`; 121 actually invoke it. Every `bash`
tool call whose command contains `skim` was paired with its `toolResult` and
with the following assistant thinking, text and tool calls, giving **1,479
calls / 1,040 skim command segments**. Segments were split into *dev* sessions
(skim's own Rust source, `cargo`) and *consumer* sessions (scouting a Java
game repo), leaving **783 consumer segments**.

Every symptom quoted below was then re-run against the shipped binary
(`skim 0.4.0`) at the commit this branch forks from, so nothing here is a stale
observation. Where a finding was already fixed during the audit window, it is
recorded under "Already handled" instead.

## Did it pay off?

The intended flows work. Classifying what the *next* tool call after a consumer
skim invocation was:

| mode | n | next call was `read` | next call fell back to `rg`/`grep` | re-ran skim |
| --- | --- | --- | --- | --- |
| `--lines` | 26 | **61.5%** | 11.5% | 11.5% |
| `--grep` | 96 | 36.5% | **34.4%** | 8.3% |
| `--map` | 80 | 28.8% | 25.0% | 23.8% |
| `--referrers` | 46 | 21.7% | 23.9% | 21.7% |
| `--docs` | 71 | 12.7% | 36.6% | 25.4% |

Median result sizes are modest (600 B - 3.9 kB), but the tail is not: `--map`
p90 20 kB / max 41 kB, `--docs` max 41 kB, `--lines` max 37 kB, and a
directory-wide `--docs` over a platform tree emits ~13.5 k lines in one shot.

### Wins worth keeping

- **`--lines` is the strongest recipe.** A guestbook-refactor session
  (2026-10-06) is ~20 consecutive `skim --lines <file>` -> `read` pairs; the
  line prefix is what makes `read` cheap. "`--lines` is what indexes into
  `read`" is earned.
- **`--docs` changes decisions.** Laying `skim --docs core/ActionButton.java`
  next to a planned constants move showed the class documents itself as owning
  "no pixels, no layout" - the plan changed home because of the javadoc skim
  printed.
- **`--referrers` scopes a migration.** `skim --referrers Shelf mc8/guestbook`
  produced the file and method list that was written straight into a task
  brief.
- **The `--grep --body` split is right.** Sessions that learned it used the
  skeleton for "which docs and modules carry this concept" and `--body` for the
  assignment or call shape, exactly as documented.

### The tax

Roughly a third of every scavenging query ends in a fallback to `rg`/`grep`,
and agents said so in their own words:

- "The real problem: an agent reaching for a token that lives in bodies types
  `skim --grep token` and gets nothing, then falls back to `rg`."
- "Earlier `skim --referrers` gave nothing useful" (query was `panel()`, a call
  shape).
- "`skim --referrers pass` earlier was noisy."

Those are not "wrong tool" - they are five fixable failure modes.

## Gaps, ranked

### 1. A flag between `--grep` and its pattern silently returns nothing

`grep` is an `Option<String>` with no `allow_hyphen_values`, so clap refuses a
flag-looking value:

```
skim --grep --body 'new Box2d' mc8/     -> exit 2, message on stderr only
skim --grep -F 'a.b' mc8/               -> exit 2, message on stderr only
skim --grep -iE 'bounds' mc8/           -> exit 2, message on stderr only
skim --grep 'new Box2d' --body mc8/     -> works
```

The diagnostic is stderr-only and agents routinely write `2>/dev/null` (186 of
1,040 segments do), so the failure is invisible: exit 2, blank stdout, no hint.
The audit caught it four times, including in the tool's own author's demo run,
where three successive `--grep --body` sections printed nothing at all. A
session doing `skim --grep -iE "drop|bonus|heart|pickup" ... 2>/dev/null` got a
blank result and went straight to `rg`.

`-iE` is `grep` muscle memory; `-E` is not accepted at all even though ERE is
the default. Exit 2 is also overloaded (bad regex *and* bad CLI), which is why
the docs must say "2 on an invalid pattern".

### 2. `--map` silently drops most of what it scanned

```
skim --map --json mc8/core                     -> 8 entries
skim --map --json mc8/core --map-budget 200000 -> 222 entries
skim --map mc8/core                            -> 431 unmarked lines
skim --map mc8/gravewatch                      -> 16 of the 33 files on disk
```

`map::run` builds `selected` as a strict subset of `defs` and then drops the
total: `Map` carries only `text` and `entries`. The rendered text has the same
shape as a plain render, with no ranking, no score (`--json` has `score`; the
text drops it) and no omission footer.

This cost real turns. One lane spent four turns on "the skim output for
GravewatchField and ... is missing. That's strange - maybe skim truncated due to
size", because a 500-line map that stopped was indistinguishable from a
complete one. In another session `skim --map mc8/core > /tmp/coremembers.txt`
wrote 429 lines and a follow-up `grep "class Roster\|class Store"` found
nothing - not because the classes are absent, but because they were not in the
top 8. The agent read a complete-looking artifact as a complete answer.

### 3. A nonexistent path is silently "0 results"

```
skim --map mc8/nope/          -> exit 0, no output, no diagnostic
skim --grep zz mc8/nope/      -> exit 1, "0 match(es) in 0 file(s)" + a --body hint
skim --referrers X mc8/nope/  -> exit 1, "0 occurrence(s) ... in 0 file(s)"
skim mc8/nope.java            -> exit 2, "No such file or directory"
```

`discover::expand` pushes any non-directory, non-glob path through untouched, so
a typo becomes an empty file list rather than an error, and `--map` returns
success. The `N ... in M file(s)` summary counts files *with hits*, not files
scanned, so "0" cannot distinguish a typo from a genuine absence. A session
typing `skim --map mc8/computer` concluded "dir maybe doesn't exist" and moved
on. Worse, the miss-hint fires after a path that matched no files at all,
nudging `--body` when the problem is the path.

### 4. Misses are undiagnosed in the two cases agents hit most

The skeleton-miss hint covers "the token is in a body". Two adjacent misses get
nothing:

```
skim --grep 'entry' --body mc8/chat/DESIGN.md  -> 0 matches, no hint
skim --referrers 'panel()' mc8/                -> 0 occurrences, hint only mentions --text
skim --referrers 'Frame.pairs' mc8/            -> 0 occurrences, same
```

The guard-paren and qualified-name forms are the natural things to type when you
want call sites, and `--referrers` matches bare identifiers only. One session
spent three skim calls and two fallbacks on `HelpPage.Style` - a token that
lives in a body - before reaching `rg`.

### 5. `--body` plus `--referrers` produces a contradictory usage error

```
skim --referrers panel --body src/main/java/dev/mc8/core/
error: the following required arguments were not provided:
  --grep <PATTERN>
Usage: skim --grep <PATTERN> --referrers <NAME> --body <PATHS>...
```

`body` is declared `requires = "grep"` but nothing conflicts it with
`referrers`, so the usage line advertises the combination that just failed and
the missing argument named is the one the user did not ask for. A session ran
exactly this with `2>/dev/null`, saw nothing, and fell back to `rg`.

### 6. Output size is unbounded and truncation is invisible

There is no `--limit`/`--max-lines`, no per-file elision notice and no
"truncated" footer. Agents respond by piping through `head`/`sed`, which
truncates without a trace - see gap 2 for what that costs. A directory-wide
`skim --docs` on a large tree is ~13.5 k lines.

### 7. Small papercuts

- **`--map <single file>` is nearly a no-op vs a render**: 72 lines vs 70 for
  the same file, identical shape. Agents reached for it when they meant
  `--map --deps` (62 `--map` uses vs 4 `--deps`).
- **`--count`'s `decls` is undocumented.** A colour table with ~243 declarators
  reports `decls: 253`, so a data table ranks as a 45-lines-per-declaration
  monster. A separate audit hit a 253-vs-10 disagreement with an AST count and
  had to write a caveat around it.
- **`--check` on a non-doc is a silent pass**: `skim --check Foo.java` prints
  `0 docs, 0 violation(s)`, exit 0. That reads as green.
- **`--focus`** is documented in `--help` (as an alias of `--deps`) but not in
  the README; `--lang`, `--against`, `--band`, `--comment-min` and `--metric`
  were used zero times in 1,479 calls.

## Already handled

Three of these gaps were found *by* the audit window and closed inside it:
the skeleton-miss -> `--body` hint, the ERE/BRE hint (`\|` -> `|`), and the
`[definition]` flag on `--referrers`. The doc-side compensation also works:
after the recipe guide landed, consumer skim use kept climbing (58 segments in
the following week, 197 six weeks later).

## What this branch changes

1. `--map` reports its selection: a leading `# map: <selected> of <n>
   definitions, budget <tokens> tokens` line and, only when the budget cut the
   ranking short, a `# omitted: <n> - raise --map-budget to include them`
   footer. `Map` carries `selected` and `omitted`; `--json` keeps its bare-array
   shape, so it still cannot show the omission count — a deliberate 0.x
   compatibility choice.
2. `--grep`, `--referrers` and `--map` expand the scan set once, and a request
   that matches no file exits 2 with `skim: no files matched <paths>` instead of
   falling through to a zero-result report. The `--grep` summaries now name the
   files scanned as well as the files that matched.
3. The `--grep` argument wedge is closed at the parser boundary rather than
   documented. `-E`/`--extended-regexp` is accepted as a no-op so `-iE` parses,
   and a scoped pre-parse normalizer rewrites `--grep F1 F2 PATTERN` to
   `F1 F2 --grep PATTERN`, so `--grep -iE PATTERN`, `--grep -iF PATTERN` and
   `--grep --body PATTERN` all resolve. It fires only when the token after
   `--grep` is a known boolean match flag, and passes `--grep=PATTERN`,
   `--grep --`, a non-UTF-8 argv and every other mode through untouched.
4. `--body` on markdown and `--referrers` with call or member syntax get a
   one-line diagnosis instead of a bare zero.
5. `--body` conflicts with `--referrers` explicitly, with a message that names
   both arguments instead of asking for `--grep`.
6. `--check` says when it matched no documents, and `--count` / `--metric`
   document `decls` as per-declarator.

Two judgement calls are worth recording, because neither is forced by the data:

- **The wedge is fixed in the parser, not the docs.** An earlier draft added
  `-E` plus a help-text ordering note, which does not fix the shape the audit
  actually caught: clap 4 refuses a short-flag token as `--grep`'s value, so
  `--grep -iE PATTERN` exits 2 before the new flag is ever examined. Accepting
  the grep-like ordering costs ~20 lines of argv normalization and removes the
  failure mode instead of describing it.
- **`skim --grep -i TOKEN` reads TOKEN as the pattern.** With no further tokens
  there is no structural way to tell "TOKEN is a path and the pattern was
  forgotten" from "TOKEN is the pattern and the paths defaulted to the cwd".
  The pattern reading wins, because it is the one shape where the required
  argument was supplied and an optional one omitted, and because it matches
  `grep -i foo`. The cost is that the ambiguous shape now reports 0 matches on
  stdout at exit 1 instead of a usage error at exit 2; a heuristic that sniffs
  whether the token looks like a path or an existing directory was considered
  and rejected.

Along the way the `--map` footer's first draft was found to be wrong on the
branch: it advised `--map --json` as the way to see what was omitted, but
`--json` honours the same budget and returns the same selection.

## Cross-check against Claude Code sessions

The audit read Pi transcripts. The same question was put to the Claude Code
session store (`~/.claude/projects`, 184 transcripts, 2026-09-16 to
2026-10-08): **432 skim calls / 485 command segments in 87 transcripts**, all
consumer use against the same Java game repo. The mix is different and so is
the dominant failure:

| mode | segments |
| --- | --- |
| `--docs` | 194 |
| `--grep` | 123 (+11 `--body`) |
| `--map` | 53 |
| `--referrers` | 37 |
| render | 32 |
| `--check` | 19 |
| `--lines` | 15 |

- **The wedge is the Claude failure, and it is always `-i`.** 21 calls in 19
  transcripts wrote `skim --grep -i PATTERN PATH` - never `-iE` or `--body` -
  and every one exited 2 under 0.4. Because the call was nearly always one
  segment of a compound command (`skim ... | head; ls mc8/; ...`), the clap
  error scrolled past among other output, 8 of them under `2>/dev/null`, and at
  least 7 were followed directly by an `rg` of the same terms. 0.5.0's
  normalizer resolves the shape; `--grep -i PATTERN PATH`, with and without
  `--body`, was re-run against it to confirm.
- **`--docs` is the Claude workhorse (40% of segments), and it shared the typo
  hole.** The 0.5.0 guard covered `--grep`, `--referrers` and `--map` only, so
  `skim --docs mc8/nope/`, a plain render and `--count` still exited 0 with
  nothing printed. The guard now covers every mode but `--check`.
- **The guard counted files, not source.** `expand` returns every walked file,
  so a directory of PNGs (the repo's `golden/<app>/` directories are exactly
  that) passed the guard: `--map` printed `0 of 0 definitions` at exit 0 and
  `--grep` reported `(scanned 1)` for one image. The guard and the scanned
  count now count only files skim parses, and a directory with files but no
  source says `no source files matched` - distinct from a typo.
- **`--referrers` never got the scanned count.** Its summary still read
  `0 occurrence(s) ... in 0 file(s)`, the exact ambiguity gap 3 describes; four
  Claude calls hit it. It now reads `(scanned N; code identifiers only, ...)`
  and `--json` carries `scanned`.
- **`--body` counted the markdown it skips.** Over a lone design doc it said
  `(scanned 1)` and, on the next line, that markdown is not searched. The count
  is now source-only and the hint says the markdown files were *skipped*.

Test coverage added: `tests/map_elision.rs` (2), `tests/miss_hints.rs` (4) and
`tests/cli_diagnostics.rs` (19, end to end against the built binary), on top of
the 34 existing integration tests.

## Follow-up: the sessions after 0.5.0

The calls made after 0.5.0 shipped (2026-10-08 23:12 to 2026-10-09 07:55) were
read the same way: **46 consumer calls / 52 segments in 11 transcripts**, three
interactive sessions, one Pi session and seven Sonnet worker lanes.

- **The fixed shapes stayed fixed.** No call wrote a flag between `--grep` and
  its pattern, and no call exited 2. A Pi session read `# map: 8 of 306
  definitions` and went straight to `read`; a lane read `10 of 51` and moved on.
- **`--check` by hand was the new tax (10 segments).** A repository's gate can
  scope the check (mc8's runs `--check --quiet <platform dir>`), while a bare
  `--check` also reports docs outside it, so a lane saw a FAIL the gate never
  showed and ran `git stash; skim --check | grep FAIL; git stash pop` to learn
  whether it was its own. That is the repository's to fix; `-q` was used zero
  times, every lane piped through `grep FAIL` or `tail` instead.
- **A FAIL row could print inside its band.** `FAIL 0.30 ... band 0.30..0.80` was
  a 0.2987 rounded up, and three lanes then grepped `--help` for "budget",
  "doc=" and "nonblank", which explained none of them. A band FAIL now carries
  an indented `under: 0.2987 < 0.30 - add 43 doc line(s)` line, and `--check`'s
  help names the row's fields, the two relations and the aggregate.
- **`--docs` is still followed by a hunt for the body.** Almost every `--docs`
  call was followed by `grep -n NAME -A60` or a guessed `sed -n` range. `--lines`
  already composed with `--docs` - no session combined them - but the doc
  comments sat outside the number gutter, so the output read ragged. The
  comments and a type's closing brace now sit in a blank gutter, and the README
  names `--lines --docs` as the reading form.
- **Output size (gap 6) is still the open item.** 10 of 46 outputs were over 10
  kB, the largest a 28 kB `--docs` over two state classes and a 20 kB
  `--referrers` cut with `head`.
