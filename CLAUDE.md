# CLAUDE.md

`cs` — a Rust CLI that searches Claude Code conversation history (`~/.claude/projects/**/*.jsonl`
plus `~/.claude/history.jsonl`). Single binary, no runtime deps except `fzf` for the picker.

**Read `README.md` first for user-facing behaviour.** It is long and current: every subcommand,
flag, output rendering and design rationale is there. This file covers only what the README does
not: how to work on the code.

## Commands

```sh
cargo test                   # 213 unit + 158 integration, ~0.5s, no network or fixtures
cargo clippy --all-targets   # kept clean
cargo fmt                    # rustfmt.toml sets use_small_heuristics = "Max"; keep it clean
cargo test <substring>       # test names are sentences, so this reads well
CLAUDE_HOME=/tmp/snapshot cargo run -- 'database'   # run against a corpus that isn't yours
```

`CLAUDE_HOME` (default `~/.claude`) is how everything is pointed at a test corpus — never
hardcode a home path. `CS_NO_PREFILTER=1` forces the slow, unconditionally-correct scan path.

## Architecture

`main.rs` dispatches: subcommands are matched by `args[0]` **before** any flag parsing, then two
separate parsers take over.

- `cli::parse` — the grammar for *searches* (a pattern is required). Flags are read on either
  side of the pattern; two bare words are an error, not a silent drop.
- `main::parse_corpus(args, Allow{..})` — the shared grammar for the pattern-less subcommands
  (`stats`, `activity`, `handoff`, `related`, `export`, `show`, `files`, `history`). One parser
  for all of them so `-P`/`-s` cannot come to mean different things in different subcommands.
  `Allow` says which of session-id / `--prices` / `--limit` / corpus filters that command takes;
  a refused filter is reported through `main::rejected()` so the wording is identical everywhere.

Search pipeline (`scan.rs`): walk transcripts → per-line **prefilter against raw JSON bytes** →
`serde_json::from_slice` only for survivors → `record.rs` filters and block flattening → `Row`s →
`output.rs` renders flat / grouped / chrono / JSON. Work is distributed over a
`std::thread::scope` pool pulling from a `Mutex<Vec<PathBuf>>` queue — no rayon.

`record.rs` reads through `serde_json::Value` with defaults, deliberately not a typed struct: the
transcript format is an internal Claude Code detail that drifts between versions. Unknown block
types are skipped, missing fields become empty strings. Keep it that way.

## Invariants worth not breaking

- **The prefilter may waste work but must never drop a match.** Two guards: a line carrying any
  escape other than `\n` is decoded rather than rejected, and a pattern containing a position
  assertion (`^ $ \b \B \A \z \Z \< \>`) disables the prefilter entirely. Tested as a property
  and end-to-end by diffing against `CS_NO_PREFILTER=1` (`Corpus::run_full_decode`).
- **A session belongs to the project it opened in.** `projects::label()` resolves it once per
  transcript from the first `cwd`; labelling per record invented projects out of subdirectories.
  `-P` still matches each record's own `cwd`.
- **Sort on `Row::precise`, not the displayed `ts`** — `ts` stops at the minute, so ties fell
  through to the role field and printed answers above their questions.
- **`--json` is the stable interface**; flat columns are stable too but positional. Adding a
  field to `Row` for internal use must not change `to_json` (that is why `precise` is absent).
- **Colour, grouping, the picker, the summary line and the empty-result probes are terminal-only.**
  Piped output is byte-identical to what scripts already parse, and pays none of that cost.
- Exit codes: `0` success, `1` nothing found (with a message on stderr — every listing says so
  rather than exiting silently), `2` usage error, `127` `-i` without fzf.
- `exit()` skips destructors, so every `BufWriter` is flushed by hand before it.

## Docs are tested

Unit tests in `cli.rs` assert that `USAGE` documents every flag and subcommand, that every flag
used in a `USAGE` example *or in a README code block* is documented above it, and that no `USAGE`
line exceeds 80 columns. **Adding a flag means editing `USAGE`, and usually README, in the same
change** — `cargo test` fails otherwise.

## Tests

Unit tests sit inline (`#[cfg(test)] mod tests`) next to the pure helpers they cover. Integration
tests (`tests/cli.rs`) build a hand-written synthetic corpus in a temp dir, run the real binary
against it via `CLAUDE_HOME`, and assert on its output; the fixture carries no personal data, and
`Corpus` deletes itself on drop. New behaviour that needs corpus data goes into an existing
session in `Corpus::build` where possible — sessions A–E each exist to cover a specific era of
the format (branches, titles, tool paths, usage blocks, shared vocabulary for `related`).

Prefer testing the *property* over the outcome where one exists: `related`'s ranking is asserted
via "a term every session uses carries no weight", not "session C comes first".

The picker is covered in two halves rather than driven end-to-end — the generated fzf argument
list is asserted directly, and the internal callbacks its key bindings invoke (`__rows`,
`__toggle`, `__header`, spelled with underscores because they are not part of the CLI) are
exercised as ordinary subcommands.

## Conventions

- Dependencies are minimal and deliberate (`lexopt` over clap, hand-rolled threads over rayon).
  Don't add one without a reason that survives the README's performance table.
- Module-level `//!` docs explain *why the design is what it is*, usually naming the failure that
  forced it. Function comments do the same. Match that register: explain the reason, not the
  mechanism the code already shows.
- No heuristics that guess quietly. Where a number or a summary would have to be invented
  (`handoff`'s "next step", a baked-in price table), the tool prints what was actually recorded
  and names what it could not answer.
- Commit messages: `type(scope): lowercase subject in prose`, then a body explaining the fault,
  the fix, and what deliberately did not change. Work happens on branches merged via PR.
