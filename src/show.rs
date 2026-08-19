//! `cs show <session-id>` — render one session as a readable transcript.
//!
//! Reading a transcript is where a search ends, so this does three things the
//! bare dump did not: it highlights whatever you searched for, it can open at
//! the first match instead of at the top of a session thousands of lines long,
//! and on a terminal it hands the result to a pager rather than letting it
//! scroll past.

use crate::output::{highlight, DIM, RESET};
use crate::record::{stringify, take_chars, Record};
use crate::scan;
use regex::Regex;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Lines of lead-in kept above the match when opening with `--at`, so the jump
/// lands with some conversation above it rather than flush at the top.
const LEAD: usize = 4;

#[derive(Default)]
pub struct ShowOpts {
    /// Mark occurrences of this pattern in the transcript.
    pub highlight: Option<Regex>,
    /// Start output at the first line matching this pattern.
    pub at: Option<Regex>,
    /// Force ANSI even when stdout is not a terminal — fzf renders the preview
    /// itself, so the pipe there is not a reason to drop colour.
    pub color: bool,
    pub pager: bool,
}

/// Compile a pattern the same forgiving way the search does, treating an empty
/// one as "no pattern" rather than as a regex that matches everything. The
/// picker passes its live query straight through here, so it has to cope with
/// half-typed input on every keystroke.
pub fn pattern(q: &str) -> Option<Regex> {
    if q.trim().is_empty() {
        return None;
    }
    Regex::new(&format!("(?i){q}"))
        .or_else(|_| Regex::new(&format!("(?i){}", regex::escape(q))))
        .ok()
}

/// Session ids are matched by prefix so a short id from search output is enough.
pub fn resolve(id: &str) -> Vec<PathBuf> {
    scan::transcripts()
        .into_iter()
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(id))
        })
        .collect()
}

/// The working directory the session ran in, read from its first record.
pub fn session_cwd(path: &Path) -> Option<String> {
    let fh = File::open(path).ok()?;
    for line in BufReader::new(fh).lines().map_while(Result::ok).take(50) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let cwd = Record::new(&v).cwd();
        if !cwd.is_empty() {
            return Some(cwd.to_owned());
        }
    }
    None
}

pub fn run_with(id: &str, o: &ShowOpts) -> i32 {
    if id.is_empty() {
        eprintln!("cs show <session-id>");
        return 2;
    }
    let matches = resolve(id);
    let Some(path) = matches.first() else {
        eprintln!("no session matching '{id}'");
        return 1;
    };
    // The shell version silently took the first match; be explicit instead.
    if matches.len() > 1 {
        eprintln!("# {} sessions match '{id}', showing the first:", matches.len());
        for p in &matches {
            eprintln!("#   {}", p.display());
        }
    }
    eprintln!("# {}", path.display());

    let Ok(fh) = File::open(path) else {
        eprintln!("cannot read {}", path.display());
        return 1;
    };

    let lines = transcript(fh);
    let color = o.color || crate::output::is_tty();
    let (body, skipped) = window(&lines, o.at.as_ref());

    let mut pager = o.pager.then(open_pager).flatten();
    let code = {
        let sink: Box<dyn Write> = match pager.as_mut() {
            Some(p) => Box::new(p.stdin.take().expect("pager stdin was piped")),
            None => Box::new(std::io::stdout().lock()),
        };
        emit(sink, body, skipped, o, color)
    };
    if let Some(mut p) = pager {
        let _ = p.wait();
    }
    code
}

fn emit(sink: Box<dyn Write>, body: &[String], skipped: usize, o: &ShowOpts, color: bool) -> i32 {
    let mut w = BufWriter::new(sink);
    if skipped > 0 {
        let _ = writeln!(w, "{}↑ {skipped} earlier lines{}", dim(color), reset(color));
    }
    for line in body {
        let painted = match (&o.highlight, color) {
            (Some(re), true) => highlight(line, re),
            _ => line.clone(),
        };
        // A closed pager (someone quit `less` early) is an ordinary end, not a
        // failure, so stop writing rather than reporting a broken pipe.
        if writeln!(w, "{painted}").is_err() {
            break;
        }
    }
    let _ = w.flush();
    0
}

fn dim(color: bool) -> &'static str {
    if color { DIM } else { "" }
}

fn reset(color: bool) -> &'static str {
    if color { RESET } else { "" }
}

