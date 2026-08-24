//! `cs history <pattern>` — when a topic started, when it stopped, and where.
//!
//! A search answers "where was this said" and hands back every line of it. The
//! question underneath is often smaller than that: when did this first come up,
//! am I still on it, and which projects did it bleed into. All three are
//! already in the result set — this is the same search, counted rather than
//! listed, so it can never report something a search would not show you.
//!
//! Nothing here is inferred. "First" is the oldest matching line and "last" is
//! the newest, both of them lines somebody actually wrote.

use crate::output::Row;
use crate::stats::thousands;
use chrono::NaiveDate;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;

pub struct History {
    pub pattern: String,
    pub matches: usize,
    pub sessions: usize,
    /// Matches per project, most first.
    pub projects: Vec<(String, usize)>,
    pub first: String,
    pub first_sid: String,
    pub last: String,
    pub last_sid: String,
}

/// How many projects to name before folding the rest away.
const TOP: usize = 8;

/// Fold a result set into the shape above.
///
/// Rows arrive sorted by timestamp, so the ends of the range are the ends of
/// the slice.
pub fn summarize(pattern: &str, rows: &[Row]) -> History {
    let mut per_project: HashMap<&str, usize> = HashMap::new();
    for r in rows {
        *per_project.entry(r.project.as_str()).or_default() += 1;
    }
    let mut projects: Vec<(String, usize)> = per_project
        .into_iter()
        .map(|(p, n)| (p.to_owned(), n))
        .collect();
    projects.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut sids: Vec<&str> = rows.iter().map(|r| r.sid.as_str()).collect();
    sids.sort_unstable();
    sids.dedup();

    let first = rows.first();
    let last = rows.last();
    History {
        pattern: pattern.to_owned(),
        matches: rows.len(),
        sessions: sids.len(),
        projects,
        first: first.map(|r| r.ts.clone()).unwrap_or_default(),
        first_sid: first.map(|r| r.sid.clone()).unwrap_or_default(),
        last: last.map(|r| r.ts.clone()).unwrap_or_default(),
        last_sid: last.map(|r| r.sid.clone()).unwrap_or_default(),
    }
}

/// How long ago a day was, in words, or nothing if it cannot be worked out.
///
/// A bare date makes you do arithmetic to answer the question you actually
/// asked — is this still live, or did it stop months ago. A timestamp that does
/// not parse, or one in the future because a machine's clock disagreed, says
/// nothing rather than something wrong.
pub fn ago(ts: &str, today: NaiveDate) -> String {
    let Ok(day) = NaiveDate::parse_from_str(crate::dates::day_of(ts), "%Y-%m-%d") else {
        return String::new();
    };
    match (today - day).num_days() {
        d if d < 0 => String::new(),
        0 => "today".into(),
        1 => "yesterday".into(),
        d => format!("{d} days ago"),
    }
}

pub fn report(w: &mut impl Write, h: &History, today: NaiveDate) {
    let _ = writeln!(w, "'{}'", h.pattern);
    let _ = writeln!(w);
    for (label, ts, sid) in [
        ("first", &h.first, &h.first_sid),
        ("last", &h.last, &h.last_sid),
    ] {
        let when = ago(ts, today);
        let _ = writeln!(w, "  {label:<7} {ts}  {sid}  {when}");
    }
    let _ = writeln!(
        w,
        "\n{} {} · {} {} · {} {}",
        thousands(h.matches as u64),
        plural(h.matches, "match", "matches"),
        h.sessions,
        plural(h.sessions, "session", "sessions"),
        h.projects.len(),
        plural(h.projects.len(), "project", "projects"),
    );

    if h.projects.len() > 1 {
        let _ = writeln!(w, "\nPROJECTS");
        for (name, n) in h.projects.iter().take(TOP) {
            let _ = writeln!(w, "  {:>8}  {name}", thousands(*n as u64));
        }
        if h.projects.len() > TOP {
            let _ = writeln!(w, "  {:>8}  … {} more", "", h.projects.len() - TOP);
        }
    }
}

