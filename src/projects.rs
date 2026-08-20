//! `cs projects` — the list `-P` expects.
//!
//! `-P` takes a substring of a session's working directory, which is only usable
//! if you already know what those directories are called. This prints them, with
//! enough context to pick one: how many sessions each holds and when it was last
//! touched.

use crate::output::{is_tty, CYAN, DIM, RESET};
use crate::record::Record;
use crate::scan;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// The cwd is on the first record of a healthy transcript; a handful of lines is
/// enough slack for one that starts with something else.
const SCAN_LINES: usize = 50;

pub struct Project {
    pub cwd: String,
    pub sessions: usize,
    pub last: SystemTime,
}

pub fn collect(jobs: usize) -> Vec<Project> {
    let queue = Arc::new(Mutex::new(scan::transcripts()));
    let found: Arc<Mutex<Vec<(String, SystemTime)>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        for _ in 0..jobs {
            let queue = Arc::clone(&queue);
            let found = Arc::clone(&found);
            s.spawn(move || {
                let mut local = Vec::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };
                    if let Some(entry) = describe(&path) {
                        local.push(entry);
                    }
                }
                found.lock().unwrap().append(&mut local);
            });
        }
    });

    let mut totals: HashMap<String, (usize, SystemTime)> = HashMap::new();
    for (cwd, mtime) in Arc::try_unwrap(found).ok().unwrap().into_inner().unwrap() {
        let e = totals.entry(cwd).or_insert((0, SystemTime::UNIX_EPOCH));
        e.0 += 1;
        e.1 = e.1.max(mtime);
    }

    let mut out: Vec<Project> = totals
        .into_iter()
        .map(|(cwd, (sessions, last))| Project { cwd, sessions, last })
        .collect();
    // Most recently used first, with the name as tie-break so repeated runs
    // agree even when two projects share an mtime.
    out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.cwd.cmp(&b.cwd)));
    out
}

fn describe(path: &Path) -> Option<(String, SystemTime)> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    let fh = File::open(path).ok()?;
    for line in BufReader::new(fh).lines().map_while(Result::ok).take(SCAN_LINES) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let cwd = Record::new(&v).cwd();
        if !cwd.is_empty() {
            return Some((cwd.to_owned(), mtime));
        }
    }
    None
}

pub fn run(filter: &str, jobs: usize) -> i32 {
    let needle = filter.to_lowercase();
    let projects: Vec<Project> = collect(jobs)
        .into_iter()
        .filter(|p| needle.is_empty() || p.cwd.to_lowercase().contains(&needle))
        .collect();

    if projects.is_empty() {
        eprintln!("no projects");
        return 1;
    }

    let (d, c, z) = if is_tty() { (DIM, CYAN, RESET) } else { ("", "", "") };
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for p in &projects {
        let when = DateTime::<Local>::from(p.last).format("%Y-%m-%d %H:%M");
        let (parent, base) = split(&p.cwd);
        let _ = writeln!(
            w,
            "{:>4}  {d}{when}{z}  {d}{parent}{z}{c}{base}{z}",
            p.sessions
        );
    }
    let _ = w.flush();
    0
}

/// Split a path into its leading directories and its final component, so the
/// name you actually type into `-P` can be picked out of the path.
fn split(cwd: &str) -> (&str, &str) {
    match cwd.rfind('/') {
        Some(i) => (&cwd[..=i], &cwd[i + 1..]),
        None => ("", cwd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_separates_the_directory_from_the_name() {
        assert_eq!(split("/home/u/alpha"), ("/home/u/", "alpha"));
        assert_eq!(split("alpha"), ("", "alpha"));
        assert_eq!(split("/alpha"), ("/", "alpha"));
        // A trailing slash leaves nothing to highlight, which is not a crash.
        assert_eq!(split("/home/u/"), ("/home/u/", ""));
    }
}
