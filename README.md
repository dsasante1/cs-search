# cs

Search your Claude Code conversation history across every session and project.

```
cs 'stripe webhook'              # all conversation text (regex, case-insensitive)
cs -p 'rate limit'               # only your own prompts (history.jsonl)
cs -i refresh.token              # fzf picker; Enter opens the full session
cs show 3f2a1b9c                 # one session as a readable transcript
cs sessions dashqard             # sessions newest-first, with their opening prompt
```

Run `cs --help` for the full flag list.

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
initial commit. The CLI is unchanged apart from an added `-j/--jobs`. Behaviour
differs in four places, each of them a fix:

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

87 tests, needing no network and no fixtures beyond what the suite creates and
cleans up itself:

- **Unit tests** sit inline in each module and cover the pure helpers —
  character-wise truncation and padding, jq-equivalent `tostring`, block
  flattening and its gating, argument parsing, and the prefilter's two guards.
- **Integration tests** (`tests/cli.rs`) build a synthetic corpus in a temp
  directory, point the binary at it with `CLAUDE_HOME`, and assert on real
  output. The fixture is hand-written, so the suite carries no personal data.

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
| `src/cli.rs` | argument parsing, usage text |
| `src/scan.rs` | parallel search engine and the prefilter |
| `src/record.rs` | transcript record model, block flattening |
| `src/sessions.rs` | `cs sessions` |
| `src/show.rs` | `cs show` |
| `src/prompts.rs` | `cs -p` |
| `src/interactive.rs` | `cs -i` (fzf) |
| `src/output.rs` | row formatting, colour, highlighting |
| `tests/cli.rs` | end-to-end tests against a synthetic corpus |
