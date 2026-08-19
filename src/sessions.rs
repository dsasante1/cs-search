//! `cs sessions [substr]` — list sessions newest-first with their opening prompt.
//!
//! This was the slowest command in the shell version: 253 sequential
//! `head | jq` pipelines. It is embarrassingly parallel, so it is parallel here.

use crate::output::{squash, Row};
use crate::record::{take_chars, Record};
use crate::scan;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Only the head of a transcript is read — the opening prompt is near the top,
/// and some of these files are tens of megabytes.
const SCAN_LINES: usize = 400;
const SUMMARY_CHARS: usize = 88;

pub fn run(filter: &str, jobs: usize) -> Vec<Row> {
    let queue = Arc::new(Mutex::new(scan::transcripts()));
    let out: Arc<Mutex<Vec<(SystemTime, PathBuf, Row)>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        for _ in 0..jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            s.spawn(move || {
                let mut local = Vec::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };
                    if let Some(entry) = first_prompt(&path) {
                        local.push(entry);
                    }
                }
                out.lock().unwrap().append(&mut local);
            });
        }
    });

    let mut rows = Arc::try_unwrap(out).ok().unwrap().into_inner().unwrap();
    // Newest first by file mtime, with the path as tie-break: workers finish in
    // arbitrary order, so mtime alone would make the listing nondeterministic.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let needle = filter.to_lowercase();
    rows.into_iter()
        .map(|(_, _, r)| r)
        .filter(|r| {
            needle.is_empty() || {
                let hay = format!("{} {} {} {} {}", r.ts, r.project, r.role, r.sid, r.text)
                    .to_lowercase();
                hay.contains(&needle)
            }
        })
        .collect()
}

fn first_prompt(path: &Path) -> Option<(SystemTime, PathBuf, Row)> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    let fh = File::open(path).ok()?;

    for line in BufReader::with_capacity(1 << 20, fh)
        .lines()
        .map_while(Result::ok)
        .take(SCAN_LINES)
    {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if r.kind() != "user" || r.is_meta() || r.is_sidechain() {
            continue;
        }
        let text = opening_text(&r);
        if text.trim().is_empty() {
            continue;
        }
        let project = r.cwd().rsplit('/').next().unwrap_or("?");
        return Some((
            mtime,
            path.to_path_buf(),
            Row {
                ts: DateTime::<Local>::from(mtime)
                    .format("%Y-%m-%d %H:%M")
                    .to_string(),
                project: if project.is_empty() { "?" } else { project }.to_owned(),
                role: "sess".to_owned(),
                sid: take_chars(r.session_id(), 8).to_owned(),
                text: take_chars(&squash(&text), SUMMARY_CHARS).to_owned(),
                ..Default::default()
            },
        ));
    }
    None
}

/// The opening prompt is the plain text of the first real user turn; tool and
/// thinking blocks are never part of it.
fn opening_text(r: &Record) -> String {
    match r.content() {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}