/// The slice to print, plus how many lines were skipped to get there.
fn window<'a>(lines: &'a [String], at: Option<&Regex>) -> (&'a [String], usize) {
    let Some(re) = at else {
        return (lines, 0);
    };
    match lines.iter().position(|l| re.is_match(l)) {
        Some(i) => {
            let from = i.saturating_sub(LEAD);
            (&lines[from..], from)
        }
        None => (lines, 0),
    }
}

fn transcript(fh: File) -> Vec<String> {
    let mut out = Vec::new();
    for line in BufReader::with_capacity(1 << 20, fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        let ts = take_chars(r.timestamp(), 16).replacen('T', " ", 1);
        let who = if r.kind() == "user" { "YOU " } else { "CC  " };

        for text in render_blocks(&r) {
            if text.is_empty() {
                continue;
            }
            out.push(String::new());
            out.push(format!("=== {who} {ts} ==="));
            out.extend(text.split('\n').map(str::to_owned));
        }
    }
    out
}

/// `$PAGER`, or `less` configured to keep colour and to not clear the screen on
/// exit. A short transcript quits immediately rather than trapping you in a
/// pager for four lines.
fn open_pager() -> Option<Child> {
    if !crate::output::is_tty() {
        return None;
    }
    let cmd = std::env::var("PAGER").unwrap_or_else(|_| "less".into());
    let mut parts = cmd.split_whitespace();
    let program = parts.next()?;
    let mut c = Command::new(program);
    c.args(parts);
    if program.ends_with("less") {
        c.args(["-R", "-F", "-X"]);
    }
    c.stdin(Stdio::piped()).spawn().ok()
}

/// Unlike search, `show` always includes thinking and tools — the point is to
/// read the whole session — but truncates tool payloads so they stay skimmable.
fn render_blocks(r: &Record) -> Vec<String> {
    const TOOL_CLIP: usize = 400;
    let Some(content) = r.content() else {
        return Vec::new();
    };
    match content {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|b| {
                let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
                match ty {
                    "text" => b.get("text").and_then(Value::as_str).map(str::to_owned),
                    "thinking" => b
                        .get("thinking")
                        .and_then(Value::as_str)
                        .map(|t| format!("[thinking] {t}")),
                    "tool_use" => {
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                        let input = b.get("input").map(stringify).unwrap_or_default();
                        Some(format!("[tool: {name}] {}", take_chars(&input, TOOL_CLIP)))
                    }
                    "tool_result" => {
                        let c = b.get("content").map(stringify).unwrap_or_default();
                        Some(format!("[result] {}", take_chars(&c, TOOL_CLIP)))
                    }
                    _ => None,
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    #[test]
    fn an_empty_pattern_is_no_pattern() {
        assert!(pattern("").is_none());
        assert!(pattern("   ").is_none());
    }

    #[test]
    fn a_half_typed_pattern_still_compiles_as_a_literal() {
        // The picker recompiles on every keystroke, so "useState(" is a state
        // the query passes through rather than an error.
        let re = pattern("useState(").expect("should fall back rather than give up");
        assert!(re.is_match("useState(0)"));
    }

    #[test]
    fn without_a_jump_pattern_the_whole_transcript_is_printed() {
        let all = lines(20);
        let (body, skipped) = window(&all, None);
        assert_eq!(body.len(), 20);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn jumping_keeps_a_few_lines_of_lead_in_above_the_match() {
        let all = lines(500);
        let re = Regex::new("line 300").unwrap();
        let (body, skipped) = window(&all, Some(&re));
        assert_eq!(skipped, 300 - LEAD);
        assert_eq!(body[0], format!("line {}", 300 - LEAD));
        assert!(body.iter().any(|l| l == "line 300"));
    }

    #[test]
    fn a_match_near_the_top_does_not_underflow() {
        let all = lines(20);
        let re = Regex::new("line 1$").unwrap();
        let (body, skipped) = window(&all, Some(&re));
        assert_eq!(skipped, 0);
        assert_eq!(body.len(), 20);
    }

    #[test]
    fn an_unmatched_jump_pattern_falls_back_to_the_whole_transcript() {
        // The match may be inside a tool payload that `show` truncated away;
        // showing the session from the top beats showing nothing.
        let all = lines(20);
        let re = Regex::new("nowhere").unwrap();
        let (body, skipped) = window(&all, Some(&re));
        assert_eq!(body.len(), 20);
        assert_eq!(skipped, 0);
    }
}
