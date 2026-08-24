//! `cs show <session-id>` — render one session as a readable transcript.
//!
//! Reading a transcript is where a search ends, so this does three things the
//! bare dump did not: it highlights whatever you searched for, it can open at
//! the first match instead of at the top of a session thousands of lines long,
//! and on a terminal it hands the result to a pager rather than letting it
//! scroll past.

use crate::output::{highlight, term_width, CYAN, DIM, MAGENTA, RESET};
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
    /// "user" or "assistant" to read one side of the conversation only;
    /// empty for both.
    pub role: String,
}

/// Who is speaking. The transcript's whole job is keeping the two apart, so the
/// speaker is modelled rather than baked into a formatted string.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Who {
    You,
    Cc,
}

impl Who {
    /// The name this speaker goes by outside the transcript view, where a
    /// three-character column is not the constraint.
    pub fn name(self) -> &'static str {
        match self {
            Who::You => "user",
            Who::Cc => "assistant",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Who::You => "YOU",
            Who::Cc => "CC ",
        }
    }

    /// Magenta is spent here and nowhere else. The search rows dropped it — next
    /// to a highlighted match a coloured role column is just noise — which frees
    /// it for the one place the speaker *is* the information.
    fn color(self) -> &'static str {
        match self {
            Who::You => CYAN,
            Who::Cc => MAGENTA,
        }
    }

    fn matches(self, role: &str) -> bool {
        match role {
            "user" => self == Who::You,
            "assistant" => self == Who::Cc,
            _ => true,
        }
    }
}

/// A transcript is turns and the lines inside them, not an undifferentiated
/// stream of strings: the divider has to be drawn, filtered and searched
/// differently from body text.
enum Chunk {
    Turn { who: Who, ts: String },
    Text(String),
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

    let chunks = transcript(fh, &o.role);
    if chunks.is_empty() {
        eprintln!("no {} turns in this session", speaker_name(&o.role));
        return 1;
    }
    let color = o.color || crate::output::is_tty();
    let (body, skipped, restated) = window(&chunks, o.at.as_ref());

    let mut pager = o.pager.then(open_pager).flatten();
    let code = {
        let sink: Box<dyn Write> = match pager.as_mut() {
            Some(p) => Box::new(p.stdin.take().expect("pager stdin was piped")),
            None => Box::new(std::io::stdout().lock()),
        };
        emit(sink, body, skipped, restated, o, color)
    };
    if let Some(mut p) = pager {
        let _ = p.wait();
    }
    code
}

fn emit(
    sink: Box<dyn Write>,
    body: &[Chunk],
    skipped: usize,
    restated: Option<&Chunk>,
    o: &ShowOpts,
    color: bool,
) -> i32 {
    let mut w = BufWriter::new(sink);
    let width = term_width();

    if skipped > 0 {
        let _ = writeln!(w, "{}↑ {skipped} earlier lines{}", dim(color), reset(color));
    }
    // Jumping to a match can land mid-turn, past the divider that said who was
    // talking. Restating it costs one line and answers the first question the
    // reader would otherwise have.
    if let Some(Chunk::Turn { who, ts }) = restated {
        let _ = writeln!(w, "{}", divider(*who, ts, width, color));
    }

    for chunk in body {
        let painted = match chunk {
            Chunk::Turn { who, ts } => divider(*who, ts, width, color),
            Chunk::Text(line) => match (&o.highlight, color) {
                (Some(re), true) => highlight(line, re),
                _ => line.clone(),
            },
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

/// The line that separates one speaker's turn from the next.
///
/// A rule spanning the full width, rather than the old `=== CC 12:00 ===`: three
/// equals signs either side read as decoration and left the two speakers running
/// together down the page. This actually cuts the page in two at every handover.
fn divider(who: Who, ts: &str, width: usize, color: bool) -> String {
    let label = format!("── {} {ts} ", who.label());
    let fill = width.saturating_sub(label.chars().count()).max(2);
    if color {
        format!(
            "{DIM}──{RESET} {}{}{RESET} {DIM}{ts} {}{RESET}",
            who.color(),
            who.label(),
            "─".repeat(fill),
        )
    } else {
        format!("{label}{}", "─".repeat(fill))
    }
}

fn speaker_name(role: &str) -> &str {
    match role {
        "user" => "your",
        "assistant" => "assistant",
        _ => "conversation",
    }
}

fn dim(color: bool) -> &'static str {
    if color { DIM } else { "" }
}

fn reset(color: bool) -> &'static str {
    if color { RESET } else { "" }
}

/// The slice to print, how many lines were skipped to get there, and the turn
/// heading to restate if the jump landed inside a turn already underway.
fn window<'a>(
    chunks: &'a [Chunk],
    at: Option<&Regex>,
) -> (&'a [Chunk], usize, Option<&'a Chunk>) {
    let Some(re) = at else {
        return (chunks, 0, None);
    };
    // Only body text can match: a divider is chrome this program drew, so
    // letting a pattern hit it would jump to an arbitrary turn.
    let hit = chunks
        .iter()
        .position(|c| matches!(c, Chunk::Text(t) if re.is_match(t)));
    let Some(i) = hit else {
        return (chunks, 0, None);
    };

    let from = i.saturating_sub(LEAD);
    let is_turn = |c: &&Chunk| matches!(c, Chunk::Turn { .. });
    let already_shown = chunks[from..=i].iter().any(|c| is_turn(&c));
    let restated = if already_shown {
        None
    } else {
        chunks[..from].iter().rev().find(is_turn)
    };
    (&chunks[from..], from, restated)
}

