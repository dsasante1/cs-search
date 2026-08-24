//! `cs files <pattern>` — which files the sessions touched, and when.
//!
//! A different axis from every other search here: the pattern is matched
//! against paths that were *acted on*, not against anything anyone said. The
//! filename is in the transcript either way, but only inside tool blocks, where
//! `-t` finds it flattened into a wall of JSON alongside whole file contents —
//! technically a hit, practically unreadable. Reading the block structurally
//! instead turns "when did I last touch settings/base.py, and in which session"
//! into an answerable question.

use crate::cli::Opts;
use crate::output::{fixed, CYAN, DIM, RESET};
use crate::record::Record;
use crate::{dates, scan};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// One tool call that named a file.
pub struct Touch {
    pub path: String,
    /// The session's working directory, so the path can be shown relative to
    /// the project it belongs to.
    pub cwd: String,
    pub project: String,
    pub branch: String,
    pub ts: String,
    pub sid: String,
}

/// A file, and everything the corpus knows about work on it.
pub struct FileHits {
    pub path: String,
    pub shown: String,
    pub project: String,
    pub branch: String,
    pub touches: usize,
    pub sessions: usize,
    /// The most recent touch: when, and the session to open to read about it.
    pub last: String,
    pub last_sid: String,
}

pub fn run(opts: &Opts, re: &Regex) -> Vec<FileHits> {
    let queue = Arc::new(Mutex::new(scan::transcripts()));
    let out: Arc<Mutex<Vec<Touch>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        for _ in 0..opts.jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            s.spawn(move || {
                let mut local = Vec::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };
                    collect(&path, opts, re, &mut local);
                }
                out.lock().unwrap().append(&mut local);
            });
        }
    });

    fold(Arc::try_unwrap(out).ok().unwrap().into_inner().unwrap())
}

fn collect(path: &Path, opts: &Opts, re: &Regex, out: &mut Vec<Touch>) {
    let Ok(fh) = File::open(path) else { return };
    // One label for the whole transcript: see `projects::label`.
    let project = crate::projects::label(path).unwrap_or_else(|| crate::projects::UNKNOWN.into());
    let mut reader = BufReader::with_capacity(1 << 20, fh);
    let mut buf = Vec::with_capacity(1 << 16);

    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // Nearly every line is neither a tool call nor about a file, and this
        // rejects those for the cost of a scan rather than a parse. Unlike the
        // search prefilter it needs no soundness argument: it tests for a JSON
        // key this program writes the name of, not for the user's pattern.
        if !contains(&buf, b"_path\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        if !opts.project.is_empty() && !r.cwd().to_lowercase().contains(&opts.project) {
            continue;
        }
        if !opts.branch.is_empty() && !r.git_branch().to_lowercase().contains(&opts.branch) {
            continue;
        }
        if !opts.since.is_empty() && r.timestamp() < opts.since.as_str() {
            continue;
        }
        if !opts.until.is_empty() && dates::day_of(r.timestamp()) > opts.until.as_str() {
            continue;
        }
        for path in paths_in(&v) {
            if !re.is_match(&path) {
                continue;
            }
            out.push(Touch {
                path,
                cwd: r.cwd().to_owned(),
                project: project.clone(),
                branch: r.git_branch().to_owned(),
                ts: crate::record::take_chars(r.timestamp(), 16).replacen('T', " ", 1),
                sid: crate::record::take_chars(r.session_id(), 8).to_owned(),
            });
        }
    }
}

/// Every file named by a tool call in this record.
///
/// Read by key rather than by tool name: `file_path` and `notebook_path` are
/// what the tools agree on, and keying off the names of the tools themselves
/// would quietly stop seeing whichever one is added next.
pub fn paths_in(v: &Value) -> Vec<String> {
    let Some(Value::Array(items)) = v.pointer("/message/content") else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|b| b.get("input"))
        .flat_map(|input| {
            ["file_path", "notebook_path"]
                .iter()
                .filter_map(|k| input.get(k).and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// Touches folded into one entry per file, most recently worked on first.
fn fold(touches: Vec<Touch>) -> Vec<FileHits> {
    let mut by_path: HashMap<String, Vec<Touch>> = HashMap::new();
    for t in touches {
        by_path.entry(t.path.clone()).or_default().push(t);
    }

    let mut out: Vec<FileHits> = by_path
        .into_iter()
        .map(|(path, mut ts)| {
            ts.sort_by(|a, b| a.ts.cmp(&b.ts));
            let last = ts.last().expect("a group is never empty");
            let mut sids: Vec<&str> = ts.iter().map(|t| t.sid.as_str()).collect();
            sids.sort_unstable();
            sids.dedup();
            FileHits {
                shown: relative(&path, &last.cwd).to_owned(),
                project: last.project.clone(),
                branch: last.branch.clone(),
                last: last.ts.clone(),
                last_sid: last.sid.clone(),
                sessions: sids.len(),
                touches: ts.len(),
                path,
            }
        })
        .collect();
    // Newest first: "what have I been working on" is the question this answers,
    // with the path as tie-break so repeated runs agree.
    out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.path.cmp(&b.path)));
    out
}

/// A path shown relative to the project it was edited in. An absolute path
/// repeats the project name on every line and pushes the part that differs off
/// the right-hand edge.
fn relative<'a>(path: &'a str, cwd: &str) -> &'a str {
    if cwd.is_empty() {
        return path;
    }
    path.strip_prefix(cwd)
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
}

