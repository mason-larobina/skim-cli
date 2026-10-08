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
(skiming's own Rust source, `cargo`) and *consumer* sessions (scouting a Java
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

1. `--map` reports its selection: a header naming the mode and the counts, and a
   footer when definitions were omitted.
2. A path that matches no files is an error, and the hit summary reports how
   many files were scanned, so a typo and an absence stop looking alike.
3. `-E` is accepted as a no-op (ERE is the default), and the `--grep` value
   ordering is stated in the help text.
4. `--body` on markdown and `--referrers` with call or member syntax get a
   one-line diagnosis instead of a bare zero.
5. `--body` conflicts with `--referrers` explicitly, with a message that names
   the right command.
6. `--check` says when it matched no documents, and `--count`'s `decls` metric
   is documented as per-declarator.
