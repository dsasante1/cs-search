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
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Only the head of a transcript is read — the opening prompt is near the top,
/// and some of these files are tens of megabytes.
const SCAN_LINES: usize = 400;
const SUMMARY_CHARS: usize = 88;

/// How much of the end of a transcript to read looking for its title.
///
/// Claude Code writes the title as its own record and rewrites it as the
/// session goes on, so the one worth having is the last, and the head scan
/// above will never reach it. Measured across a real corpus the final title
/// sits in the last 1-11% of the file; 64K covers that without the head scan's
/// saving being given back.
const TAIL_BYTES: u64 = 64 * 1024;

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
        // A generated title says what the session turned out to be about; an
        // opening prompt says only how it started, and "run it against staging
        // first" identifies nothing. The prompt stays as the fallback, since
        // short and abandoned sessions never get a title.
        let label = last_title(path).unwrap_or(text);
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
                text: take_chars(&squash(&label), SUMMARY_CHARS).to_owned(),
                ..Default::default()
            },
        ));
    }
    None
}

/// The session's own title, if it has been given one: the last `ai-title`
/// record in the file.
///
/// Read from the end rather than by scanning, so a 38 MB transcript costs the
/// same as a small one. Starting mid-file leaves a partial first line, which is
/// dropped rather than parsed.
fn last_title(path: &Path) -> Option<String> {
    let mut fh = File::open(path).ok()?;
    let len = fh.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL_BYTES);
    fh.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    fh.read_to_end(&mut buf).ok()?;
    Some(title_in(&buf, from > 0)).flatten()
}

/// The last title in a chunk of transcript. Split out from the file handling so
/// the partial-line rule can be tested without a corpus.
fn title_in(buf: &[u8], partial_first_line: bool) -> Option<String> {
    let mut lines = buf.split(|b| *b == b'\n');
    if partial_first_line {
        lines.next();
    }
    lines
        .filter(|l| find(l, b"\"ai-title\"").is_some())
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("ai-title"))
        .filter_map(|v| v.get("aiTitle").and_then(Value::as_str).map(str::to_owned))
        .rfind(|t| !t.trim().is_empty())
}

/// Whether `needle` occurs in `hay`. Only worth parsing the lines that could
/// possibly be titles, which on a busy transcript is a handful out of thousands.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
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

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: &[u8] = br#"{"type":"ai-title","aiTitle":"Search the history","sessionId":"a"}"#;

    #[test]
    fn a_session_without_a_title_has_none() {
        assert_eq!(title_in(br#"{"type":"user"}"#, false), None);
        assert_eq!(title_in(b"", false), None);
    }

    /// Titles are rewritten as the session goes on, so the last one is the one
    /// that describes what it became.
    #[test]
    fn the_last_title_wins() {
        let mut buf = TITLE.to_vec();
        buf.push(b'\n');
        buf.extend_from_slice(br#"{"type":"ai-title","aiTitle":"Search it faster","sessionId":"a"}"#);
        assert_eq!(title_in(&buf, false).as_deref(), Some("Search it faster"));
    }

    /// Reading from a byte offset lands mid-record; that fragment is not a
    /// title even if the bytes of one survive in it.
    #[test]
    fn a_partial_first_line_is_dropped() {
        let mut buf = TITLE.to_vec();
        buf.extend_from_slice(b"\n");
        buf.extend_from_slice(br#"{"type":"user"}"#);
        assert_eq!(title_in(&buf, true), None);
        assert_eq!(title_in(&buf, false).as_deref(), Some("Search the history"));
    }

    /// A record that merely mentions the field is not one: the cheap byte scan
    /// only decides what to parse, never what to believe.
    #[test]
    fn only_a_real_title_record_counts() {
        let line = br#"{"type":"user","message":{"content":"the \"ai-title\" record"}}"#;
        assert_eq!(title_in(line, false), None);
    }

    #[test]
    fn an_empty_title_falls_back_rather_than_blanking_the_row() {
        let line = br#"{"type":"ai-title","aiTitle":"   ","sessionId":"a"}"#;
        assert_eq!(title_in(line, false), None);
    }
}
