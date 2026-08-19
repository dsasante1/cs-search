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
        let sid = v.get("sessionId").and_then(Value::as_str).unwrap_or("");

        rows.push(Row {
            ts: v
                .get("timestamp")
                .and_then(Value::as_i64)
                .and_then(DateTime::from_timestamp_millis)
                .map(|dt| {
                    dt.with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default(),
            project: {
                let p = project.rsplit('/').next().unwrap_or("?");
                if p.is_empty() { "?" } else { p }.to_owned()
            },
            role: "you".to_owned(),
            sid: take_chars(sid, 8).to_owned(),
            text: take_chars(&squash(display), opts.chars).to_owned(),
        });
    }

    rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(rows)
}