/// One speaker's turn: who, when, and what they said, unsplit.
///
/// The transcript view immediately breaks these into lines, because a divider
/// and a body line are drawn differently; `export` wants them whole. Reading
/// the file produces turns, and the view is built from them, so the two can
/// never disagree about what a session contains.
pub struct Turn {
    pub who: Who,
    pub ts: String,
    pub text: String,
}

/// Read a transcript as turns, honouring the same role filter `show` takes.
pub fn turns(fh: File, role: &str) -> Vec<Turn> {
    let mut out = Vec::new();
    for line in BufReader::with_capacity(1 << 20, fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        let who = if r.kind() == "user" { Who::You } else { Who::Cc };
        if !who.matches(role) {
            continue;
        }
        let ts = take_chars(r.timestamp(), 16).replacen('T', " ", 1);

        // Asking for "user" means what you typed. Tool results come back as
        // user-type records, so without this the filtered view is half machine
        // output attributed to you.
        for text in render_blocks(&r, role == "user") {
            if text.is_empty() {
                continue;
            }
            out.push(Turn { who, ts: ts.clone(), text });
        }
    }
    out
}

fn transcript(fh: File, role: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    for t in turns(fh, role) {
        out.push(Chunk::Text(String::new()));
        out.push(Chunk::Turn { who: t.who, ts: t.ts });
        out.extend(t.text.split('\n').map(|l| Chunk::Text(l.to_owned())));
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
/// `typed_only` drops the machine's half of a user turn; see the call site.
fn render_blocks(r: &Record, typed_only: bool) -> Vec<String> {
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
                    "tool_result" if !typed_only => {
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

    fn text(n: usize) -> Vec<Chunk> {
        (0..n).map(|i| Chunk::Text(format!("line {i}"))).collect()
    }

    /// A transcript shaped like the real thing: turns, each with body lines.
    fn conversation() -> Vec<Chunk> {
        let mut out = Vec::new();
        for (i, who) in [Who::You, Who::Cc, Who::You, Who::Cc].into_iter().enumerate() {
            out.push(Chunk::Turn { who, ts: format!("2026-08-03 10:0{i}") });
            for j in 0..20 {
                out.push(Chunk::Text(format!("turn {i} line {j}")));
            }
        }
        out
    }

    fn is_turn(c: &Chunk) -> bool {
        matches!(c, Chunk::Turn { .. })
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
        let all = text(20);
        let (body, skipped, restated) = window(&all, None);
        assert_eq!(body.len(), 20);
        assert_eq!(skipped, 0);
        assert!(restated.is_none());
    }

    #[test]
    fn jumping_keeps_a_few_lines_of_lead_in_above_the_match() {
        let all = text(500);
        let re = Regex::new("line 300").unwrap();
        let (body, skipped, _) = window(&all, Some(&re));
        assert_eq!(skipped, 300 - LEAD);
        assert!(matches!(&body[0], Chunk::Text(t) if t == &format!("line {}", 300 - LEAD)));
    }

    #[test]
    fn a_match_near_the_top_does_not_underflow() {
        let all = text(20);
        let re = Regex::new("line 1$").unwrap();
        let (body, skipped, _) = window(&all, Some(&re));
        assert_eq!(skipped, 0);
        assert_eq!(body.len(), 20);
    }

    #[test]
    fn an_unmatched_jump_pattern_falls_back_to_the_whole_transcript() {
        // The match may be inside a tool payload that `show` truncated away;
        // showing the session from the top beats showing nothing.
        let all = text(20);
        let re = Regex::new("nowhere").unwrap();
        let (body, skipped, restated) = window(&all, Some(&re));
        assert_eq!(body.len(), 20);
        assert_eq!(skipped, 0);
        assert!(restated.is_none());
    }

    #[test]
    fn a_divider_is_never_what_a_jump_lands_on() {
        // Dividers are chrome this program drew; matching them would jump to an
        // arbitrary turn rather than to the text the user searched for.
        let all = conversation();
        let re = Regex::new("YOU").unwrap();
        let (_, skipped, _) = window(&all, Some(&re));
        assert_eq!(skipped, 0, "a pattern matching only chrome should not jump");
    }

    #[test]
    fn landing_mid_turn_restates_who_is_speaking() {
        let all = conversation();
        // Deep inside the third turn, far past its heading.
        let re = Regex::new("turn 2 line 15").unwrap();
        let (body, _, restated) = window(&all, Some(&re));
        assert!(!body.iter().take(LEAD).any(is_turn), "the heading was cut off");
        match restated.expect("the turn heading should be restated") {
            Chunk::Turn { who, .. } => assert_eq!(*who, Who::You, "turn 2 is a user turn"),
            _ => panic!("restated chunk should be a turn heading"),
        }
    }

    #[test]
    fn landing_just_below_a_heading_does_not_repeat_it() {
        let all = conversation();
        let re = Regex::new("turn 1 line 1$").unwrap();
        let (body, _, restated) = window(&all, Some(&re));
        assert!(body.iter().take(LEAD + 1).any(is_turn), "the heading is in view");
        assert!(restated.is_none(), "restating it would print it twice");
    }

    #[test]
    fn the_divider_fills_the_terminal_width() {
        let plain = divider(Who::You, "2026-08-03 16:31", 60, false);
        assert_eq!(plain.chars().count(), 60);
        assert!(plain.starts_with("── YOU 2026-08-03 16:31 ─"));
        assert!(plain.ends_with('─'), "the rule runs to the edge: {plain}");
        assert!(!plain.contains('='), "the old === form is gone");
    }

    #[test]
    fn both_speakers_produce_the_same_width_rule() {
        // "CC " is padded so the two dividers line up down the page.
        let you = divider(Who::You, "2026-08-03 16:31", 72, false);
        let cc = divider(Who::Cc, "2026-08-03 16:31", 72, false);
        assert_eq!(you.chars().count(), cc.chars().count());
        assert!(cc.starts_with("── CC  2026-08-03 16:31 ─"));
    }

    #[test]
    fn a_narrow_pane_still_gets_a_rule_rather_than_a_wrapped_line() {
        // fzf's preview pane can be far narrower than the label.
        let squeezed = divider(Who::Cc, "2026-08-03 16:31", 4, false);
        assert!(squeezed.contains("CC"), "the speaker survives: {squeezed}");
        assert!(squeezed.ends_with("──"), "with a minimum stub of rule");
    }

    #[test]
    fn the_coloured_divider_measures_the_same_as_the_plain_one() {
        let plain = divider(Who::You, "2026-08-03 16:31", 70, false);
        let painted = divider(Who::You, "2026-08-03 16:31", 70, true);
        let stripped: String = painted
            .replace(DIM, "")
            .replace(CYAN, "")
            .replace(MAGENTA, "")
            .replace(RESET, "");
        assert_eq!(stripped, plain, "colour must not change the geometry");
    }

    #[test]
    fn each_speaker_gets_its_own_colour() {
        let you = divider(Who::You, "ts", 40, true);
        let cc = divider(Who::Cc, "ts", 40, true);
        assert!(you.contains(CYAN) && !you.contains(MAGENTA));
        assert!(cc.contains(MAGENTA) && !cc.contains(CYAN));
    }

    #[test]
    fn the_role_filter_selects_one_side_of_the_conversation() {
        assert!(Who::You.matches("user") && !Who::Cc.matches("user"));
        assert!(Who::Cc.matches("assistant") && !Who::You.matches("assistant"));
        // Anything else, including the empty default, keeps both.
        for both in ["", "anything"] {
            assert!(Who::You.matches(both) && Who::Cc.matches(both), "role={both:?}");
        }
    }
}
