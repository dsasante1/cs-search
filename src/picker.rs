//! State shared between fzf's key bindings and the `cs` processes they spawn.
//!
//! fzf cannot hold variables of its own, so a binding that toggles a filter has
//! to change something outside the process it will re-read on the next reload.
//! That something is a small JSON file: `alt-t` runs `cs __toggle <file> tools`
//! and then re-runs `cs __rows <file> <query>`, which reads the file back. The
//! file lives for exactly as long as the picker does.

use crate::cli::Opts;
use crate::output::{fixed, pad, Row, CYAN, DIM, MAGENTA, RESET};
use crate::{prompts, scan};
use serde_json::{json, Value};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Live search re-runs on every keystroke, so a one-character query would scan
/// the whole corpus for something that matches nearly everything.
const MIN_QUERY: usize = 2;

pub fn state_path() -> PathBuf {
    std::env::temp_dir().join(format!("cs-picker-{}.json", std::process::id()))
}

pub fn save(path: &Path, o: &Opts) {
    let v = json!({
        "project": o.project,
        "role": o.role,
        "since": o.since,
        "until": o.until,
        "branch": o.branch,
        "tools": o.tools,
        "thinking": o.thinking,
        "no_sub": o.no_sub,
        "prompts": o.prompts,
        "fixed": o.fixed,
        "chars": o.chars,
        "jobs": o.jobs,
    });
    let _ = std::fs::write(path, v.to_string());
}

/// A missing or malformed state file yields defaults rather than an error: the
/// picker should degrade to an unfiltered search, never fail to draw.
pub fn load(path: &Path) -> Opts {
    let v: Value = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let d = Opts::default();
    let s = |k: &str, fallback: String| {
        v.get(k).and_then(Value::as_str).map(str::to_owned).unwrap_or(fallback)
    };
    let b = |k: &str, fallback: bool| v.get(k).and_then(Value::as_bool).unwrap_or(fallback);
    let n = |k: &str, fallback: usize| {
        v.get(k).and_then(Value::as_u64).map(|x| x as usize).unwrap_or(fallback)
    };
    Opts {
        project: s("project", d.project),
        role: s("role", d.role),
        since: s("since", d.since),
        until: s("until", d.until),
        branch: s("branch", d.branch),
        tools: b("tools", d.tools),
        thinking: b("thinking", d.thinking),
        no_sub: b("no_sub", d.no_sub),
        prompts: b("prompts", d.prompts),
        fixed: b("fixed", d.fixed),
        chars: n("chars", d.chars),
        jobs: n("jobs", d.jobs),
        ..Opts::default()
    }
}

/// Flip one filter. `value` carries the argument for the filters that take one
/// (currently the project under the cursor); the rest ignore it.
pub fn toggle(path: &Path, field: &str, value: &str) {
    let mut o = load(path);
    match field {
        "tools" => o.tools = !o.tools,
        "thinking" => o.thinking = !o.thinking,
        "sub" => o.no_sub = !o.no_sub,
        "fixed" => o.fixed = !o.fixed,
        // Pressing it again on the same project clears the filter, so one key
        // both narrows and widens.
        "project" => {
            let v = value.trim().to_lowercase();
            o.project = if o.project == v { String::new() } else { v };
        }
        "role" => {
            o.role = match o.role.as_str() {
                "" => "user".into(),
                "user" => "assistant".into(),
                _ => String::new(),
            }
        }
        "clear" => {
            let keep_prompts = o.prompts;
            o = Opts { prompts: keep_prompts, ..Opts::default() };
        }
        _ => {}
    }
    save(path, &o);
}

