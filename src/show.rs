//! `cs show <session-id>` — render one session as a readable transcript.

use crate::record::{stringify, take_chars, Record};
use crate::scan;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

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

pub fn run(id: &str) -> i32 {
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
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());

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
            let _ = write!(w, "\n=== {who} {ts} ===\n{text}\n");
        }
    }
    let _ = w.flush();
    0
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
