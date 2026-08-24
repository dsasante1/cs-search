# cs

Search your Claude Code conversation history across every session and project.

```
cs 'stripe webhook'              # opens the picker on a terminal; prints rows when piped
cs -F 'useState('                # match the pattern literally, not as a regex
cs -p 'rate limit'               # only your own prompts (history.jsonl)
cs -C 2 'ALTER TABLE'            # with two lines of surrounding context
cs -t 'ALTER TABLE'              # searching tool calls and their output too
cs -s last-week --thread 'flaky' # last week's matches, each with the turns around it
cs -b ui-overhaul 'divider'      # only what happened on that git branch
cs -s 7d -u yesterday 'deploy'   # a closed range; both ends inclusive
cs -q -p 'cloud sql'             # only the prompts that asked something
cs 'rate limit' --chrono         # one line per session, oldest first
cs history 'django-celery'       # when a topic started, and when it stopped
cs activity -s 30d               # where the month went, by day and project
cs show 3f2a1b9c                 # one session as a readable transcript
cs show 3f2a1b9c -r user         # only the half of it you typed
cs sessions dashqard             # sessions newest-first, by title
cs files 'settings/base.py'      # which sessions touched a file, and when
cs handoff 3f2a1b9c              # where that session left off
cs related 3f2a1b9c              # other sessions about the same thing
cs stats -P dashqard             # models, tokens and cache use
cs stats 3f2a1b9c                # or for one session alone
cs export 3f2a1b9c --format md   # one session as a document
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

Flags are read on either side of the pattern. The shell version stopped at the
first bare word, so `cs database --json` searched for `database` and dropped the
flag without saying so — including in four of the examples shipped with this
program. Two bare words are now an error rather than the first one and a
silence:

```console
$ cs stripe webhook
unexpected argument 'webhook' after the pattern 'stripe' — quote them if they are one pattern
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

`--chrono` is a third rendering, for the question grouping does not answer —
not "where was this mentioned" but "how did it develop". One line per session,
oldest first, quoting the line that first matched in it:

```
2026-07-11 10:17 api      3f2a1b9c   4  should we cache availability at all?
2026-07-12 09:02 api      7c1d0e2a   9  going with redis for the read-heavy path
2026-08-02 14:31 api      a4b1c9d3  12  the stale-cache race is in the lease path
2026-08-17 11:40 api      f091ba22   6  invalidation now happens after the commit
```

Nothing there is summarised. Each line is one somebody wrote, picked by being
the first hit in that session; reading the progression is yours to do. On a
terminal the snippet is cut to fit the window, because a row that wraps three
times is no longer one line per session.

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

`-q/--questions` keeps only the lines that ask something, which is most of what
`-p` is for — what you *asked* is a different set from what you instructed:

```sh
cs -q -p 'cloud sql'             # the questions you typed
cs -q 'rate limit'               # and the ones either speaker asked
```

A question is recognised by punctuation: a `?` that ends a clause and follows a
word. There is no list of interrogative openers, because "can you run the
tests" is a request rather than a question and no wordlist tells the two apart
without being tuned forever. So `?id=1` in a URL, a `colou?r` quantifier and a
`a ?? b` are all excluded, and a question typed without its mark is missed. That
is the direction worth being wrong in.

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

### `cs history` — when a topic started, and when it stopped

A search hands back every line and leaves the counting to you. The question
underneath is usually smaller: when did this first come up, am I still on it,
and which projects did it bleed into.

```console
$ cs history 'django-celery'
'django-celery'

  first   2026-05-12 14:03  aaaa1111  104 days ago
  last    2026-08-19 09:41  bbbb2222  5 days ago

86 matches · 14 sessions · 3 projects

PROJECTS
        52  api
        28  worker
         6  cs
```

It is the same search, counted rather than listed, so it can never report
something a search would not show you — and it takes every flag one takes, `-P`
and `-s` and `-t` and `-p` included. "First" and "last" are lines somebody
actually wrote, each naming the session to open. `--sessions` adds the
chronology underneath, rendered by the same code `--chrono` uses.

### `cs activity` — where the month went

`stats` totals the corpus; this cuts the same records by day, which is the only
axis that answers "what happened last month" rather than "what is in here".

```
DAY         sessions  messages
2026-08-24         4       721  ███▌
2026-08-23         2     1,211  ██████
2026-08-22         5     2,671  ██████████████
2026-08-21         9     3,788  ████████████████████

15 active days · 86 sessions · 41,301 messages

PROJECTS
    16,102  unicare_hostel_management
    14,105  dashqard-customer-backend-api
```

Days nothing happened on are absent rather than drawn as zero — a year would
otherwise be three hundred empty rows, and the gaps between the dates say the
same thing. The bars are drawn only for a terminal; piped, the columns arrive
without them, and `--json` gives one object per day.

### `cs handoff` — where a session left off

Coming back to work after a break, the questions are always the same: what was
this, how long did it run, which files did it touch, what was said last.