/// Two lines: what the keys do, and what is currently narrowing the search.
///
/// Only filters that are actually ON get printed. Listing all six every time —
/// `tools:off thinking:on subagents:on role:any project:any since:any` — meant
/// the default state was the noisiest thing on screen and said nothing. Now the
/// line is empty until you narrow something, so its presence is the signal.
pub fn header(path: &Path, query: &str) -> String {
    let o = load(path);
    let scope = if o.prompts { "your prompts" } else { "all text" };
    let mut active: Vec<String> = Vec::new();
    if o.tools {
        active.push("+tools".into());
    }
    if !o.thinking {
        active.push("-thinking".into());
    }
    if o.no_sub {
        active.push("-subagents".into());
    }
    if o.fixed {
        active.push("literal".into());
    }
    for (name, value) in [
        ("role", &o.role),
        ("project", &o.project),
        ("branch", &o.branch),
        ("since", &o.since),
        ("until", &o.until),
    ] {
        if !value.is_empty() {
            active.push(format!("{name}:{value}"));
        }
    }

    format!(
        "{DIM}enter open · alt-enter resume · alt-t tools · alt-h thinking · \
         alt-s subagents · alt-r role · alt-p this project · alt-c clear filters{RESET}\n\
         {CYAN}{scope}{RESET}{}{}",
        if active.is_empty() {
            String::new()
        } else {
            format!("  {}", active.join("  "))
        },
        if query.chars().count() < MIN_QUERY {
            format!("\n{DIM}type at least {MIN_QUERY} characters{RESET}")
        } else {
            String::new()
        },
    )
}

/// Print the rows fzf should display for `query`, under the saved filters.
///
/// The session id and project ride in hidden leading columns (`--with-nth=3..`)
/// so the preview command and the project-filter binding can read them back.
pub fn rows(path: &Path, query: &str) -> i32 {
    if query.chars().count() < MIN_QUERY {
        return 0;
    }
    let mut o = load(path);
    o.pattern = query.to_owned();

    let Ok((re, _)) = scan::compile(&o) else {
        return 0; // an unusable query mid-typing is not an error, just no rows
    };
    let rows = if o.prompts {
        prompts::run(&o, &re).unwrap_or_default()
    } else {
        scan::search(&o, &re).rows
    };
    write_rows(&rows, Some(&re));
    0
}

pub fn write_rows(rows: &[Row], hl: Option<&regex::Regex>) {
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    render_rows(&mut w, rows, hl);
    let _ = w.flush();
}