/// `sessions` is `Some` when `--sessions` was asked for: the breakdown rides
/// inside the one object rather than as a second, differently-shaped stream of
/// lines after it.
pub fn report_json(w: &mut impl Write, h: &History, sessions: Option<&[Row]>) {
    let projects: Vec<_> = h
        .projects
        .iter()
        .map(|(p, n)| json!({"project": p, "matches": n}))
        .collect();
    let mut out = json!({
            "pattern": h.pattern,
            "matches": h.matches,
            "sessions": h.sessions,
            "first": h.first,
            "first_session": h.first_sid,
            "last": h.last,
            "last_session": h.last_sid,
            "projects": projects,
    });
    if let Some(rows) = sessions {
        out["sessions"] = json!(crate::output::per_session(rows)
            .into_iter()
            .map(|(r, n)| json!({"session": r.sid, "project": r.project, "branch": r.branch,
                                 "ts": r.ts, "matches": n, "text": r.text}))
            .collect::<Vec<_>>());
    }
    let _ = writeln!(w, "{out}");
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ts: &str, project: &str, sid: &str) -> Row {
        Row {
            ts: ts.into(),
            project: project.into(),
            sid: sid.into(),
            role: "asst".into(),
            text: "matched line".into(),
            ..Default::default()
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_range_runs_from_the_oldest_match_to_the_newest() {
        let h = summarize(
            "celery",
            &[
                row("2026-05-12 14:03", "api", "aaaa1111"),
                row("2026-07-01 09:00", "api", "bbbb2222"),
                row("2026-08-19 09:41", "worker", "cccc3333"),
            ],
        );
        assert_eq!(h.first, "2026-05-12 14:03");
        assert_eq!(h.first_sid, "aaaa1111", "and names the session to open");
        assert_eq!(h.last, "2026-08-19 09:41");
        assert_eq!(h.last_sid, "cccc3333");
    }

    #[test]
    fn matches_sessions_and_projects_are_counted_separately() {
        let h = summarize(
            "celery",
            &[
                row("2026-05-12 14:03", "api", "aaaa1111"),
                row("2026-05-12 14:05", "api", "aaaa1111"),
                row("2026-08-19 09:41", "worker", "cccc3333"),
            ],
        );
        assert_eq!(h.matches, 3, "three lines");
        assert_eq!(h.sessions, 2, "in two sessions");
        assert_eq!(h.projects.len(), 2, "across two projects");
        assert_eq!(h.projects[0], ("api".to_owned(), 2), "busiest project first");
    }

    #[test]
    fn an_empty_result_summarises_to_nothing_rather_than_panicking() {
        let h = summarize("celery", &[]);
        assert_eq!(h.matches, 0);
        assert!(h.first.is_empty() && h.last.is_empty());
    }

    #[test]
    fn recency_is_said_in_words() {
        let today = day("2026-08-24");
        assert_eq!(ago("2026-08-24 09:00", today), "today");
        assert_eq!(ago("2026-08-23 09:00", today), "yesterday");
        assert_eq!(ago("2026-05-12 14:03", today), "104 days ago");
    }

    /// A stamp that does not parse, or one from a machine whose clock is ahead,
    /// says nothing at all rather than "-3 days ago".
    #[test]
    fn an_unreadable_or_future_stamp_says_nothing() {
        let today = day("2026-08-24");
        assert_eq!(ago("", today), "");
        assert_eq!(ago("not a date", today), "");
        assert_eq!(ago("2026-08-25 09:00", today), "");
    }

    #[test]
    fn a_single_project_needs_no_breakdown() {
        // The table would repeat the count printed one line above it.
        let h = summarize("celery", &[row("2026-05-12 14:03", "api", "aaaa1111")]);
        let mut out = Vec::new();
        report(&mut out, &h, day("2026-08-24"));
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("PROJECTS"), "{text}");
        assert!(text.contains("1 match · 1 session · 1 project"), "{text}");
    }
}