```console
$ cs handoff 4258e94f
SESSION  4258e94f-90c2-4f11-b3a7-1d0e2a7c4b58
  project  dashqard-customer-backend-api  /home/u/dashqard
  branch   feat/cache-invalidation
  when     2026-08-02 10:17 → 2026-08-02 12:59  (2h 42m)
  turns    38 yours · 61 assistant
  tokens   1.8K in · 412K out · 7.3M cached  (96.4% from cache)

FILES
    12  src/cache/availability.ts
     4  src/services/lease.ts

LAST TURNS
── CC  2026-08-02 12:58 ───────────────────────────────────────────
Invalidation now runs after the transaction commits, not inside it.
```

All four are recorded, so all four are read. What is deliberately absent:
"open threads", "the decision", "the next step". Those cannot be had from a
transcript without summarising it, and a heuristic that guessed them would be
wrong quietly — the one failure this tool tries hardest not to have. The closing
turns are printed verbatim instead, with the tool calls left out: a tail made of
JSON payloads says nothing about where the work got to. `--prices` prices the
session through the same arithmetic `stats` uses.

### `cs related` — other sessions about the same thing

Work on one problem scatters across sessions, and nothing joins them up: the
session that hit the bug and the session that fixed it share a subject but not a
word you would think to search for.

```console
$ cs related 9f42c8f8
related to 9f42c8f8 · Project description for feature ideas

weight  last        project   session   title
  9.10  2026-08-24  cs        d943d496  Feature recommendations
                                        ↳ --branch, --prices, --thread, alt-x  +395 more
  6.35  2026-08-18  move      ad653846  Search .claude folder by keyword
                                        ↳ embedding, ripgrep, subagent, fzf  +175 more
```

The measure is ordinary and old: a term is worth something in proportion to how
*rare* it is across the corpus, and two sessions are related in proportion to
the rare terms they have in common. That has one property worth the whole
design — **it needs no list of stopwords**. "the" appears in every session, so
`ln(sessions / sessions)` is zero and it counts for nothing on its own. Nothing
has to be tuned, and there is no list to go stale. The total is divided by the
square root of the session's length, or the ranking would measure size as much
as subject.

It is not an understanding of either session; it is a claim about vocabulary.
So the words that earned each result are printed beside it, and a result whose
words are obviously the wrong ones can be dismissed without opening it. The
weight column is named because the number needs it: it orders the list and means
nothing on its own.

Only conversation text is read — tool calls and their output are full of paths
and file contents that would drown the subject in incidentals. It is also the
one command here with no prefilter to hide behind: which terms are rare is not
knowable until every record has been read, so every record is read. On the
corpus below that is 0.39s.

### `cs stats` — what the corpus is made of

Every assistant record carries a model and a usage block, and nothing here had
ever read them:

```
181 sessions · 78,379 messages · 45 projects
2026-07-10 → 2026-08-24

MODEL           replies    input   output    cached
claude-opus-5    46,608     288K    40.9M      9.5B

TOKENS
  input            389K
  cache read      10.1B
  from cache      97.3%
```

Cost is not built in. Prices change, a hardcoded table would go stale silently,
and a number that is quietly wrong is worse than no number — so `--prices
<file>` takes a table of dollars per million tokens from you, and a model
missing from it is named rather than billed at zero.

A session id narrows all of it to one session — `cs stats 3f2a1b9c` — which is
where "what did this one cost me" is answered. It is the same walk and the same
arithmetic, pointed at the single file the transcripts are named by, so a
session's cost and the corpus's can never be computed two different ways.

### `cs export` — a session as a document

`show` renders for a terminal: ANSI, a rule sized to the window, a pager. None
of that survives being redirected into a file or attached to an issue.
`cs export <id> --format md|html|json` is the same transcript with the terminal
taken out of it — the HTML is self-contained, readable in either colour scheme,
and escaped, which matters because a transcript is full of markup. Both read the
file through one function, so they can never disagree about what a session
contains.

### `cs completions`

```sh
eval "$(cs completions bash)"    # or zsh; fish wants `cs completions fish | source`
```

Session ids are eight hex characters, so `cs show` was really `cs sessions |
grep` followed by a copy and a paste. Ids and project names are completed by
shelling out to `cs` itself rather than from a cache — the corpus changes every
time you use Claude Code, and `sessions` answers in 0.05s.

## Recipes

**Where did we land on something.** A search gives you the line; `--thread`
gives you the exchange it came out of, which is usually the half you wanted:

```sh
cs --thread 'rate limiting'
```

**How a decision developed.** `history` bounds it, `--chrono` walks it, and the
session at the end of the list is where it was settled:

```console
$ cs history 'rate limiting'
'rate limiting'

  first   2026-06-18 09:12  3f2a1b9c  67 days ago
  last    2026-08-19 14:02  f091ba22  5 days ago

47 matches · 9 sessions · 3 projects

$ cs 'rate limiting' --chrono
```

**What else was about this.** Having found one session, `related` finds the
others that share its vocabulary, and says which words those were:

```sh
cs related 3f2a1b9c
```

**Picking work back up.** `handoff` is the first thing to run on a Monday:

```sh
cs sessions dashqard | head -3     # what was I in the middle of
cs handoff 4258e94f                # and where did it get to
```

**What touched a file, and what was said about it.** `files` answers the first
half and hands you the session id for the second:

```console
$ cs files 'settings/base.py'
   9  2026-08-23 13:09  anasset-api           d8ec79b4  config/settings/base.py
  53  2026-08-20 11:58  unicare_ho…anagement  ca98bc25  config/settings/base.py

$ cs show ca98bc25
```

**Where the month went.** `activity` cuts the corpus by day; `stats` totals it.
Both take the same filters:

```sh
cs activity -s 30d
cs activity -s 30d -P dashqard
```

**A day, or a week, in aggregate.** Both ends of the range are inclusive, so a
date repeated is that one day:

```console
$ cs stats -s 2026-08-20 -u 2026-08-20
8 sessions · 2,684 messages · 4 projects
2026-08-20 → 2026-08-20
998 yours · 1,686 assistant
```

**What one session cost.** The same command, given an id instead of a filter:

```console
$ cs stats 4258e94f --prices prices.json
1 session · 99 messages · 1 project
```

**What a month cost.** The price table is yours, in dollars per million tokens,
rather than one baked into the binary and quietly going out of date:

```console
$ cat prices.json
{"claude-opus-5": {"input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25}}

$ cs stats -s last-month --prices prices.json
COST
  estimated    $7406.22
  not in the price table: <synthetic>, claude-opus-4-8
```

Anything the table does not price is named rather than counted as free, so the
total is never short without saying so.

**What you actually asked, as opposed to told.** `-q` keeps the lines that end a
clause with a question mark, so `-p` stops being half instructions:

```sh
cs -q -p 'cloud sql'
```

**Every session that discussed a topic.** `--json` is the interface to build on;
the flat columns are stable, but the field names are the thing that will not
move:

```sh
cs --json 'prefilter' | jq -r .session | sort -u
cs --json 'migration' | jq -r .project | sort | uniq -c | sort -rn
```

**A session someone else has to read.** `show` renders for your terminal;
`export` renders for anywhere else:

```sh
cs export 3f2a1b9c --format md   > session.md
cs export 3f2a1b9c --format html > session.html
```

**A search that came back with too much.** Rather than quitting and retyping,
narrow it in the picker: `alt-p` scopes to the project under the cursor, `alt-r`
cycles the speaker, `alt-x` adds the surrounding turns. Typing re-runs the
search rather than filtering what is already on screen.

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

359 tests, needing no network and no fixtures beyond what the suite creates and
cleans up itself:

- **Unit tests** sit inline in each module and cover the pure helpers —
  character-wise truncation and padding, jq-equivalent `tostring`, block
  flattening and its gating, argument parsing, the prefilter's two guards,
  middle-elision, session grouping, the picker's state transitions, the fzf
  command line the picker is launched with, the transcript divider's geometry in
  both colour and plain form, which filters an empty result probes, which
  prompts a date cutoff keeps, date specs resolved against a fixed "today",
  which title in a file wins, how touches fold into files, the token arithmetic
  behind `stats`, what counts as a question and what is only punctuation, how a
  session's span reads off its two ends, that a day's messages and its sessions
  are counted separately, that every example in this README and in the help page
  uses a flag the help page documents, and the count-gutter alignment that
  survives having escape sequences in the line.
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
the newer ones have something to read. A fourth sits beside it in another
project, saying what the third said in different words: two sessions have to
share a vocabulary before `related` has anything to find, and its wording is
chosen to overlap with that session and with nothing else in the corpus.

`related`'s ranking is tested on the property rather than on an outcome — that a
term every session uses carries no weight, that a rare term outranks a common
one however many of the common ones are shared, and that a long session is not
related to everything merely by being long. Those hold whatever the corpus is,
which a fixture asserting "session C comes first" would not.

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
| `src/completions.rs` | `cs completions`: bash, zsh and fish scripts |
| `src/export.rs` | `cs export`: markdown, self-contained HTML, JSONL |
| `src/files.rs` | `cs files`: paths that were acted on, folded per file |
| `src/history.rs` | `cs history`: a result set counted rather than listed |
| `src/activity.rs` | `cs activity`: the corpus cut by day |
| `src/handoff.rs` | `cs handoff`: one session's shape, and how it ended |
| `src/related.rs` | `cs related`: rare shared vocabulary, and its weights |
| `src/stats.rs` | `cs stats`: models, tokens, cache, optional priced cost |
| `src/resume.rs` | `cs resume` |
| `src/prompts.rs` | `cs -p` |
| `src/interactive.rs` | the picker: the fzf command line and what comes back |
| `src/picker.rs` | filter state shared with fzf's key bindings |
| `src/output.rs` | flat, grouped and JSON rendering; colour and highlighting |
| `tests/cli.rs` | end-to-end tests against a synthetic corpus |