pub fn print(w: &mut impl Write, hits: &[FileHits], color: bool) {
    let (d, c, z) = if color { (DIM, CYAN, RESET) } else { ("", "", "") };
    let width = hits.iter().map(|h| h.project.chars().count()).max().unwrap_or(8).clamp(8, 20);

    for h in hits {
        let (parent, base) = split(&h.shown);
        let _ = writeln!(
            w,
            "{:>4}  {d}{}{z}  {c}{}{z}  {d}{}{z}  {d}{parent}{z}{}",
            h.touches,
            h.last,
            fixed(&h.project, width),
            h.last_sid,
            base,
        );
    }
}

pub fn print_json(w: &mut impl Write, hits: &[FileHits]) {
    for h in hits {
        let _ = writeln!(
            w,
            "{}",
            serde_json::json!({
                "path": h.path,
                "shown": h.shown,
                "project": h.project,
                "branch": h.branch,
                "touches": h.touches,
                "sessions": h.sessions,
                "last": h.last,
                "session": h.last_sid,
            })
        );
    }
}

/// `12 files · 87 touches · 9 sessions`, the orientation a long list does not
/// give on its own.
pub fn summary(hits: &[FileHits]) -> String {
    let touches: usize = hits.iter().map(|h| h.touches).sum();
    format!(
        "{} file{} · {touches} touch{} · {} session{}",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" },
        if touches == 1 { "" } else { "es" },
        hits.iter().map(|h| h.sessions).max().unwrap_or(0),
        if hits.iter().map(|h| h.sessions).max().unwrap_or(0) == 1 { "" } else { "s" },
    )
}

fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_path_is_shown_relative_to_the_project_it_lives_in() {
        assert_eq!(relative("/home/u/app/src/main.rs", "/home/u/app"), "src/main.rs");
    }

    /// A file outside the session's directory — /tmp, or another checkout —
    /// keeps its full path, because the relative form would be a lie.
    #[test]
    fn a_path_outside_the_project_stays_absolute() {
        assert_eq!(relative("/tmp/scratch.py", "/home/u/app"), "/tmp/scratch.py");
        assert_eq!(relative("/home/u/app/x", ""), "/home/u/app/x");
    }

    /// Editing the project directory itself leaves nothing to show, so the
    /// absolute path is kept rather than printing an empty column.
    #[test]
    fn the_project_directory_itself_is_not_reduced_to_nothing() {
        assert_eq!(relative("/home/u/app", "/home/u/app"), "/home/u/app");
    }

    #[test]
    fn tool_calls_naming_a_file_are_found_whatever_the_tool_is_called() {
        let v = json!({"message": {"content": [
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "/a/b.rs"}},
            {"type": "tool_use", "name": "SomeFutureTool", "input": {"file_path": "/c/d.rs"}},
            {"type": "tool_use", "name": "NotebookEdit", "input": {"notebook_path": "/e/f.ipynb"}},
        ]}});
        assert_eq!(paths_in(&v), ["/a/b.rs", "/c/d.rs", "/e/f.ipynb"]);
    }

    #[test]
    fn blocks_that_name_no_file_yield_none() {
        let v = json!({"message": {"content": [
            {"type": "text", "text": "the file_path is /a/b.rs"},
            {"type": "tool_use", "name": "Bash", "input": {"command": "ls"}},
        ]}});
        assert!(paths_in(&v).is_empty());
        assert!(paths_in(&json!({"message": {"content": "plain string"}})).is_empty());
        assert!(paths_in(&json!({})).is_empty());
    }

    fn touch(path: &str, ts: &str, sid: &str) -> Touch {
        Touch {
            path: path.into(),
            cwd: "/home/u/app".into(),
            project: "app".into(),
            branch: "main".into(),
            ts: ts.into(),
            sid: sid.into(),
        }
    }

    #[test]
    fn touches_fold_into_one_row_per_file() {
        let hits = fold(vec![
            touch("/home/u/app/a.rs", "2026-08-01 10:00", "aaaa"),
            touch("/home/u/app/a.rs", "2026-08-03 10:00", "bbbb"),
            touch("/home/u/app/a.rs", "2026-08-02 10:00", "aaaa"),
        ]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].touches, 3);
        assert_eq!(hits[0].sessions, 2, "two distinct sessions touched it");
        assert_eq!(hits[0].last, "2026-08-03 10:00", "the latest touch is the one shown");
        assert_eq!(hits[0].last_sid, "bbbb", "and it names the session to open");
        assert_eq!(hits[0].shown, "a.rs");
    }

    #[test]
    fn files_are_listed_most_recently_touched_first() {
        let hits = fold(vec![
            touch("/home/u/app/old.rs", "2026-08-01 10:00", "aaaa"),
            touch("/home/u/app/new.rs", "2026-08-09 10:00", "aaaa"),
        ]);
        let order: Vec<&str> = hits.iter().map(|h| h.shown.as_str()).collect();
        assert_eq!(order, ["new.rs", "old.rs"]);
    }

    #[test]
    fn the_summary_counts_files_and_touches_separately() {
        let hits = fold(vec![
            touch("/home/u/app/a.rs", "2026-08-01 10:00", "aaaa"),
            touch("/home/u/app/a.rs", "2026-08-02 10:00", "aaaa"),
            touch("/home/u/app/b.rs", "2026-08-02 10:00", "aaaa"),
        ]);
        let s = summary(&hits);
        assert!(s.starts_with("2 files · 3 touches"), "{s}");
    }
}
