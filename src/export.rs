//! `cs export <session-id>` — a session as a document rather than as a view.
//!
//! `show` renders for a terminal: ANSI, a rule sized to the window, a pager.
//! None of that survives being redirected into a file or attached to an issue,
//! which is what people actually do with a session worth keeping. This is the
//! same transcript with the terminal taken out of it.

use crate::show::{self, Turn, Who};
use serde_json::json;
use std::io::Write;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Markdown,
    Html,
    Json,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format, String> {
        match s {
            "md" | "markdown" => Ok(Format::Markdown),
            "html" => Ok(Format::Html),
            "json" => Ok(Format::Json),
            _ => Err(format!("--format must be md, html or json, got: {s}")),
        }
    }
}

pub fn write(w: &mut impl Write, turns: &[Turn], sid: &str, cwd: &str, format: Format) {
    match format {
        Format::Markdown => markdown(w, turns, sid, cwd),
        Format::Html => html(w, turns, sid, cwd),
        Format::Json => jsonl(w, turns, sid),
    }
}

fn markdown(w: &mut impl Write, turns: &[Turn], sid: &str, cwd: &str) {
    let _ = writeln!(w, "# Session {sid}");
    if !cwd.is_empty() {
        let _ = writeln!(w, "\n`{cwd}`");
    }
    if let (Some(first), Some(last)) = (turns.first(), turns.last()) {
        let _ = writeln!(w, "\n{} — {}", first.ts, last.ts);
    }
    for t in turns {
        // A heading per turn, so the document folds in an editor and the two
        // speakers stay apart without the rule the terminal view draws.
        let _ = writeln!(w, "\n## {} · {}\n", speaker(t.who), t.ts);
        let _ = writeln!(w, "{}", t.text);
    }
}

fn jsonl(w: &mut impl Write, turns: &[Turn], sid: &str) {
    for t in turns {
        let _ = writeln!(
            w,
            "{}",
            json!({"session": sid, "ts": t.ts, "role": t.who.name(), "text": t.text})
        );
    }
}

fn html(w: &mut impl Write, turns: &[Turn], sid: &str, cwd: &str) {
    let _ = writeln!(
        w,
        r#"<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Session {}</title>
<style>{STYLE}</style>
<body>
<header><h1>Session {}</h1><p class="cwd">{}</p></header>"#,
        escape(sid),
        escape(sid),
        escape(cwd)
    );
    for t in turns {
        let _ = writeln!(
            w,
            r#"<article class="{}"><h2>{} <time>{}</time></h2><pre>{}</pre></article>"#,
            match t.who {
                Who::You => "you",
                Who::Cc => "cc",
            },
            speaker(t.who),
            escape(&t.ts),
            escape(&t.text),
        );
    }
    let _ = writeln!(w, "</body>\n</html>");
}

