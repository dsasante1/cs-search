//! The parallel search engine.
//!
//! The shell version got its speed by having ripgrep pick files before jq parsed
//! them, but the two stages disagreed: rg matched *escaped JSON* while jq matched
//! *decoded text*, so patterns spanning an escape could be silently dropped. Here
//! the prefilter and the parser live in the same process, which is both faster
//! (a rejected line is never parsed at all, not merely a rejected file) and
//! fixable — see `might_match` for how the false negatives are closed.

use crate::cli::Opts;
use crate::output::{clip, squash, Row};
use crate::record::{BlockOpts, Record};
use regex::bytes::Regex as BytesRegex;
use regex::Regex;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use walkdir::WalkDir;

pub fn claude_home() -> PathBuf {
    match std::env::var_os("CLAUDE_HOME") {
        Some(h) => PathBuf::from(h),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude"),
    }
}

pub fn projects_dir() -> PathBuf {
    claude_home().join("projects")
}

pub fn transcripts() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = WalkDir::new(projects_dir())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    v.sort();
    v
}

/// Position assertions match relative to the haystack, so they mean one thing
/// against a raw JSON line and something else entirely against a decoded text
/// line — `^SELECT` can never match raw JSON, which starts with `{`. The raw
/// prefilter is therefore unsound for such patterns and gets switched off; those
/// searches decode every record instead. Detection is deliberately conservative:
/// a `$` inside a character class costs speed, never correctness.
fn has_positional_assertion(pat: &str) -> bool {
    if pat.contains('^') || pat.contains('$') {
        return true;
    }
    let b = pat.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\\' {
            if matches!(b[i + 1], b'b' | b'B' | b'A' | b'z' | b'Z' | b'<' | b'>') {
                return true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

/// Could this raw JSON line contain a match once decoded?
///
/// Matching the raw bytes is sound only where the encoded and decoded text agree.
/// They diverge at escape sequences, so any line carrying an escape *other than*
/// `\n` is decoded rather than rejected. `\n` is exempt because decoded text is
/// searched line by line anyway: a pattern is never allowed to span one, so a
/// `\n` boundary can't hide a match. The result is no false negatives, and in
/// practice almost every line is still resolved by the cheap path.
fn might_match(raw: &[u8], pre: Option<&BytesRegex>) -> bool {
    let Some(pre) = pre else {
        return true; // prefilter disabled: every record is decoded
    };
    if pre.is_match(raw) {
        return true;
    }
    let mut i = 0;
    while i + 1 < raw.len() {
        if raw[i] == b'\\' {
            if raw[i + 1] != b'n' {
                return true;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    false
}

pub struct Hits {
    pub rows: Vec<Row>,
    pub files: Vec<PathBuf>,
}

struct Ctx<'a> {
    re: &'a Regex,
    pre: Option<&'a BytesRegex>,
    opts: &'a Opts,
    blocks: BlockOpts,
}

pub fn search(opts: &Opts, re: &Regex) -> Hits {
    // `CS_NO_PREFILTER=1` forces the slow, unconditionally-correct path; the test
    // suite uses it to check the prefilter never drops a result.
    let disabled = has_positional_assertion(&opts.pattern)
        || std::env::var_os("CS_NO_PREFILTER").is_some();
    let pre = (!disabled)
        .then(|| BytesRegex::new(re.as_str()).expect("prefilter mirrors the main pattern"));
    let ctx = Ctx {
        re,
        pre: pre.as_ref(),
        opts,
        blocks: BlockOpts {
            thinking: opts.thinking,
            tools: opts.tools,
        },
    };

    let queue = Arc::new(Mutex::new(transcripts()));
    let out: Arc<Mutex<Hits>> = Arc::new(Mutex::new(Hits {
        rows: Vec::new(),
        files: Vec::new(),
    }));

    std::thread::scope(|s| {
        for _ in 0..opts.jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            let ctx = &ctx;
            s.spawn(move || {
                let mut rows = Vec::new();
                let mut files = Vec::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };
                    let before = rows.len();
                    scan_file(&path, ctx, &mut rows);
                    if rows.len() > before {
                        files.push(path);
                    }
                }
                let mut o = out.lock().unwrap();
                o.rows.append(&mut rows);
                o.files.append(&mut files);
            });
        }
    });

    let mut hits = Arc::try_unwrap(out).ok().unwrap().into_inner().unwrap();
    hits.rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    hits.files.sort();
    hits
}

fn scan_file(path: &Path, ctx: &Ctx, rows: &mut Vec<Row>) {
    let Ok(fh) = File::open(path) else { return };
    let mut reader = BufReader::with_capacity(1 << 20, fh);
    let mut buf = Vec::with_capacity(1 << 16);

    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if !might_match(&buf, ctx.pre) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        emit(&v, ctx, rows);
    }
}

fn emit(v: &Value, ctx: &Ctx, rows: &mut Vec<Row>) {
    let r = Record::new(v);
    let o = ctx.opts;

    if !r.is_conversation() || r.is_meta() {
        return;
    }
    if !o.role.is_empty() && r.kind() != o.role {
        return;
    }
    if o.no_sub && r.is_sidechain() {
        return;
    }
    if !o.since.is_empty() && r.timestamp() < o.since.as_str() {
        return;
    }
    if !o.project.is_empty() && !r.cwd().to_lowercase().contains(&o.project) {
        return;
    }

    let ts = crate::record::take_chars(r.timestamp(), 16).replacen('T', " ", 1);
    let project = r.cwd().rsplit('/').next().unwrap_or("?");
    let project = if project.is_empty() { "?" } else { project };
    let role = crate::record::take_chars(r.kind(), 4).to_owned();
    let sid = crate::record::take_chars(r.session_id(), 8).to_owned();

    for block in r.blocks(ctx.blocks) {
        for line in block.split('\n') {
            if !ctx.re.is_match(line) {
                continue;
            }
            rows.push(Row {
                ts: ts.clone(),
                project: project.to_owned(),
                role: role.clone(),
                sid: sid.clone(),
                text: clip(&squash(line), o.chars),
            });
        }
    }
}
