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
    assert!(line.contains("assi"), "role: {line}");
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
fn show_renders_a_transcript_with_speaker_headers() {
    let c = Corpus::new();
    let r = c.run(&["show", SID_A]);
    assert_eq!(r.code, 0);
    assert!(r.stdout.contains("=== YOU"), "{}", r.stdout);
    assert!(r.stdout.contains("=== CC"), "{}", r.stdout);
    assert!(r.stdout.contains("SELECT * FROM users"));
    // show always includes thinking and tools, regardless of search flags.
    assert!(r.stdout.contains("[thinking]"), "{}", r.stdout);
    assert!(r.stdout.contains("[tool: Bash]"), "{}", r.stdout);
    // The path is reported on stderr so stdout stays pipeable.
    assert!(r.stderr.contains(SID_A));
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
    assert_eq!(r.count(), 2, "{}", r.stdout);
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

// ------------------------------------------------------------------------ cli

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
fn an_invalid_regex_is_reported_not_panicked() {
    let c = Corpus::new();
    let r = c.run(&["("]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("bad pattern"), "stderr: {}", r.stderr);
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
