//! `cs -i` — the picker, and on a terminal the default way results are shown.
//!
//! fzf is used as a front-end for the search rather than as a filter over its
//! results: `--disabled` stops fzf matching on the query itself, and a
//! `change:reload` binding re-runs `cs` instead. Typing therefore searches the
//! whole corpus again rather than narrowing a list frozen at launch, which is
//! what made the old picker a dead end whenever the first pattern was wrong.
//!
//! Filters are keys rather than flags you have to quit and retype, held in a
//! state file between the processes fzf spawns — see `picker`.

use crate::cli::Opts;
use crate::output::Row;
use crate::{picker, show};
use regex::Regex;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// `None` means fzf is not installed; the caller decides whether that is an
/// error (`-i` was asked for) or a reason to print results instead.
pub fn run(rows: &[Row], opts: &Opts, re: &Regex) -> Option<i32> {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "cs".into());
    let state = picker::state_path();
    picker::save(&state, opts);

    let args = fzf_args(&exe, &state, opts);
    let mut child = Command::new("fzf")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    if let Some(mut stdin) = child.stdin.take() {
        let mut buf: Vec<u8> = Vec::new();
        picker::render_rows(&mut buf, rows, Some(re));
        let _ = stdin.write_all(&buf);
    }

    let out = child.wait_with_output();
    let _ = std::fs::remove_file(&state);
    let Ok(out) = out else { return Some(1) };

    // --print-query puts the final query first, so the transcript is opened
    // highlighting what the user last typed, not what they originally ran.
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let query = lines.next().unwrap_or("").to_owned();
    let Some(sid) = lines.next().and_then(|l| l.split('\t').next()).map(str::trim) else {
        return Some(0);
    };
    if sid.is_empty() {
        return Some(0);
    }

    let jump = show::pattern(&query);
    Some(show::run_with(
        sid,
        &show::ShowOpts {
            highlight: jump.clone(),
            at: jump,
            pager: true,
            ..Default::default()
        },
    ))
}

fn fzf_args(exe: &str, state: &Path, opts: &Opts) -> Vec<String> {
    let e = quote(exe);
    let s = quote(&state.display().to_string());
    let rows = format!("{e} __rows {s} {{q}}");
    let header = format!("{e} __header {s} {{q}}");
    // Every filter key does the same three things: change the state, re-run the
    // search against it, and redraw the header so the change is visible.
    let key = |field: &str, arg: &str| {
        format!("execute-silent({e} __toggle {s} {field} {arg})+reload({rows})+transform-header({header})")
    };

    vec![
        "--ansi".into(),
        "--no-sort".into(),
        "--reverse".into(),
        "--height=90%".into(),
        "--delimiter=\t".into(),
        "--with-nth=3..".into(),
        // fzf must not filter: the query belongs to cs.
        "--disabled".into(),
        "--print-query".into(),
        "--prompt=cs > ".into(),
        format!("--query={}", opts.pattern),
        format!("--header={}", picker::header(state, &opts.pattern)),
        format!("--preview={e} show {{1}} --color --no-pager --highlight {{q}} --at {{q}}"),
        "--preview-window=right:55%:wrap".into(),
        format!("--bind=change:reload({rows})+transform-header({header})"),
        format!("--bind=alt-t:{}", key("tools", "")),
        format!("--bind=alt-h:{}", key("thinking", "")),
        format!("--bind=alt-s:{}", key("sub", "")),
        format!("--bind=alt-r:{}", key("role", "")),
        // {2} is the hidden project column of the highlighted row.
        format!("--bind=alt-p:{}", key("project", "{2}")),
        format!("--bind=alt-c:{}", key("clear", "")),
        // `become` replaces fzf with Claude Code, so the session takes over the
        // terminal cleanly instead of running underneath a live picker.
        format!("--bind=alt-enter:become({e} resume {{1}})"),
    ]
}

/// fzf runs bindings through a shell, so anything we interpolate has to survive
/// it. fzf quotes its own `{...}` placeholders; these are the parts we supply.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Vec<String> {
        fzf_args(
            "/usr/local/bin/cs",
            Path::new("/tmp/cs-state.json"),
            &Opts { pattern: "needle".into(), ..Default::default() },
        )
    }

    fn arg_starting(prefix: &str) -> String {
        args()
            .into_iter()
            .find(|a| a.starts_with(prefix))
            .unwrap_or_else(|| panic!("no argument starting {prefix}"))
    }

    #[test]
    fn the_query_drives_a_reload_rather_than_fzf_own_filtering() {
        let all = args();
        assert!(all.iter().any(|a| a == "--disabled"), "fzf must not filter: {all:?}");
        let bind = arg_starting("--bind=change:");
        assert!(bind.contains("reload("), "typing has to re-run the search: {bind}");
        assert!(bind.contains("__rows"), "{bind}");
        assert!(bind.contains("{q}"), "the reload has to receive the query: {bind}");
    }

    #[test]
    fn the_starting_query_is_the_pattern_that_was_run() {
        assert_eq!(arg_starting("--query="), "--query=needle");
    }

    #[test]
    fn the_preview_opens_at_the_match_and_is_not_truncated() {
        let p = arg_starting("--preview=");
        assert!(p.contains("--at {q}"), "preview should jump to the match: {p}");
        assert!(p.contains("--highlight {q}"), "{p}");
        assert!(p.contains("--color"), "a piped preview still needs ANSI: {p}");
        assert!(!p.contains("head -"), "the old preview could not show late matches: {p}");
    }

    #[test]
    fn every_filter_key_updates_state_results_and_header_together() {
        for k in ["alt-t", "alt-h", "alt-s", "alt-r", "alt-p", "alt-c"] {
            let b = arg_starting(&format!("--bind={k}:"));
            assert!(b.contains("__toggle"), "{k} should change state: {b}");
            assert!(b.contains("reload("), "{k} should re-run the search: {b}");
            assert!(b.contains("transform-header("), "{k} should redraw the header: {b}");
        }
    }

    #[test]
    fn the_project_key_is_given_the_row_under_the_cursor() {
        assert!(arg_starting("--bind=alt-p:").contains("project {2}"));
    }

    #[test]
    fn alt_enter_hands_the_terminal_to_claude_code() {
        let b = arg_starting("--bind=alt-enter:");
        assert!(b.contains("become("), "resume must replace fzf, not nest under it: {b}");
        assert!(b.contains("resume {1}"), "{b}");
    }

    #[test]
    fn the_hidden_columns_are_not_displayed() {
        let all = args();
        assert!(all.iter().any(|a| a == "--with-nth=3.."), "{all:?}");
        assert!(all.iter().any(|a| a == "--delimiter=\t"));
    }

    #[test]
    fn interpolated_paths_are_shell_quoted() {
        let odd = fzf_args(
            "/home/some one/bin/cs",
            Path::new("/tmp/state.json"),
            &Opts::default(),
        );
        let preview = odd.iter().find(|a| a.starts_with("--preview=")).unwrap();
        assert!(preview.contains("'/home/some one/bin/cs'"), "{preview}");
    }

    #[test]
    fn quote_survives_an_embedded_single_quote() {
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

}
