//! `cs handoff <session-id>` — where a session left off.
//!
//! Coming back to work after a break, the questions are always the same: what
//! was this, how long did it run, which files did it touch, and what was said
//! last. All four are recorded, so all four are read rather than reconstructed.
//!
//! What is deliberately absent: "open threads", "the decision", "the next
//! step". Those cannot be had from a transcript without summarising it, and a
//! heuristic that guessed them would be wrong quietly — which is the one
//! failure this tool tries hardest not to have. The last turns are printed
//! verbatim instead, and the reading of them is yours.

use crate::cli::Opts;
use crate::output::{term_width, DIM, RESET};
use crate::record::Record;
use crate::show::{self, Turn};
use crate::stats::{self, short, thousands, Prices};
use chrono::DateTime;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// How many closing turns to print.
const TAIL: usize = 3;

/// How much of a long turn to print before cutting it. A tail is a reminder,
/// not the transcript; `cs show` is one keystroke away.
const TAIL_LINES: usize = 12;

/// How many files to list.
const FILES: usize = 10;

pub struct Session {
    pub sid: String,
    pub project: String,
    pub cwd: String,
    pub branches: Vec<String>,
    /// The raw first and last timestamps, kept whole so the span can be worked
    /// out from them.
    pub first: String,
    pub last: String,
    pub yours: usize,
    pub theirs: usize,
    /// Files the session acted on, most touched first.
    pub files: Vec<(usize, String)>,
}

