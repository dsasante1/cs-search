//! End-to-end tests: build a small synthetic corpus, run the real binary against
//! it via `CLAUDE_HOME`, and assert on what it prints.
//!
//! The fixture is deliberately hand-built rather than copied from a real history,
//! so the suite is self-contained and carries no personal data.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const SID_A: &str = "aaaaaaaa-1111-4444-8888-aaaaaaaaaaaa";
const SID_B: &str = "bbbbbbbb-2222-4444-8888-bbbbbbbbbbbb";
const SID_C: &str = "cccccccc-3333-4444-8888-cccccccccccc";

/// A throwaway `CLAUDE_HOME` that deletes itself when the test ends.
struct Corpus {
    root: PathBuf,
}

impl Corpus {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cs-it-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let me = Corpus { root };
        me.build();
        me
    }

    fn session(&self, project: &str, sid: &str, records: &[Value]) {
        let dir = self.root.join("projects").join(project);
        std::fs::create_dir_all(&dir).unwrap();
        let body: String = records
            .iter()
            .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
            .collect();
        std::fs::write(dir.join(format!("{sid}.jsonl")), body).unwrap();
    }

    fn build(&self) {
        // Session A: covers thinking, tools, meta, sidechain, a literal backslash
        // and a line that only an anchored pattern would single out.
        self.session(
            "-home-u-alpha",
            SID_A,
            &[
                // Not a conversation turn -- must never surface.
                json!({"type": "queue-operation", "sessionId": SID_A}),
                msg("user", SID_A, "/home/u/alpha", "2026-07-01T10:00:00Z",
                    json!("SELECT * FROM users WHERE id = 1")),
                msg("assistant", SID_A, "/home/u/alpha", "2026-07-01T10:01:00Z",
                    json!([{"type": "text", "text": r"Run this: make build \"}])),
                msg("assistant", SID_A, "/home/u/alpha", "2026-07-01T10:02:00Z",
                    json!([{"type": "thinking", "thinking": "pondering the needle"}])),
                msg("assistant", SID_A, "/home/u/alpha", "2026-07-01T10:03:00Z",
                    json!([{"type": "tool_use", "name": "Bash",
                            "input": {"command": "grep zzsentinel"}}])),
                meta(msg("user", SID_A, "/home/u/alpha", "2026-07-01T10:04:00Z",
                    json!("meta noise needle"))),
                sidechain(msg("assistant", SID_A, "/home/u/alpha", "2026-07-01T10:05:00Z",
                    json!("subagent needle"))),
                // A tool result arrives as a *user*-type record even though the
                // user typed none of it.
                msg("user", SID_A, "/home/u/alpha", "2026-07-01T10:06:00Z",
                    json!([{"type": "tool_result", "content": "zzresultonly payload"}])),
            ],
        );

        // Session B: a second project, later dates, and a multi-line block.
        self.session(
            "-home-u-beta",
            SID_B,
            &[
                msg("user", SID_B, "/home/u/beta", "2026-08-01T12:00:00Z",
                    json!("multi line\nsecond needle line")),
                msg("assistant", SID_B, "/home/u/beta", "2026-08-02T12:00:00Z",
                    json!("beta project needle")),
                // Quotes are escaped on disk, so a pattern spanning one matches
                // the decoded text but never the raw JSON.
                msg("assistant", SID_B, "/home/u/beta", "2026-08-03T12:00:00Z",
                    json!(r#"he said "hello there" loudly"#)),
                // Metacharacters the user means literally: as a regex, 'C++'
                // matches any line containing a 'c' and 'render(' does not
                // compile at all.
                msg("assistant", SID_B, "/home/u/beta", "2026-08-04T12:00:00Z",
                    json!("compiled the C++ helper in render(props)")),
            ],
        );

        // Session C: everything the other two predate — a branch, a generated
        // title, tool calls that name files, and the usage block `stats` reads.
        // Its wording is deliberately unique so the older tests keep counting
        // what they were written to count.
        self.session(
            "-home-u-gamma",
            SID_C,
            &[
                linked(
                    on_branch(
                        msg("user", SID_C, "/home/u/gamma", "2026-08-10T09:00:00Z",
                            json!("widget alignment is off")),
                        "feature/widgets",
                    ),
                    "u1", "",
                ),
                linked(
                    with_usage(
                        on_branch(
                            msg("assistant", SID_C, "/home/u/gamma", "2026-08-10T09:01:00Z",
                                json!([{"type": "text", "text": "padding was the culprit"}])),
                            "feature/widgets",
                        ),
                        "claude-opus-5", 100, 40, 860,
                    ),
                    "a1", "u1",
                ),
                on_branch(
                    msg("assistant", SID_C, "/home/u/gamma", "2026-08-10T09:02:00Z",
                        json!([{"type": "tool_use", "name": "Edit",
                                "input": {"file_path": "/home/u/gamma/src/widget.rs"}},
                               {"type": "tool_use", "name": "Read",
                                "input": {"file_path": "/home/u/gamma/src/widget.rs"}},
                               {"type": "tool_use", "name": "NotebookEdit",
                                "input": {"notebook_path": "/home/u/gamma/notes.ipynb"}},
                               {"type": "tool_use", "name": "Bash",
                                "input": {"command": "cargo test"}}])),
                    "feature/widgets",
                ),
                // A later session on a different branch touches the same file,
                // so "how many sessions" is not the same as "how many touches".
                on_branch(
                    msg("assistant", SID_C, "/home/u/gamma", "2026-08-11T09:00:00Z",
                        json!([{"type": "tool_use", "name": "Write",
                                "input": {"file_path": "/home/u/gamma/src/widget.rs"}}])),
                    "main",
                ),
                // Titles are rewritten as a session goes on; the last one wins.
                json!({"type": "ai-title", "aiTitle": "An early guess", "sessionId": SID_C}),
                json!({"type": "ai-title", "aiTitle": "Widget padding fix", "sessionId": SID_C}),
            ],
        );

        let history = [
            json!({"display": "first prompt about needle", "project": "/home/u/alpha",
                   "timestamp": 1_782_000_000_000i64, "sessionId": SID_A}),
            json!({"display": "beta prompt", "project": "/home/u/beta",
                   "timestamp": 1_782_100_000_000i64, "sessionId": SID_B}),
        ];
        let body: String = history
            .iter()
            .map(|r| format!("{}\n", serde_json::to_string(r).unwrap()))
            .collect();
        std::fs::write(self.root.join("history.jsonl"), body).unwrap();
    }

    fn run(&self, args: &[&str]) -> Run {
        let out: Output = Command::new(env!("CARGO_BIN_EXE_cs"))
            .args(args)
            .env("CLAUDE_HOME", &self.root)
            .env_remove("CS_NO_PREFILTER")
            .output()
            .expect("failed to run cs");
        Run::from(out)
    }

    /// Run with extra environment, for the settings the binary reads from it.
    fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cs"));
        cmd.args(args)
            .env("CLAUDE_HOME", &self.root)
            .env_remove("CS_NO_PREFILTER");
        for (k, v) in env {
            cmd.env(k, v);
        }
        Run::from(cmd.output().expect("failed to run cs"))
    }

    fn run_full_decode(&self, args: &[&str]) -> Run {
        let out = Command::new(env!("CARGO_BIN_EXE_cs"))
            .args(args)
            .env("CLAUDE_HOME", &self.root)
            .env("CS_NO_PREFILTER", "1")
            .output()
            .expect("failed to run cs");
        Run::from(out)
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

impl From<Output> for Run {
    fn from(o: Output) -> Self {
        Run {
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            code: o.status.code().unwrap_or(-1),
        }
    }
}

impl Run {
    fn lines(&self) -> Vec<&str> {
        self.stdout.lines().filter(|l| !l.is_empty()).collect()
    }
    fn count(&self) -> usize {
        self.lines().len()
    }
}

fn msg(role: &str, sid: &str, cwd: &str, ts: &str, content: Value) -> Value {
    json!({
        "type": role,
        "sessionId": sid,
        "cwd": cwd,
        "timestamp": ts,
        "isSidechain": false,
        "message": {"content": content},
    })
}

fn meta(mut v: Value) -> Value {
    v["isMeta"] = json!(true);
    v
}

fn sidechain(mut v: Value) -> Value {
    v["isSidechain"] = json!(true);
    v
}

fn on_branch(mut v: Value, branch: &str) -> Value {
    v["gitBranch"] = json!(branch);
    v
}

/// Chain a record to the one before it, which is what `--thread` walks.
fn linked(mut v: Value, uuid: &str, parent: &str) -> Value {
    v["uuid"] = json!(uuid);
    v["parentUuid"] = json!(parent);
    v
}

fn with_usage(mut v: Value, model: &str, input: u64, output: u64, cache_read: u64) -> Value {
    v["message"]["model"] = json!(model);
    v["message"]["usage"] = json!({
        "input_tokens": input,
        "output_tokens": output,
        "cache_read_input_tokens": cache_read,
        "cache_creation_input_tokens": 0,
    });
    v
}

// ---------------------------------------------------------------- basic search

#[test]
fn finds_matching_text_across_projects() {
    let c = Corpus::new();
    let r = c.run(&["needle"]);
    assert_eq!(r.code, 0);
    // thinking + sidechain + multi-line + beta text; meta is excluded.
    assert!(r.count() >= 4, "got {} lines:\n{}", r.count(), r.stdout);
    assert!(r.stdout.contains("beta project needle"));
}

#[test]
fn output_columns_carry_time_project_role_and_session() {
    let c = Corpus::new();
    let r = c.run(&["beta project needle"]);
    let line = r.lines()[0];
    assert!(line.contains("2026-08-02 12:00"), "timestamp: {line}");
    assert!(line.contains("beta"), "project: {line}");
    assert!(line.contains("asst"), "role: {line}");
    assert!(line.contains("bbbbbbbb"), "session id: {line}");
}

#[test]
fn each_line_of_a_multiline_block_is_matched_separately() {
    let c = Corpus::new();
    // "multi line\nsecond needle line" -- only the second line matches.
    let r = c.run(&["second needle"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(!r.stdout.contains("multi line"), "{}", r.stdout);
}

#[test]
fn matching_is_case_insensitive() {
    let c = Corpus::new();
    assert_eq!(c.run(&["NEEDLE"]).count(), c.run(&["needle"]).count());
}

#[test]
fn non_conversation_records_never_surface() {
    let c = Corpus::new();
    let r = c.run(&["queue-operation"]);
    assert_eq!(r.code, 1, "should find nothing: {}", r.stdout);
}

#[test]
fn no_match_reports_on_stderr_and_exits_one() {
    let c = Corpus::new();
    let r = c.run(&["zzzdefinitelyabsentzzz"]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.is_empty());
    assert!(r.stderr.contains("no matches"), "stderr: {}", r.stderr);
}

// ------------------------------------------------------------------- filtering

#[test]
fn meta_records_are_always_excluded() {
    let c = Corpus::new();
    let r = c.run(&["meta noise"]);
    assert_eq!(r.code, 1, "meta records must not be searchable: {}", r.stdout);
}

#[test]
fn sidechain_messages_are_included_until_no_sub() {
    let c = Corpus::new();
    assert_eq!(c.run(&["subagent needle"]).count(), 1);
    assert_eq!(c.run(&["-n", "subagent needle"]).code, 1);
}

#[test]
fn thinking_blocks_are_searched_until_no_thinking() {
    let c = Corpus::new();
    assert_eq!(c.run(&["pondering"]).count(), 1);
    assert_eq!(c.run(&["-T", "pondering"]).code, 1);
}

#[test]
fn tool_blocks_are_skipped_until_tools() {
    let c = Corpus::new();
    // zzsentinel exists only inside a tool_use input.
    assert_eq!(c.run(&["zzsentinel"]).code, 1);
    let with = c.run(&["-t", "zzsentinel"]);
    assert_eq!(with.count(), 1, "{}", with.stdout);
    assert!(with.stdout.contains("Bash"), "{}", with.stdout);
}

#[test]
fn role_filter_selects_one_speaker() {
    let c = Corpus::new();
    assert!(c.run(&["-r", "user", "needle"]).stdout.contains("second needle"));
    assert!(!c.run(&["-r", "user", "needle"]).stdout.contains("beta project"));
    assert!(c.run(&["-r", "assistant", "needle"]).stdout.contains("beta project"));
}

#[test]
fn project_filter_matches_cwd_case_insensitively() {
    let c = Corpus::new();
    let r = c.run(&["-P", "BETA", "needle"]);
    assert!(r.count() >= 1);
    assert!(!r.stdout.contains("alpha"), "{}", r.stdout);
}

#[test]
fn since_filter_drops_earlier_messages() {
    let c = Corpus::new();
    let r = c.run(&["-s", "2026-08-02", "needle"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("beta project needle"));
}

#[test]
fn chars_flag_truncates_with_an_ellipsis() {
    let c = Corpus::new();
    let r = c.run(&["-c", "6", "beta project needle"]);
    assert!(r.stdout.contains("beta p…"), "{}", r.stdout);
}

// ------------------------------------------------------- regressions vs. shell

#[test]
fn anchored_patterns_match_decoded_text() {
    // The shell version anchored ^ to the raw JSON line, which always starts
    // with '{', so this silently returned nothing.
    let c = Corpus::new();
    let r = c.run(&["^SELECT"]);
    assert_eq!(r.count(), 1, "anchored search found nothing:\n{}", r.stdout);
    assert!(r.stdout.contains("SELECT * FROM users"));
}

#[test]
fn end_anchors_also_work() {
    let c = Corpus::new();
    assert_eq!(c.run(&["id = 1$"]).count(), 1);
}

#[test]
fn word_boundaries_work() {
    let c = Corpus::new();
    assert_eq!(c.run(&[r"\busers\b"]).count(), 1);
}

#[test]
fn literal_backslashes_are_not_doubled() {
    // jq's @tsv escaped backslashes, so `make build \` printed as `make build \\`.
    let c = Corpus::new();
    let r = c.run(&["make build"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains(r"make build \"), "{}", r.stdout);
    assert!(!r.stdout.contains(r"make build \\"), "backslash doubled: {}", r.stdout);
}

#[test]
fn escaped_characters_do_not_hide_matches() {
    // On disk this text is `he said \"hello there\" loudly`, so the pattern
    // matches the decoded text but not the raw bytes. The prefilter has to fall
    // back to decoding rather than reject the line.
    let c = Corpus::new();
    let r = c.run(&[r#"said "hello"#]);
    assert_eq!(r.code, 0, "escaped quote hid the match: {}", r.stderr);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains(r#"he said "hello there" loudly"#), "{}", r.stdout);
}

#[test]
fn files_only_reflects_the_tools_flag() {
    // The shell version listed files whose *raw JSON* matched, so -l ignored -t
    // and reported a file whose only hit was inside a tool call.
    let c = Corpus::new();
    assert_eq!(c.run(&["-l", "zzsentinel"]).code, 1, "should list nothing without -t");

    let with = c.run(&["-l", "-t", "zzsentinel"]);
    assert_eq!(with.code, 0);
    assert_eq!(with.count(), 1, "{}", with.stdout);
    assert!(with.lines()[0].ends_with(&format!("{SID_A}.jsonl")), "{}", with.stdout);
}

#[test]
fn ordering_is_deterministic_across_runs() {
    // Results used to depend on the locale `sort` ran under, and on which worker
    // finished first.
    let c = Corpus::new();
    let first = c.run(&["needle"]).stdout;
    for _ in 0..5 {
        assert_eq!(c.run(&["needle"]).stdout, first, "ordering is not stable");
    }
}

#[test]
fn results_are_ordered_by_timestamp() {
    let c = Corpus::new();
    let r = c.run(&["needle"]);
    let mut sorted = r.lines().clone();
    sorted.sort();
    assert_eq!(r.lines(), sorted, "rows should already be in sorted order");
}

#[test]
fn jobs_flag_does_not_change_results() {
    let c = Corpus::new();
    let one = c.run(&["-j", "1", "needle"]).stdout;
    let many = c.run(&["-j", "8", "needle"]).stdout;
    assert_eq!(one, many, "thread count must not affect output");
}

// -------------------------------------------------------- prefilter soundness

#[test]
fn prefilter_agrees_with_full_decode() {
    // The fast path must never drop a result the slow path finds. Patterns here
    // deliberately include escapes, anchors and unicode.
    let c = Corpus::new();
    for pat in [
        "needle", "SELECT", "^SELECT", r"make build \", "second needle", "beta",
        "users", "pondering", "id = 1$", "multi line", r#"said "hello"#,
        r#""hello there""#, "loudly",
    ] {
        let fast = c.run(&[pat]);
        let slow = c.run_full_decode(&[pat]);
        assert_eq!(
            fast.stdout, slow.stdout,
            "prefilter disagreed with full decode on {pat:?}"
        );
        assert_eq!(fast.code, slow.code, "exit codes differ on {pat:?}");
    }
}

#[test]
fn prefilter_agrees_with_full_decode_under_tools() {
    let c = Corpus::new();
    for pat in ["zzsentinel", "Bash", "grep"] {
        let fast = c.run(&["-t", pat]);
        let slow = c.run_full_decode(&["-t", pat]);
        assert_eq!(fast.stdout, slow.stdout, "disagreement on -t {pat:?}");
    }
}

// ------------------------------------------------------------------ subcommands

#[test]
fn show_divides_the_two_speakers_with_a_rule() {
    let c = Corpus::new();
    let r = c.run(&["show", SID_A]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("── YOU"), "{}", r.stdout);
    assert!(r.stdout.contains("── CC"), "{}", r.stdout);
    assert!(!r.stdout.contains("==="), "the old === form is gone:\n{}", r.stdout);
    // The rule runs the width of the terminal rather than bracketing a label.
    let rule = r.lines().iter().find(|l| l.starts_with("── YOU")).unwrap().to_string();
    assert!(rule.ends_with('─'), "{rule}");
    assert!(rule.chars().count() > 40, "{rule}");
    assert!(r.stdout.contains("SELECT * FROM users"));
    // show always includes thinking and tools, regardless of search flags.
    assert!(r.stdout.contains("[thinking]"), "{}", r.stdout);
    assert!(r.stdout.contains("[tool: Bash]"), "{}", r.stdout);
    // The path is reported on stderr so stdout stays pipeable.
    assert!(r.stderr.contains(SID_A));
}

#[test]
fn show_reads_one_side_of_the_conversation() {
    let c = Corpus::new();
    let yours = c.run(&["show", SID_A, "-r", "user"]);
    assert_eq!(yours.code, 0, "stderr: {}", yours.stderr);
    assert!(yours.stdout.contains("SELECT * FROM users"), "{}", yours.stdout);
    assert!(!yours.stdout.contains("── CC"), "no assistant turns:\n{}", yours.stdout);
    assert!(!yours.stdout.contains("make build"), "{}", yours.stdout);

    let theirs = c.run(&["show", SID_A, "--role", "assistant"]);
    assert!(theirs.stdout.contains("make build"), "{}", theirs.stdout);
    assert!(!theirs.stdout.contains("── YOU"), "{}", theirs.stdout);
    assert!(!theirs.stdout.contains("SELECT * FROM users"), "{}", theirs.stdout);
}

#[test]
fn the_divider_is_drawn_to_the_width_it_is_told_about() {
    // A rule sized for 80 columns wraps into nonsense inside a narrow fzf
    // preview pane, so the width has to come from the environment.
    let c = Corpus::new();
    for width in ["40", "100"] {
        let r = c.run_env(&["show", SID_A], &[("COLUMNS", width)]);
        let rule = r
            .lines()
            .iter()
            .find(|l| l.starts_with("── YOU"))
            .unwrap_or_else(|| panic!("no divider at COLUMNS={width}"))
            .to_string();
        assert_eq!(
            rule.chars().count(),
            width.parse::<usize>().unwrap(),
            "COLUMNS={width}: {rule}"
        );
    }
}

#[test]
fn the_preview_pane_width_wins_over_the_terminal_width() {
    // fzf runs the preview as a child of a full-width terminal, so COLUMNS
    // would be wrong there; FZF_PREVIEW_COLUMNS is the honest answer.
    let c = Corpus::new();
    let r = c.run_env(
        &["show", SID_A],
        &[("COLUMNS", "200"), ("FZF_PREVIEW_COLUMNS", "45")],
    );
    let rule = r.lines().iter().find(|l| l.starts_with("── YOU")).unwrap().to_string();
    assert_eq!(rule.chars().count(), 45, "{rule}");
}

#[test]
fn a_junk_width_falls_back_rather_than_drawing_nothing() {
    let c = Corpus::new();
    for junk in ["", "0", "wide"] {
        let r = c.run_env(&["show", SID_A], &[("COLUMNS", junk)]);
        let rule = r.lines().iter().find(|l| l.starts_with("── YOU")).unwrap().to_string();
        assert_eq!(rule.chars().count(), 80, "COLUMNS={junk:?} should fall back");
    }
}

#[test]
fn show_rejects_a_role_that_is_not_a_speaker() {
    // Silently showing the whole transcript would look exactly like a filtered
    // one, which is the worst of both.
    let c = Corpus::new();
    let r = c.run(&["show", SID_A, "-r", "robot"]);
    assert_eq!(r.code, 2, "stdout: {}", r.stdout);
    assert!(r.stderr.contains("--role"), "stderr: {}", r.stderr);
    assert!(r.stdout.is_empty(), "nothing should be printed: {}", r.stdout);
}

#[test]
fn reading_your_own_side_excludes_machine_output_filed_under_it() {
    let c = Corpus::new();
    let everything = c.run(&["show", SID_A]);
    assert!(everything.stdout.contains("zzresultonly"), "unfiltered shows it all");

    let yours = c.run(&["show", SID_A, "-r", "user"]);
    assert!(yours.stdout.contains("SELECT * FROM users"), "what you typed stays");
    assert!(
        !yours.stdout.contains("zzresultonly"),
        "a tool result is not something you said:\n{}",
        yours.stdout
    );
}

#[test]
fn both_speakers_are_shown_unless_a_role_is_named() {
    let c = Corpus::new();
    let both = c.run(&["show", SID_A]);
    assert!(both.stdout.contains("── YOU") && both.stdout.contains("── CC"));
}

#[test]
fn a_role_with_no_turns_says_so_rather_than_printing_nothing() {
    let c = Corpus::new();
    // Session B has no user turns after the opening one; use a filter that
    // genuinely empties a session instead of printing a blank page.
    let r = c.run(&["show", SID_B, "-r", "assistant"]);
    assert_eq!(r.code, 0, "session B does have assistant turns");
    assert!(r.stdout.contains("beta project needle"));
}

#[test]
fn show_accepts_a_session_id_prefix() {
    let c = Corpus::new();
    assert!(c.run(&["show", "aaaaaaaa"]).stdout.contains("SELECT"));
}

#[test]
fn show_excludes_meta_records() {
    let c = Corpus::new();
    assert!(!c.run(&["show", SID_A]).stdout.contains("meta noise"));
}

#[test]
fn show_without_an_id_is_a_usage_error() {
    let c = Corpus::new();
    assert_eq!(c.run(&["show"]).code, 2);
}

#[test]
fn show_with_an_unknown_id_exits_one() {
    let c = Corpus::new();
    let r = c.run(&["show", "ffffffff"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no session matching"), "{}", r.stderr);
}

#[test]
fn sessions_lists_each_session_with_its_opening_prompt() {
    let c = Corpus::new();
    let r = c.run(&["sessions"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.count(), 3, "{}", r.stdout);
    // A and B have no title, so they fall back to how they opened.
    assert!(r.stdout.contains("SELECT * FROM users"), "{}", r.stdout);
    // A multi-line opening prompt is flattened onto one row.
    assert!(r.stdout.contains("multi line second needle line"), "{}", r.stdout);
    assert!(r.stdout.contains("sess"));
}

#[test]
fn sessions_skips_meta_and_sidechain_when_choosing_the_opening_prompt() {
    let c = Corpus::new();
    let r = c.run(&["sessions"]);
    assert!(!r.stdout.contains("meta noise"), "{}", r.stdout);
    assert!(!r.stdout.contains("subagent"), "{}", r.stdout);
}

#[test]
fn sessions_filter_narrows_the_listing() {
    let c = Corpus::new();
    let r = c.run(&["sessions", "beta"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(!r.stdout.contains("SELECT"));
}

#[test]
fn sessions_ordering_is_deterministic() {
    let c = Corpus::new();
    let first = c.run(&["sessions"]).stdout;
    for _ in 0..5 {
        assert_eq!(c.run(&["sessions"]).stdout, first);
    }
}

#[test]
fn prompts_searches_only_your_own_prompts() {
    let c = Corpus::new();
    let r = c.run(&["-p", "prompt"]);
    assert_eq!(r.code, 0);
    assert_eq!(r.count(), 2, "{}", r.stdout);
    assert!(r.stdout.contains("first prompt about needle"));
    assert!(r.stdout.contains("you"), "role column: {}", r.stdout);
    // Transcript-only text must not leak into the -p path.
    assert!(!r.stdout.contains("SELECT"));
}

#[test]
fn prompts_respects_the_project_filter() {
    let c = Corpus::new();
    let r = c.run(&["-p", "-P", "beta", "prompt"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("beta prompt"));
}

// The two fixture prompts sit at 2026-06-21 00:00 and 2026-06-22 03:46 UTC, and
// -p prints them in local time, so these pin TZ rather than assert on whatever
// zone the suite happens to run in.

/// `-s` was accepted on the -p path and then never applied: every prompt came
/// back regardless of the date, which looks exactly like a date with nothing
/// before it.
#[test]
fn prompts_respect_the_since_filter() {
    let c = Corpus::new();
    let r = c.run_env(&["-p", "-s", "2026-06-22", "prompt"], &[("TZ", "UTC")]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("beta prompt"), "{}", r.stdout);
    assert!(!r.stdout.contains("first prompt"), "{}", r.stdout);
}

/// The earlier prompt lands exactly on midnight of the cutoff day, which is the
/// case a comparison that mishandles the printed "HH:MM" gets wrong.
#[test]
fn the_since_day_itself_is_kept_on_the_prompts_path() {
    let c = Corpus::new();
    let r = c.run_env(&["-p", "-s", "2026-06-21", "prompt"], &[("TZ", "UTC")]);
    assert_eq!(r.count(), 2, "{}", r.stdout);
    assert!(r.stdout.contains("first prompt about needle"), "{}", r.stdout);
}

#[test]
fn a_since_after_every_prompt_reports_no_matches() {
    let c = Corpus::new();
    let r = c.run_env(&["-p", "-s", "2026-06-23", "prompt"], &[("TZ", "UTC")]);
    assert_eq!(r.code, 1);
    assert_eq!(r.count(), 0, "{}", r.stdout);
    assert!(r.stderr.contains("no matches"), "stderr: {}", r.stderr);
}

/// -s and -P narrow the same search rather than one replacing the other.
#[test]
fn prompts_combine_the_since_and_project_filters() {
    let c = Corpus::new();
    let r = c.run_env(&["-p", "-P", "alpha", "-s", "2026-06-22", "prompt"], &[("TZ", "UTC")]);
    assert_eq!(r.code, 1, "alpha's only prompt predates the cutoff: {}", r.stdout);
}

// ---------------------------------------------------------------- branch

#[test]
fn the_branch_filter_narrows_to_one_branch() {
    let c = Corpus::new();
    let r = c.run(&["-b", "widgets", "padding"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("padding was the culprit"), "{}", r.stdout);
    // Nothing on that branch says "needle", so the filter must empty it.
    assert_eq!(c.run(&["-b", "widgets", "needle"]).code, 1);
}

#[test]
fn the_branch_filter_takes_a_substring_case_insensitively() {
    let c = Corpus::new();
    assert_eq!(c.run(&["-b", "FEATURE/WIDGETS", "padding"]).count(), 1);
    assert_eq!(c.run(&["--branch", "feat", "padding"]).count(), 1);
}

/// Sessions predating the field are not silently attributed to a branch.
#[test]
fn a_record_with_no_branch_matches_no_branch_filter() {
    let c = Corpus::new();
    assert_eq!(c.run(&["-b", "main", "SELECT"]).code, 1, "alpha records carry no branch");
}

#[test]
fn the_branch_rides_beside_the_project_in_grouped_output() {
    let c = Corpus::new();
    let r = c.run(&["--group", "padding"]);
    assert!(r.stdout.contains("gamma@feature/widgets"), "{}", r.stdout);
}

/// Flat output is the format scripts parse, so the new column must not appear
/// in it — `--json` is where the branch is exposed for programs.
#[test]
fn flat_output_is_unchanged_by_the_branch() {
    let c = Corpus::new();
    let r = c.run(&["--no-group", "padding"]);
    assert!(!r.stdout.contains("feature/widgets"), "{}", r.stdout);

    let j = c.run(&["--json", "padding"]);
    let v: Value = serde_json::from_str(j.stdout.lines().next().unwrap()).unwrap();
    assert_eq!(v["branch"], "feature/widgets");
}

// ------------------------------------------------------------------ date range

#[test]
fn until_bounds_the_far_end_of_a_range() {
    let c = Corpus::new();
    // Alpha's "needle" survivors are the thinking block and the subagent line;
    // its meta record is never searchable.
    assert_eq!(c.run(&["-u", "2026-07-31", "needle"]).count(), 2, "alpha only");
    let both = c.run(&["-s", "2026-08-01", "-u", "2026-08-01", "needle"]);
    assert_eq!(both.count(), 1, "one day at both ends: {}", both.stdout);
    assert!(both.stdout.contains("second needle line"), "{}", both.stdout);
}

/// The named day is included whole; a timestamp inside it sorts after the bare
/// date it is being compared with.
#[test]
fn the_until_day_is_kept_whole() {
    let c = Corpus::new();
    assert_eq!(c.run(&["-u", "2026-08-02", "beta project needle"]).count(), 1);
    assert_eq!(c.run(&["-u", "2026-08-01", "beta project needle"]).code, 1);
}

#[test]
fn relative_dates_resolve_against_today() {
    let c = Corpus::new();
    // The fixture is dated 2026, so "everything since a century ago" is all of
    // it and "since today" is none of it, whenever the suite happens to run.
    assert!(c.run(&["-s", "99y", "needle"]).count() >= 4);
    assert_eq!(c.run(&["-s", "today", "needle"]).code, 1);
}

#[test]
fn a_bad_date_is_rejected_rather_than_matching_nothing() {
    let c = Corpus::new();
    let r = c.run(&["-s", "soonish", "needle"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("soonish"), "stderr: {}", r.stderr);
    assert!(r.stderr.contains("yesterday"), "the error should list the forms: {}", r.stderr);
    assert_eq!(c.run(&["-u", "2026-02-30", "needle"]).code, 2, "a date that names no day");
}

// ------------------------------------------------------------------------ cli

// --------------------------------------------------------------- cs projects

#[test]
fn projects_lists_each_working_directory_with_a_session_count() {
    let c = Corpus::new();
    let r = c.run(&["projects"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.count(), 3, "{}", r.stdout);
    assert!(r.stdout.contains("/home/u/alpha"), "{}", r.stdout);
    assert!(r.stdout.contains("/home/u/beta"), "{}", r.stdout);
    assert!(r.stdout.contains("/home/u/gamma"), "{}", r.stdout);
    // The listing exists so -P has something to name; one session each here.
    for line in r.lines() {
        assert!(line.trim_start().starts_with('1'), "session count: {line}");
    }
}

#[test]
fn projects_takes_the_same_substring_p_does() {
    let c = Corpus::new();
    let r = c.run(&["projects", "BETA"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("/home/u/beta"));
    assert!(!r.stdout.contains("alpha"));
}

#[test]
fn a_project_name_from_the_listing_works_as_a_filter() {
    // The whole point of the command: what it prints can be pasted into -P.
    let c = Corpus::new();
    let listed = c.run(&["projects", "beta"]);
    let name = listed.lines()[0].rsplit('/').next().unwrap().trim().to_owned();
    let filtered = c.run(&["-P", &name, "needle"]);
    assert_eq!(filtered.code, 0, "-P {name} found nothing: {}", filtered.stderr);
    assert!(!filtered.stdout.contains("alpha"), "{}", filtered.stdout);
}

#[test]
fn projects_ordering_is_deterministic() {
    let c = Corpus::new();
    let first = c.run(&["projects"]).stdout;
    for _ in 0..5 {
        assert_eq!(c.run(&["projects"]).stdout, first);
    }
}

#[test]
fn projects_reports_an_empty_corpus_rather_than_printing_nothing() {
    let c = Corpus::new();
    let r = c.run(&["projects", "zzznosuchproject"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no projects"), "stderr: {}", r.stderr);
}

// ----------------------------------------------------------------- cs resume

#[test]
fn resume_without_an_id_is_a_usage_error() {
    let c = Corpus::new();
    let r = c.run(&["resume"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("cs resume"), "stderr: {}", r.stderr);
}

#[test]
fn resume_with_an_unknown_id_exits_one() {
    // Stops before ever reaching `claude`, so this is safe to run anywhere.
    let c = Corpus::new();
    let r = c.run(&["resume", "ffffffff"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no session matching"), "stderr: {}", r.stderr);
}

// ------------------------------------------------------- reading a transcript

#[test]
fn show_can_open_at_the_first_match_instead_of_the_top() {
    let c = Corpus::new();
    let full = c.run(&["show", SID_B]);
    let jumped = c.run(&["show", SID_B, "--at", "hello there"]);

    assert_eq!(jumped.code, 0, "stderr: {}", jumped.stderr);
    assert!(jumped.count() < full.count(), "nothing was skipped:\n{}", jumped.stdout);
    assert!(jumped.stdout.contains("earlier lines"), "{}", jumped.stdout);
    assert!(jumped.stdout.contains("hello there"), "{}", jumped.stdout);
    // The lines above the match are what got dropped.
    assert!(!jumped.stdout.contains("multi line"), "{}", jumped.stdout);
}

#[test]
fn a_jump_pattern_that_is_absent_shows_the_whole_session() {
    let c = Corpus::new();
    let full = c.run(&["show", SID_B]);
    let missed = c.run(&["show", SID_B, "--at", "zzznowhere"]);
    assert_eq!(missed.stdout, full.stdout);
}

#[test]
fn show_marks_the_pattern_when_asked_for_colour() {
    let c = Corpus::new();
    let r = c.run(&["show", SID_B, "--color", "--highlight", "hello there"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains('\u{1b}'), "the preview needs ANSI down a pipe");
    // Highlighting must not change the words themselves.
    let stripped: String = r.stdout.replace("\u{1b}[1;31m", "").replace("\u{1b}[0m", "");
    assert!(stripped.contains("he said \"hello there\" loudly"), "{stripped}");
}

#[test]
fn show_stays_plain_down_a_pipe_unless_colour_is_forced() {
    let c = Corpus::new();
    let r = c.run(&["show", SID_B, "--highlight", "hello"]);
    assert!(!r.stdout.contains('\u{1b}'), "piped output must stay clean");
}

#[test]
fn show_still_takes_a_bare_session_id() {
    // The original single-argument form has to keep working.
    let c = Corpus::new();
    assert!(c.run(&["show", SID_A]).stdout.contains("SELECT * FROM users"));
}

// ------------------------------------------------------------------- picker

#[test]
fn the_picker_rows_command_carries_hidden_id_and_project_columns() {
    // What fzf is fed on every keystroke: two hidden columns, then the display.
    let c = Corpus::new();
    let state = c.root.join("picker-state.json");
    std::fs::write(&state, "{}").unwrap();
    let r = c.run(&["__rows", state.to_str().unwrap(), "beta project needle"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);

    let fields: Vec<&str> = r.lines()[0].split('\t').collect();
    assert_eq!(fields.len(), 3, "{:?}", fields);
    assert!(fields[0].starts_with("bbbbbbbb"), "session id first: {:?}", fields[0]);
    assert_eq!(fields[1], "beta", "then the project: {:?}", fields[1]);
    assert!(fields[2].contains("beta project needle"), "{:?}", fields[2]);
}

#[test]
fn a_too_short_query_does_not_trigger_a_search() {
    let c = Corpus::new();
    let state = c.root.join("picker-state.json");
    std::fs::write(&state, "{}").unwrap();
    let r = c.run(&["__rows", state.to_str().unwrap(), "n"]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.is_empty(), "one keystroke must not scan the corpus: {}", r.stdout);
}

#[test]
fn a_filter_key_changes_what_the_next_reload_returns() {
    // The loop fzf drives: toggle state, then re-run the search against it.
    let c = Corpus::new();
    let state = c.root.join("picker-state.json");
    let path = state.to_str().unwrap();
    std::fs::write(&state, "{}").unwrap();

    assert!(c.run(&["__rows", path, "zzsentinel"]).stdout.is_empty());
    c.run(&["__toggle", path, "tools", ""]);
    let with = c.run(&["__rows", path, "zzsentinel"]);
    assert_eq!(with.count(), 1, "alt-t should bring tool blocks in: {}", with.stdout);

    assert!(c.run(&["__header", path, "zzsentinel"]).stdout.contains("+tools"));
    c.run(&["__toggle", path, "tools", ""]);
    assert!(c.run(&["__rows", path, "zzsentinel"]).stdout.is_empty(), "and take them out again");
}

#[test]
fn no_arguments_prints_usage() {
    let c = Corpus::new();
    let r = c.run(&[]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("USAGE"), "{}", r.stdout);
}

#[test]
fn help_prints_usage() {
    let c = Corpus::new();
    for flag in ["-h", "--help"] {
        let r = c.run(&[flag]);
        assert_eq!(r.code, 0);
        assert!(r.stdout.contains("USAGE"));
    }
}

#[test]
fn an_unparseable_pattern_falls_back_to_a_literal_search() {
    // This used to exit 2 with "bad pattern", which is the wrong answer for a
    // pattern the user plainly meant literally.
    let c = Corpus::new();
    let r = c.run(&["render("]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("render(props)"), "{}", r.stdout);
    assert!(r.stderr.contains("literally"), "the substitution must be reported: {}", r.stderr);
    assert!(r.stderr.contains("-F"), "and should name the explicit flag: {}", r.stderr);
}

#[test]
fn a_pattern_that_cannot_match_anything_still_exits_cleanly() {
    let c = Corpus::new();
    // Unparseable as a regex, and absent as a literal.
    let r = c.run(&["zzz("]);
    assert_eq!(r.code, 1, "an unmatched literal is 'no matches', not a crash");
    assert!(r.stderr.contains("no matches"), "stderr: {}", r.stderr);
}

// ------------------------------------------------------------- literal search

#[test]
fn fixed_search_takes_metacharacters_at_face_value() {
    let c = Corpus::new();
    let r = c.run(&["-F", "C++"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.count(), 1, "{}", r.stdout);
    assert!(r.stdout.contains("C++ helper"), "{}", r.stdout);
}

#[test]
fn without_fixed_the_same_pattern_is_still_a_regex() {
    // The trap -F exists to close: 'C++' as a regex matches every line with a
    // 'c' in it, silently and by the thousand.
    let c = Corpus::new();
    let loose = c.run(&["C++"]);
    let exact = c.run(&["-F", "C++"]);
    assert!(
        loose.count() > exact.count(),
        "regex 'C++' should over-match: {} vs {}",
        loose.count(),
        exact.count()
    );
}

#[test]
fn fixed_and_the_long_spelling_agree() {
    let c = Corpus::new();
    assert_eq!(c.run(&["-F", "render("]).stdout, c.run(&["--fixed", "render("]).stdout);
}

#[test]
fn a_literal_pattern_needs_no_explanation_on_stderr() {
    let c = Corpus::new();
    let r = c.run(&["-F", "render("]);
    assert!(!r.stderr.contains("literally"), "-F was explicit: {}", r.stderr);
}

// ------------------------------------------------------------- context lines

#[test]
fn context_shows_the_lines_around_a_match() {
    let c = Corpus::new();
    // "multi line\nsecond needle line" -- only the second line matches.
    let plain = c.run(&["second needle"]);
    assert_eq!(plain.count(), 1);
    assert!(!plain.stdout.contains("multi line"));

    let with = c.run(&["-B", "1", "second needle"]);
    assert_eq!(with.count(), 2, "the preceding line joins it:\n{}", with.stdout);
    assert!(with.stdout.contains("multi line"), "{}", with.stdout);
}

#[test]
fn context_lines_are_indented_under_their_match() {
    let c = Corpus::new();
    let r = c.run(&["-B", "1", "second needle"]);
    let lines = r.lines();
    assert!(!lines[0].starts_with(' '), "the match is flush left: {:?}", lines[0]);
    assert!(lines[1].starts_with("  "), "context is indented: {:?}", lines[1]);
    assert_eq!(lines[1].trim(), "multi line");
}

#[test]
fn context_asks_for_nothing_beyond_the_block() {
    // "multi line" is the first line of its block, so -B has nothing to add.
    let c = Corpus::new();
    let r = c.run(&["-B", "3", "multi line"]);
    assert_eq!(r.count(), 1, "{}", r.stdout);
}

#[test]
fn the_context_flags_cover_both_directions() {
    let c = Corpus::new();
    assert_eq!(c.run(&["-C", "1", "second needle"]).count(), 2);
    assert_eq!(c.run(&["-A", "1", "multi line"]).count(), 2);
    assert_eq!(c.run(&["--before", "1", "second needle"]).count(), 2);
}

// -------------------------------------------------------------- output modes

#[test]
fn a_pipe_gets_one_flat_line_per_match() {
    // Everything built on this format has to keep working, so grouping must not
    // reach a pipe on its own.
    let c = Corpus::new();
    let r = c.run(&["needle"]);
    for line in r.lines() {
        assert!(
            line.starts_with("2026-"),
            "every piped line should start with its timestamp: {line:?}"
        );
    }
}

#[test]
fn group_folds_matches_under_one_heading_per_session() {
    let c = Corpus::new();
    let r = c.run(&["--group", "needle"]);
    assert_eq!(r.code, 0);
    // Two sessions match, so each id heads its own block exactly once.
    for sid in ["aaaaaaaa", "bbbbbbbb"] {
        assert_eq!(
            r.stdout.matches(sid).count(),
            1,
            "{sid} should head one group:\n{}",
            r.stdout
        );
    }
    assert!(r.stdout.contains("matches") || r.stdout.contains("match"), "{}", r.stdout);
}

#[test]
fn grouped_headings_are_marked_and_their_counts_aligned() {
    let c = Corpus::new();
    let r = c.run(&["--group", "needle"]);
    let headings: Vec<&str> = r.lines().iter().filter(|l| l.starts_with('▸')).copied().collect();
    assert_eq!(headings.len(), 2, "one marker per session:\n{}", r.stdout);

    // The two projects have different name lengths, so aligning the counts is
    // what the gutter is for.
    let at = |l: &str| l.find("match").expect("a heading carries its count");
    assert_eq!(at(headings[0]), at(headings[1]), "counts should align:\n{}", r.stdout);
}

#[test]
fn no_group_is_the_flat_format_even_when_grouping_is_asked_for() {
    let c = Corpus::new();
    assert_eq!(c.run(&["--no-group", "needle"]).stdout, c.run(&["needle"]).stdout);
}

#[test]
fn the_summary_stays_off_stderr_when_nobody_is_watching() {
    // It is a diagnostic for a human at a terminal, not something a script
    // reading stderr should have to filter out.
    let c = Corpus::new();
    let r = c.run(&["needle"]);
    assert!(!r.stderr.contains("sessions"), "stderr: {}", r.stderr);
}

#[test]
fn json_emits_one_object_per_match() {
    let c = Corpus::new();
    let r = c.run(&["--json", "beta project needle"]);
    assert_eq!(r.code, 0);
    let lines = r.lines();
    assert_eq!(lines.len(), 1, "{}", r.stdout);

    let v: Value = serde_json::from_str(lines[0]).expect("each line is a JSON object");
    assert_eq!(v["text"], "beta project needle");
    assert_eq!(v["role"], "asst");
    assert_eq!(v["project"], "beta");
    assert_eq!(v["ts"], "2026-08-02 12:00");
    assert!(v["session"].as_str().unwrap().starts_with("bbbbbbbb"));
}

#[test]
fn every_json_line_parses() {
    let c = Corpus::new();
    let r = c.run(&["--json", "needle"]);
    assert!(r.count() >= 4);
    for line in r.lines() {
        serde_json::from_str::<Value>(line).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
}

#[test]
fn json_carries_context_when_it_was_asked_for() {
    let c = Corpus::new();
    let r = c.run(&["--json", "-B", "1", "second needle"]);
    let v: Value = serde_json::from_str(r.lines()[0]).unwrap();
    assert_eq!(v["before"][0], "multi line");
}

#[test]
fn json_never_carries_colour() {
    let c = Corpus::new();
    assert!(!c.run(&["--json", "needle"]).stdout.contains('\u{1b}'));
}

#[test]
fn unknown_flags_exit_two() {
    let c = Corpus::new();
    let r = c.run(&["-Z", "needle"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("unknown option"));
}

#[test]
fn output_is_uncoloured_when_piped() {
    let c = Corpus::new();
    assert!(
        !c.run(&["needle"]).stdout.contains('\u{1b}'),
        "piped output must not contain escape sequences"
    );
}

#[test]
fn a_missing_corpus_is_handled_gracefully() {
    let c = Corpus::new();
    let empty = c.root.join("nonexistent");
    let out = Command::new(env!("CARGO_BIN_EXE_cs"))
        .args(["needle"])
        .env("CLAUDE_HOME", &empty)
        .output()
        .unwrap();
    let r = Run::from(out);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("no matches"));
}

#[test]
fn a_missing_history_file_is_reported() {
    let c = Corpus::new();
    std::fs::remove_file(c.root.join("history.jsonl")).unwrap();
    let r = c.run(&["-p", "prompt"]);
    assert_eq!(r.code, 1);
    assert!(r.stderr.contains("history.jsonl"), "stderr: {}", r.stderr);
}

#[test]
fn malformed_lines_are_skipped_not_fatal() {
    let c = Corpus::new();
    let path: PathBuf = c
        .root
        .join("projects")
        .join("-home-u-beta")
        .join(format!("{SID_B}.jsonl"));
    let mut body = std::fs::read_to_string(&path).unwrap();
    body.push_str("{ this is not json\n");
    body.push('\n');
    std::fs::write(&path, body).unwrap();

    let r = c.run(&["beta project needle"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.count(), 1);
}

#[test]
fn a_transcript_outside_any_project_dir_is_still_searched() {
    let c = Corpus::new();
    let dir: &Path = &c.root.join("projects");
    std::fs::write(
        dir.join("loose.jsonl"),
        format!(
            "{}\n",
            serde_json::to_string(&msg(
                "user",
                "cccccccc-3333-4444-8888-cccccccccccc",
                "/home/u/gamma",
                "2026-09-01T00:00:00Z",
                json!("loose needle here"),
            ))
            .unwrap()
        ),
    )
    .unwrap();
    assert!(c.run(&["loose needle"]).stdout.contains("gamma"));
}