/// Self-contained, and readable in either colour scheme: an exported
/// transcript is a file someone opens later, with no stylesheet beside it and
/// no say in how their browser is set.
const STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1a1a1a;--bg:#fff;--rule:#d8d8d8;--you:#0a6b7c;--cc:#7c3aad;--dim:#666}\
@media(prefers-color-scheme:dark){:root{--fg:#e8e8e8;--bg:#161616;--rule:#333;--you:#4bc6de;--cc:#c99bf0;--dim:#999}}\
body{max-width:52rem;margin:2rem auto;padding:0 1.25rem;background:var(--bg);color:var(--fg);\
font:16px/1.6 ui-sans-serif,-apple-system,Segoe UI,Roboto,sans-serif}\
h1{font-size:1.3rem;margin:0}\
.cwd{color:var(--dim);font:13px ui-monospace,SFMono-Regular,Menlo,monospace;margin:.35rem 0 0}\
header{border-bottom:1px solid var(--rule);padding-bottom:1rem;margin-bottom:.5rem}\
article{border-top:1px solid var(--rule);padding-top:1rem;margin-top:1rem}\
article:first-of-type{border-top:0}\
h2{font-size:.78rem;letter-spacing:.08em;text-transform:uppercase;margin:0 0 .6rem}\
.you h2{color:var(--you)}.cc h2{color:var(--cc)}\
time{color:var(--dim);font-weight:400;letter-spacing:0;text-transform:none}\
pre{white-space:pre-wrap;overflow-wrap:anywhere;margin:0;\
font:13.5px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace}";

fn speaker(who: Who) -> &'static str {
    match who {
        Who::You => "You",
        Who::Cc => "Claude",
    }
}

/// The five characters that would otherwise let transcript text close a tag or
/// an attribute. A transcript is full of HTML, JSX and shell quoting, so this
/// is the common case rather than the edge one.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `cs export <id> [--format md|html|json] [-r user|assistant]`.
pub fn run(id: &str, role: &str, format: Format) -> i32 {
    if id.is_empty() {
        eprintln!("cs export <session-id> [--format md|html|json]");
        return 2;
    }
    let Some(path) = show::pick(id, "exporting") else {
        return 1;
    };
    let Ok(fh) = std::fs::File::open(&path) else {
        eprintln!("cannot read {}", path.display());
        return 1;
    };

    let turns = show::turns(fh, role);
    if turns.is_empty() {
        eprintln!("nothing to export from '{id}'");
        return 1;
    }
    let sid = path.file_stem().and_then(|s| s.to_str()).unwrap_or(id);
    let cwd = show::session_cwd(&path).unwrap_or_default();

    let stdout = std::io::stdout();
    let mut w = std::io::BufWriter::new(stdout.lock());
    write(&mut w, &turns, sid, &cwd, format);
    let _ = w.flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns() -> Vec<Turn> {
        vec![
            Turn { who: Who::You, ts: "2026-08-03 16:31".into(), text: "run <it> & wait".into() },
            Turn { who: Who::Cc, ts: "2026-08-03 16:32".into(), text: "line one\nline two".into() },
        ]
    }

    fn rendered(f: Format) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write(&mut buf, &turns(), "abc123", "/home/u/app", f);
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn markdown_heads_each_turn_with_its_speaker() {
        let md = rendered(Format::Markdown);
        assert!(md.starts_with("# Session abc123"), "{md}");
        assert!(md.contains("## You · 2026-08-03 16:31"), "{md}");
        assert!(md.contains("## Claude · 2026-08-03 16:32"), "{md}");
        assert!(md.contains("/home/u/app"), "the project belongs in the header: {md}");
        // Markdown carries the text as written, angle brackets and all.
        assert!(md.contains("run <it> & wait"), "{md}");
    }

    #[test]
    fn json_is_one_object_per_turn() {
        let out = rendered(Format::Json);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["role"], "user");
        assert_eq!(first["session"], "abc123");
        assert_eq!(first["text"], "run <it> & wait");
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["role"], "assistant");
        // Newlines stay inside one turn rather than splitting it in two.
        assert_eq!(second["text"], "line one\nline two");
    }

    /// A transcript is full of markup; unescaped it would close the tag it
    /// sits in and the rest of the page with it.
    #[test]
    fn html_escapes_transcript_text() {
        let html = rendered(Format::Html);
        assert!(html.contains("run &lt;it&gt; &amp; wait"), "{html}");
        assert!(!html.contains("run <it>"), "{html}");
    }

    #[test]
    fn html_is_self_contained_and_labels_both_speakers() {
        let html = rendered(Format::Html);
        assert!(html.starts_with("<!doctype html>"), "{html}");
        assert!(html.contains("<style>"), "an exported file has no stylesheet beside it");
        assert!(!html.contains("http://") && !html.contains("https://"), "no external fetches");
        assert!(html.contains(r#"<article class="you">"#), "{html}");
        assert!(html.contains(r#"<article class="cc">"#), "{html}");
    }

    #[test]
    fn every_escaped_character_round_trips() {
        assert_eq!(
            escape(r#"<a href="x">&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn formats_are_named_the_way_people_write_them() {
        assert_eq!(Format::parse("md"), Ok(Format::Markdown));
        assert_eq!(Format::parse("markdown"), Ok(Format::Markdown));
        assert_eq!(Format::parse("html"), Ok(Format::Html));
        assert_eq!(Format::parse("json"), Ok(Format::Json));
        assert!(Format::parse("pdf").unwrap_err().contains("--format"));
    }
}