/// The wire format between `cs` and fzf: two hidden columns, then the line the
/// user actually sees.
pub fn render_rows(w: &mut impl Write, rows: &[Row], hl: Option<&regex::Regex>) {
    let width = crate::output::project_width(rows);
    for r in rows {
        let text = match hl {
            Some(re) => crate::output::highlight(&r.text, re),
            None => r.text.clone(),
        };
        let _ = writeln!(
            w,
            "{}\t{}\t{DIM}{}{RESET} {CYAN}{}{RESET} {MAGENTA}{}{RESET}  {text}",
            r.sid,
            r.project,
            r.ts,
            fixed(&r.project, width),
            pad(&r.role, 4),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cs-picker-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn state_survives_a_round_trip() {
        let p = scratch("roundtrip");
        let o = Opts {
            project: "dashqard".into(),
            role: "user".into(),
            since: "2026-07-01".into(),
            tools: true,
            thinking: false,
            no_sub: true,
            prompts: true,
            chars: 60,
            ..Default::default()
        };
        save(&p, &o);
        let back = load(&p);
        assert_eq!(back.project, "dashqard");
        assert_eq!(back.role, "user");
        assert_eq!(back.since, "2026-07-01");
        assert!(back.tools && !back.thinking && back.no_sub && back.prompts);
        assert_eq!(back.chars, 60);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_missing_state_file_loads_defaults() {
        let o = load(Path::new("/nonexistent/cs-picker-state.json"));
        assert!(o.thinking && !o.tools, "defaults, not zeroes");
        assert_eq!(o.chars, Opts::default().chars);
    }

    #[test]
    fn a_corrupt_state_file_loads_defaults() {
        let p = scratch("corrupt");
        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(load(&p).chars, Opts::default().chars);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn boolean_filters_flip() {
        let p = scratch("flip");
        save(&p, &Opts::default());
        toggle(&p, "tools", "");
        assert!(load(&p).tools);
        toggle(&p, "tools", "");
        assert!(!load(&p).tools, "the same key switches it back off");

        toggle(&p, "thinking", "");
        assert!(!load(&p).thinking);
        toggle(&p, "sub", "");
        assert!(load(&p).no_sub);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn role_cycles_through_both_speakers_and_back_to_any() {
        let p = scratch("role");
        save(&p, &Opts::default());
        for want in ["user", "assistant", ""] {
            toggle(&p, "role", "");
            assert_eq!(load(&p).role, want);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_project_key_narrows_then_widens() {
        let p = scratch("project");
        save(&p, &Opts::default());
        toggle(&p, "project", "DashQard");
        assert_eq!(load(&p).project, "dashqard", "stored lowercased, as -P is");
        toggle(&p, "project", "DashQard");
        assert_eq!(load(&p).project, "", "the same project again clears it");
        toggle(&p, "project", "alpha");
        toggle(&p, "project", "beta");
        assert_eq!(load(&p).project, "beta", "a different project replaces it");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn clear_resets_filters_but_stays_in_prompts_mode() {
        let p = scratch("clear");
        save(&p, &Opts { prompts: true, tools: true, project: "x".into(), ..Default::default() });
        toggle(&p, "clear", "");
        let o = load(&p);
        assert!(!o.tools && o.project.is_empty(), "filters are reset");
        assert!(o.prompts, "-p is what you are searching, not a filter over it");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_header_names_only_what_is_switched_on() {
        let p = scratch("header");
        save(&p, &Opts { tools: true, role: "user".into(), ..Default::default() });
        let h = header(&p, "needle");
        // The key legend always names every key; it is the state line below it
        // that has to stay quiet, so assert against that line alone.
        let state = h.lines().nth(1).unwrap();
        assert!(state.contains("+tools"), "{state}");
        assert!(state.contains("role:user"), "{state}");
        assert!(!state.contains("project:"), "an unset filter says nothing: {state}");
        assert!(!state.contains("thinking"), "a default-on filter says nothing: {state}");
        assert!(h.contains("alt-t"), "the keys stay on screen: {h}");
        assert_eq!(h.lines().count(), 2);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn an_unfiltered_search_says_nothing_beyond_its_scope() {
        // The default state used to be six "off"/"any" readings. Its whole value
        // is being quiet until something is actually narrowing the search.
        let p = scratch("quiet");
        save(&p, &Opts::default());
        let h = header(&p, "needle");
        let state = h.lines().nth(1).unwrap();
        assert!(state.contains("all text"), "{state}");
        assert!(!state.contains(':'), "nothing is on, so nothing is listed: {state}");
    }

    #[test]
    fn turning_a_filter_off_again_removes_it_from_the_header() {
        let p = scratch("header-toggle");
        save(&p, &Opts::default());
        toggle(&p, "tools", "");
        assert!(header(&p, "needle").contains("+tools"));
        toggle(&p, "tools", "");
        assert!(!header(&p, "needle").contains("+tools"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_header_distinguishes_a_default_from_its_opposite() {
        // thinking is on by default, so only its absence is worth a word.
        let p = scratch("header-default");
        save(&p, &Opts { thinking: false, no_sub: true, ..Default::default() });
        let h = header(&p, "needle");
        assert!(h.contains("-thinking") && h.contains("-subagents"), "{h}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_too_short_query_is_called_out_rather_than_silently_empty() {
        let p = scratch("short");
        save(&p, &Opts::default());
        assert!(header(&p, "a").contains("at least"));
        assert!(!header(&p, "ab").contains("at least"));
        let _ = std::fs::remove_file(&p);
    }
}
