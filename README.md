# cs

Search your Claude Code conversation history across every session and project.

```
cs 'stripe webhook'              # opens the picker on a terminal; prints rows when piped
cs -F 'useState('                # match the pattern literally, not as a regex
cs -p 'rate limit'               # only your own prompts (history.jsonl)
cs -C 2 'ALTER TABLE'            # with two lines of surrounding context
cs -s last-week --thread 'flaky' # last week's matches, each with the turns around it
cs -b ui-overhaul 'divider'      # only what happened on that git branch
cs show 3f2a1b9c                 # one session as a readable transcript
cs show 3f2a1b9c -r user         # only the half of it you typed
cs sessions dashqard             # sessions newest-first, by title
cs files 'settings/base.py'      # which sessions touched a file, and when
cs projects                      # what -P can be given
cs resume 3f2a1b9c               # reopen that session in Claude Code
```

Run `cs --help` for the full flag list.

## The interface

A search on a terminal opens the picker. Piped, it prints the same flat rows it
always has, so anything built on that keeps working:

```sh
cs database | wc -l              # unchanged: one line per match
cs database --plain              # results instead of the picker
cs database --json               # one JSON object per match, per line
```

Inside the picker, **typing runs the search again** rather than filtering the
list you arrived with — fzf's own matching is switched off and every keystroke
re-runs `cs`, so a first pattern that was too narrow is not a dead end. Filters
are keys rather than flags you have to quit and retype:

| | |
|---|---|
| `enter` | open the session, at the match, in a pager |
| `alt-enter` | resume the session in Claude Code |
| `alt-t` / `alt-h` / `alt-s` | tool blocks · thinking blocks · subagent messages |
| `alt-r` | cycle role: any → user → assistant |
| `alt-x` | thread context: the turns either side of the match |
| `alt-p` | filter to the project under the cursor, or clear it |
| `alt-c` | clear every filter |

The header names only the filters that are actually on, so an unnarrowed search
says nothing and the line's presence is itself the signal. The bindings use `transform-header` and
`become`, so they want a reasonably recent fzf — 0.38 or newer, and 0.74 is what
this was tested against. Without fzf at all, results are printed instead.

### Reading results

A broad search spans more sessions than fits on a screen — `database` returns
507 matches across 99 sessions on the corpus benchmarked below — so terminal
output groups matches under the session they came from, folding all but the
first few:

```
dashqard-customer-backend-api 4258e94f  2026-08-02 10:17  38 matches
  08-02 10:17 asst **7. Import-time side effects amplify the singleton coupling.**…
  08-07 12:21 asst **4. `eganow_checkout` and `corporate-vendor` got DI** — both had…
  …
  … 33 more · cs show 4258e94f
```

`--no-group` gives one line per match instead. A count of matches, sessions and
projects goes to stderr when a person is there to read it.

Colour is spent where it carries something: the project, and the match. The role
column is dim rather than magenta — beside a highlighted hit, a coloured role was
just competing with it.

### Reading a session

`cs show` divides the transcript at every handover with a rule the width of the
terminal, rather than the old `=== CC 12:00 ===`, which read as decoration and
let the two speakers run together down the page:

```
── YOU 2026-08-03 16:31 ──────────────────────────────────────────────────
run it against staging first

── CC  2026-08-03 16:31 ──────────────────────────────────────────────────
Now I'll implement. Starting with the migrations.
```

`-r user` or `-r assistant` reads one side alone. `-r user` means *what you
typed*: tool results arrive as user-type records, and a filtered view half full
of machine output filed under your name is not what the flag is for, so those
are dropped.

### When nothing matches

Five filters can each independently empty a search, and `no matches` named none
of them. At a terminal each active filter is now lifted in turn and the search
re-run, so the report says which one cost what:

```
$ cs -P callout 'ALTER TABLE'
no matches for 'ALTER TABLE'

  14 matches without  -P callout

  -t  also searches tool calls and results
```

That is up to five extra scans, paid only on an empty result and only for a
person at a terminal — piped, the answer is the same single line at the same
speed as before.

### Patterns

The pattern is a regex, which is a trap for anything that merely looks like one:
`cs 'C++'` reads as "a `c`, repeated", and quietly returns tens of thousands of
rows. Two things guard against it — `-F` matches literally, and a pattern that
returns a very large result set while containing metacharacters says so on
stderr. A pattern that is not valid regex at all (`useState(`) is retried as a
literal rather than rejected, and the substitution is reported.

## Narrowing

`-P` takes a substring of the project directory. Two filters sit beside it.