/// Read everything about a session that is not its text.
pub fn scan(path: &Path) -> Session {
    let mut s = Session {
        sid: path.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_owned(),
        project: String::new(),
        cwd: String::new(),
        branches: Vec::new(),
        first: String::new(),
        last: String::new(),
        yours: 0,
        theirs: 0,
        files: Vec::new(),
    };
    let mut seen_branches: HashSet<String> = HashSet::new();
    let mut touches: HashMap<String, usize> = HashMap::new();

    let Ok(fh) = File::open(path) else { return s };
    for line in BufReader::with_capacity(1 << 20, fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        if s.cwd.is_empty() && !r.cwd().is_empty() {
            s.cwd = r.cwd().to_owned();
            let p = r.cwd().rsplit('/').next().unwrap_or("?");
            s.project = if p.is_empty() { "?" } else { p }.to_owned();
        }
        // Recorded per line, so a session that changed branch reports both, in
        // the order it worked on them.
        if !r.git_branch().is_empty() && seen_branches.insert(r.git_branch().to_owned()) {
            s.branches.push(r.git_branch().to_owned());
        }
        let ts = r.timestamp();
        if !ts.is_empty() {
            if s.first.is_empty() {
                s.first = ts.to_owned();
            }
            s.last = ts.to_owned();
        }
        if r.kind() == "user" {
            s.yours += 1;
        } else {
            s.theirs += 1;
        }
        for p in crate::files::paths_in(&v) {
            *touches.entry(p).or_default() += 1;
        }
    }

    let cwd = s.cwd.clone();
    let mut files: Vec<(usize, String)> = touches
        .into_iter()
        .map(|(path, n)| (n, relative(&path, &cwd)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    s.files = files;
    s
}

/// A path shown relative to the directory the session ran in.
fn relative(path: &str, cwd: &str) -> String {
    if cwd.is_empty() {
        return path.to_owned();
    }
    path.strip_prefix(cwd)
        .map(|rest| rest.strip_prefix('/').unwrap_or(rest))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// How long the session ran, in words.
///
/// Both ends are timestamps the transcript carries, so this is elapsed time
/// between the first and last thing said — not time spent working, which
/// nothing here records. A pair that will not parse yields nothing rather than
/// a made-up duration.
pub fn span(first: &str, last: &str) -> String {
    let at = |s: &str| DateTime::parse_from_rfc3339(s).ok();
    let (Some(a), Some(b)) = (at(first), at(last)) else {
        return String::new();
    };
    let mins = (b - a).num_minutes();
    if mins < 0 {
        return String::new();
    }
    let (d, h, m) = (mins / 1440, mins % 1440 / 60, mins % 60);
    match (d, h) {
        (0, 0) => format!("{m}m"),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

/// The closing turns, each cut to a readable length.
fn tail(turns: &[Turn]) -> Vec<(&Turn, String)> {
    turns
        .iter()
        .rev()
        .take(TAIL)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|t| {
            let lines: Vec<&str> = t.text.lines().collect();
            if lines.len() <= TAIL_LINES {
                return (t, t.text.clone());
            }
            (
                t,
                format!(
                    "{}\n… {} more lines",
                    lines[..TAIL_LINES].join("\n"),
                    lines.len() - TAIL_LINES
                ),
            )
        })
        .collect()
}

pub fn report(
    w: &mut impl Write,
    s: &Session,
    st: &stats::Stats,
    turns: &[Turn],
    prices: Option<&Prices>,
    color: bool,
) {
    let (d, z) = if color { (DIM, RESET) } else { ("", "") };
    let _ = writeln!(w, "SESSION  {}", s.sid);
    let field = |w: &mut dyn Write, k: &str, v: &str| {
        if !v.is_empty() {
            let _ = writeln!(w, "  {d}{k:<9}{z}{v}");
        }
    };
    field(w, "project", &format!("{}  {d}{}{z}", s.project, s.cwd));
    field(w, "branch", &s.branches.join(", "));

    let when = match span(&s.first, &s.last) {
        sp if sp.is_empty() => format!("{} → {}", stamp(&s.first), stamp(&s.last)),
        sp => format!("{} → {}  {d}({sp}){z}", stamp(&s.first), stamp(&s.last)),
    };
    field(w, "when", &when);
    field(
        w,
        "turns",
        &format!(
            "{} yours · {} assistant",
            thousands(s.yours as u64),
            thousands(s.theirs as u64)
        ),
    );

    let t = &st.tokens;
    if t.input + t.output + t.cache_read + t.cache_write > 0 {
        let rate = match t.cache_hit_rate() {
            Some(r) => format!("  {d}({:.1}% from cache){z}", r * 100.0),
            None => String::new(),
        };
        field(
            w,
            "tokens",
            &format!(
                "{} in · {} out · {} cached{rate}",
                short(t.input),
                short(t.output),
                short(t.cache_read)
            ),
        );
    }
    if let Some(prices) = prices {
        let (total, unpriced) = stats::cost(st, prices);
        let note = if unpriced.is_empty() {
            String::new()
        } else {
            format!("  {d}(not priced: {}){z}", unpriced.join(", "))
        };
        field(w, "cost", &format!("${total:.2}{note}"));
    }

    if !s.files.is_empty() {
        let _ = writeln!(w, "\nFILES");
        for (n, path) in s.files.iter().take(FILES) {
            let _ = writeln!(w, "  {n:>4}  {path}");
        }
        if s.files.len() > FILES {
            let _ = writeln!(w, "  {:>4}  … {} more", "", s.files.len() - FILES);
        }
    }

    let closing = tail(turns);
    if !closing.is_empty() {
        let _ = writeln!(w, "\nLAST TURNS");
        let width = term_width();
        for (t, text) in closing {
            // The same rule `show` draws, so the two views of a transcript look
            // like the same program.
            let _ = writeln!(w, "{}", show::divider(t.who, &t.ts, width, color));
            let _ = writeln!(w, "{text}");
        }
    }
}

/// A timestamp as the rest of the tool prints it: minutes, no zone marker.
fn stamp(ts: &str) -> String {
    crate::record::take_chars(ts, 16).replacen('T', " ", 1)
}

/// `cs handoff <id> [--prices <file>]`.
pub fn run(id: &str, prices: Option<&Prices>, jobs: usize) -> i32 {
    let Some(path) = show::pick(id, "reading") else {
        return 1;
    };
    let s = scan(&path);
    if s.yours + s.theirs == 0 {
        eprintln!("no conversation in '{id}'");
        return 1;
    }
    // Tokens and cost come from `stats` rather than from a second copy of the
    // arithmetic here, so a session's cost and the corpus's are always computed
    // the same way.
    // Deliberately unfiltered but for the session itself: a token total shaped
    // by a date range the rest of the report knows nothing about would be a
    // number that disagreed with the lines above it.
    let st = stats::collect(&Opts {
        session: s.sid.clone(),
        jobs,
        ..Opts::default()
    });
    // What was said, not what was run: see `show::turns_with`.
    let turns = File::open(&path)
        .map(|fh| show::turns_with(fh, "", crate::record::BlockOpts { thinking: false, tools: false }))
        .unwrap_or_default();

    let stdout = std::io::stdout();
    let mut w = std::io::BufWriter::new(stdout.lock());
    report(&mut w, &s, &st, &turns, prices, crate::output::is_tty());
    let _ = w.flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::Who;

    #[test]
    fn a_span_is_read_off_the_two_ends() {
        assert_eq!(span("2026-08-20T01:00:00Z", "2026-08-20T01:43:00Z"), "43m");
        assert_eq!(span("2026-08-20T01:00:00Z", "2026-08-20T03:43:00Z"), "2h 43m");
        assert_eq!(span("2026-08-20T01:00:00Z", "2026-08-23T05:00:00Z"), "3d 4h");
    }

    #[test]
    fn a_session_of_one_message_has_no_span_worth_naming() {
        assert_eq!(span("2026-08-20T01:00:00Z", "2026-08-20T01:00:00Z"), "0m");
    }

    /// Rather than invent one from stamps that cannot be compared.
    #[test]
    fn unreadable_or_reversed_stamps_yield_no_span() {
        assert_eq!(span("", ""), "");
        assert_eq!(span("2026-08-20", "2026-08-21"), "", "not RFC 3339");
        assert_eq!(span("2026-08-21T01:00:00Z", "2026-08-20T01:00:00Z"), "");
    }

    fn turn(text: &str) -> Turn {
        Turn { who: Who::Cc, ts: "2026-08-20 01:00".into(), text: text.into() }
    }

    #[test]
    fn the_tail_is_the_last_few_turns_in_the_order_they_happened() {
        let turns: Vec<Turn> = ["first", "second", "third", "fourth"]
            .iter()
            .map(|t| turn(t))
            .collect();
        let got: Vec<&str> = tail(&turns).iter().map(|(t, _)| t.text.as_str()).collect();
        assert_eq!(got, ["second", "third", "fourth"]);
    }

    #[test]
    fn a_short_session_yields_what_it_has() {
        assert_eq!(tail(&[turn("only")]).len(), 1);
        assert!(tail(&[]).is_empty());
    }

    #[test]
    fn a_long_turn_is_cut_and_says_how_much_was_cut() {
        let turns = [turn(&(1..=20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n"))];
        let (_, text) = &tail(&turns)[0];
        assert!(text.contains("line 12"), "keeps the first {TAIL_LINES}");
        assert!(!text.contains("line 13"), "and drops the rest");
        assert!(text.ends_with("… 8 more lines"), "{text}");
    }

    #[test]
    fn a_path_inside_the_project_is_shown_relative_to_it() {
        assert_eq!(relative("/home/u/app/src/main.rs", "/home/u/app"), "src/main.rs");
        assert_eq!(relative("/tmp/scratch.py", "/home/u/app"), "/tmp/scratch.py");
    }
}
