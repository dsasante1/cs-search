//! `cs -p` — search only your own prompts via `~/.claude/history.jsonl`.
//!
//! A single small file with one record per prompt, so this stays sequential and
//! finishes in milliseconds regardless of how large the transcripts get.

use crate::cli::Opts;
use crate::output::{squash, Row};
use crate::record::take_chars;
use crate::scan::claude_home;
use chrono::{DateTime, Local};
use regex::Regex;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

pub fn history_path() -> PathBuf {
    claude_home().join("history.jsonl")
}

pub fn run(opts: &Opts, re: &Regex) -> Result<Vec<Row>, String> {
    let path = history_path();
    let fh = File::open(&path).map_err(|_| format!("no {}", path.display()))?;
    let mut rows = Vec::new();

    for line in BufReader::new(fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let display = v.get("display").and_then(Value::as_str).unwrap_or("");
        if !re.is_match(display) {
            continue;
        }
        let project = v.get("project").and_then(Value::as_str).unwrap_or("");
        if !opts.project.is_empty() && !project.to_lowercase().contains(&opts.project) {
            continue;
        }
        // Rendered before it is filtered on, because history.jsonl stores epoch
        // milliseconds where the transcripts carry an ISO string -- there is no
        // date here to compare until one has been formatted.
        let ts = stamp(&v);
        if !after_since(&ts, &opts.since) {
            continue;
        }
        let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");

        rows.push(Row {
            ts,
            project: {
                let p = project.rsplit('/').next().unwrap_or("?");
                if p.is_empty() { "?" } else { p }.to_owned()
            },
            role: "you".to_owned(),
            sid: take_chars(sid, 8).to_owned(),
            text: take_chars(&squash(display), opts.chars).to_owned(),
            ..Default::default()
        });
    }

    rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(rows)
}

/// The prompt's time, in the local zone, as the row will print it.
fn stamp(v: &Value) -> String {
    v.get("timestamp")
        .and_then(Value::as_i64)
        .and_then(DateTime::from_timestamp_millis)
        .map(|dt| {
            dt.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Whether a prompt clears `--since`.
///
/// The cutoff is compared against the timestamp *as printed*, so every row the
/// filter keeps is one whose visible date is on or after it — filtering in a
/// different zone from the one being displayed would hide rows the user can see
/// are in range. `scan` applies the same rule to its own printed timestamp;
/// there it happens to be the raw UTC string, so the comparison looks direct.
///
/// A record with no usable timestamp cannot clear a cutoff, so it drops out
/// while `--since` is on, exactly as it does on the transcript path.
fn after_since(ts: &str, since: &str) -> bool {
    since.is_empty() || ts >= since
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cutoff_keeps_everything() {
        assert!(after_since("2026-06-21 00:00", ""));
        assert!(after_since("", ""));
    }

    /// A date-only cutoff has to keep the whole of its own day, and the printed
    /// stamp carries a time the cutoff does not: the comparison must not read
    /// the extra " 00:00" as putting the row before the date it names.
    #[test]
    fn the_cutoff_day_is_kept_whole() {
        assert!(after_since("2026-06-21 00:00", "2026-06-21"));
        assert!(after_since("2026-06-21 23:59", "2026-06-21"));
    }

    #[test]
    fn earlier_days_are_dropped() {
        assert!(!after_since("2026-06-20 23:59", "2026-06-21"));
        assert!(!after_since("2025-12-31 12:00", "2026-06-21"));
    }

    #[test]
    fn later_days_are_kept() {
        assert!(after_since("2026-06-22 03:46", "2026-06-21"));
    }

    /// Nothing checks that `--since` is a bare date, so a cutoff carrying a time
    /// can equal a stamp exactly. A cutoff includes the instant it names, here
    /// as on the transcript path.
    #[test]
    fn a_cutoff_includes_the_instant_it_names() {
        assert!(after_since("2026-06-21 00:00", "2026-06-21 00:00"));
        assert!(!after_since("2026-06-20 23:59", "2026-06-21 00:00"));
    }

    /// An unparseable timestamp renders empty; with a cutoff on, that cannot be
    /// shown to be in range, so it is dropped rather than let through.
    #[test]
    fn an_undated_prompt_cannot_clear_a_cutoff() {
        assert!(!after_since("", "2026-06-21"));
    }
}