`-b/--branch` matches the git branch the session was on, recorded per line
rather than per session, so a session that switched branches answers for each
half separately. The branch also rides beside the project in grouped output:

```
▸ cs@ui-overhaul 623fcafd  2026-08-20 01:01  4 matches
```

Flat output is deliberately unchanged — those columns are what scripts parse —
so `--json` is where a program reads the branch.

`-s/--since` and `-u/--until` bound a range from either end, and neither needs a
date you have to look up:

```sh
cs -s 7d 'timeout'               # the last week
cs -s yesterday -t 'migrate'
cs -s 2026-08-01 -u 2026-08-01 'ALTER TABLE'   # one day, both ends inclusive
```

`today`, `yesterday`, `last-week`, `last-month`, `Nd`, `Nw`, `Nm`, `Ny` and a
plain `YYYY-MM-DD` are all accepted; months step by calendar, so `1m` from the
31st lands on the last day of the previous month rather than somewhere in the
middle of it. A spec that names no real day is rejected at parse time rather
than quietly matching nothing — `-u 2026-02-30` is an error, not an empty result.

### Context that crosses records

`-C/-A/-B` widen a match within the message it sits in, which for prose is
usually more of the same paragraph. `--thread` instead shows the turns either
side of it — the prompt that produced the reply, the reply the prompt drew:

```
2026-08-24 14:19 cs  asst d943d496  export is that renderer minus the pager
                                    ↑ you   can we get a session out as markdown?
                                    ↓ you   create a branch and implement it
```

Those are different records, reachable only through `parentUuid`, so this reads
the chain rather than the block: every conversation record is decoded, prefilter
or not, because which turns are neighbours is not knowable until the chain is
built. On a 302 MB corpus that is 0.51s against 0.21s, which is why it is a flag
and not the default. `alt-x` toggles it inside the picker.

## Beyond search

### `cs files` — the axis that is not text

```
   9  2026-08-23 13:09  anasset-api    d8ec79b4  config/settings/base.py
  53  2026-08-20 11:58  unicare_ho…t   ca98bc25  config/settings/base.py
```

Touches, when it was last one, the session to open, and the path relative to the
project it belongs to. The filename is in the transcript either way, but only
inside tool blocks, where `-t` finds it flattened into a wall of JSON alongside
whole file contents — technically a hit, practically unreadable. Reading the
block structurally instead makes "when did I last touch this, and in which
session" answerable. Paths are read by key (`file_path`, `notebook_path`) rather
than by tool name, so a tool added later is seen without a change here. It takes
the same `-P`, `-b`, `-s`, `-u` and `-F` a search does.

## Install

```sh
cargo install --path .
```

The binary reads `$CLAUDE_HOME` (default `~/.claude`), so you can point it at a
copy of the history for testing:

```sh
CLAUDE_HOME=/tmp/snapshot cs 'database'
```

`cs -i` shells out to `fzf`; nothing else is required at runtime.

## How it works

Claude Code writes one JSONL file per session under `~/.claude/projects/`, plus a
small `~/.claude/history.jsonl` holding just your typed prompts. A search walks
the transcripts across every core, and for each line decides whether to parse it:

1. **Prefilter.** The pattern is matched against the *raw* JSON bytes first, via
   the same literal-scanning machinery ripgrep uses. Rejecting a line here costs
   a memchr pass instead of a full JSON parse, and most lines are rejected.
2. **Decode.** Survivors are parsed into `serde_json::Value` — deliberately the
   dynamic type rather than a fixed struct, because the transcript format is an
   internal Claude Code detail that drifts between versions. Unknown block types
   are skipped, missing fields default to empty.
3. **Filter and emit.** Role, project, date, sidechain and meta filters apply,
   then each text block is split into lines and matched individually.

### On the prefilter being sound

Matching raw JSON is only valid where the encoded and decoded text agree, so two
cases are handled explicitly:

- **Escapes.** A line carrying any escape other than `\n` is decoded rather than
  rejected, since a match could straddle it. `\n` is exempt because decoded text
  is searched line by line anyway, so a pattern is never allowed to span one.
- **Position assertions.** `^`, `$`, `\b` and friends anchor to the haystack, and
  raw JSON is a different haystack — `^SELECT` can never match a line starting
  with `{`. Patterns containing them skip the prefilter and decode everything.

The `CS_NO_PREFILTER=1` environment variable forces the slow, unconditionally
correct path; comparing against it is how the prefilter is checked for dropped
results.

## Relationship to the original

This replaces a bash + `ripgrep` + `jq` pipeline, preserved in this repository's
initial commit. Every flag the shell version took still means what it meant, and
piped output is still one line per match in the same columns; the picker,
grouping, `-F`, `-C`, `--json` and the `projects` and `resume` subcommands are
additions on top. Two things inside those columns did change: the assistant is
labelled `asst` rather than `assi`, which was "assistant" cut to four characters
and read as a typo, and the project column sizes itself to the widest name in
the result set instead of truncating everything at 16. Splitting on whitespace
is unaffected by both; `--json` is the interface to build on if column positions
matter. Behaviour differs in four places besides, each of them a fix:

| | shell version | here |
|---|---|---|
| Anchored patterns (`^SELECT`, `ERROR$`) | silently return nothing — `rg` anchored them to the raw JSON line | matched against decoded text |
| Literal backslashes in output | doubled, because jq's `@tsv` escapes them | printed as-is |
| `-l/--files` | listed files whose raw JSON matched, ignoring `-t` | lists files that actually produced rows |
| Result ordering | depended on the locale `sort` ran under | deterministic |

Performance on a 338 MB / 254-session corpus, best of three (`-t` is a single run —
the shell version is too slow to repeat):

| | shell version | here |
|---|---|---|
| common term (`database`) | 3.22s | 0.16s |
| rare term (`ALTER TABLE`) | 1.80s | 0.24s |
| with `-t` | 40.8s | 0.28s |
| `sessions` | 4.79s | 0.03s |
| no match anywhere | 0.09s | 0.28s |

The last row is the one regression, and it is the cost of soundness: on a pattern
that matches nothing, the shell version stops after ripgrep's scan, while this one
still decodes every line carrying an escape rather than risk dropping a match.

The old pipeline spent ~98% of its time in `jq`: `rg` chose which *files* to look
at, but every surviving file was then re-read and fully parsed. Here the
prefilter and the parser are in the same process, so rejecting a line actually
avoids the work.

## Tests

```sh
cargo test
```

261 tests, needing no network and no fixtures beyond what the suite creates and
cleans up itself:

- **Unit tests** sit inline in each module and cover the pure helpers —
  character-wise truncation and padding, jq-equivalent `tostring`, block
  flattening and its gating, argument parsing, the prefilter's two guards,
  middle-elision, session grouping, the picker's state transitions, the fzf
  command line the picker is launched with, the transcript divider's geometry in
  both colour and plain form, which filters an empty result probes, which
  prompts a date cutoff keeps, date specs resolved against a fixed "today",
  which title in a file wins, how touches fold into files, and the count-gutter
  alignment that survives having escape sequences in the line.
- **Integration tests** (`tests/cli.rs`) build a synthetic corpus in a temp
  directory, point the binary at it with `CLAUDE_HOME`, and assert on real
  output. The fixture is hand-written, so the suite carries no personal data.

The picker itself is covered in two halves rather than driven end-to-end: the
generated fzf arguments are asserted on directly, and the commands its key
bindings invoke (`__rows`, `__toggle`, `__header`) are exercised as ordinary
subcommands, including the toggle-then-reload loop a keypress performs.

The synthetic corpus carries a third session holding what the first two
predate — a git branch, a generated title, tool calls naming files, and a usage
block — so the older tests keep counting what they were written to count while
the newer ones have something to read.

The prefilter's invariant — that it may waste work but must never drop a line
whose decoded text matches — is checked twice over: as a property test across
texts containing quotes, backslashes, tabs, newlines and unicode, and end-to-end
by diffing every result against `CS_NO_PREFILTER=1`.

Each behaviour fixed relative to the shell version has a named regression test.
The guards behind them were confirmed to have teeth by mutation: separately
disabling the anchor check, the escape fallback and the meta-record filter each
makes the suite fail.

## Layout

| | |
|---|---|
| `src/main.rs` | CLI dispatch |
| `src/dates.rs` | `--since` / `--until` specs, absolute and relative |
| `src/cli.rs` | argument parsing, usage text |
| `src/scan.rs` | parallel search engine and the prefilter |
| `src/record.rs` | transcript record model, block flattening |
| `src/sessions.rs` | `cs sessions` |
| `src/projects.rs` | `cs projects` |
| `src/show.rs` | `cs show`: speaker dividers, role filter, jump-to-match, pager |
| `src/files.rs` | `cs files`: paths that were acted on, folded per file |
| `src/resume.rs` | `cs resume` |
| `src/prompts.rs` | `cs -p` |
| `src/interactive.rs` | the picker: the fzf command line and what comes back |
| `src/picker.rs` | filter state shared with fzf's key bindings |
| `src/output.rs` | flat, grouped and JSON rendering; colour and highlighting |
| `tests/cli.rs` | end-to-end tests against a synthetic corpus |
