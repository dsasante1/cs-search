//! Row formatting: colour only when stdout is a terminal, so piping into another
//! command yields clean text.
//!
//! Three renderings share one `Row`. Flat output is the original awk-style
//! columns and is what a pipe gets, unchanged. Grouped output folds the same
//! rows under one heading per session, which is what a terminal gets, because a
//! broad search returns hundreds of lines spread over dozens of sessions and a
//! flat list of them is unreadable. `--json` emits one object per line.

use crate::cli::Opts;
use regex::Regex;
use serde_json::json;
use std::collections::HashMap;
use std::io::{IsTerminal, Write};

pub const DIM: &str = "\x1b[2m";
pub const CYAN: &str = "\x1b[36m";
pub const MAGENTA: &str = "\x1b[35m";
pub const HIT: &str = "\x1b[1;31m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";

/// The project column sizes itself to the widest name actually present, within
/// these bounds — a fixed 16 truncated real names into things like
/// `unicare_hostel_m`, and a fully elastic column ruins alignment.
const MIN_PROJECT: usize = 8;
const MAX_PROJECT: usize = 20;

/// How many matches a session shows before the rest are folded away.
const PER_GROUP: usize = 5;

pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Width of the terminal this output will land in.
///
/// Checked in order of how much each source actually knows. fzf exports the
/// preview pane's width, and that is the case that matters most: a rule sized
/// for an 80-column terminal wraps into nonsense inside a 45-column preview.
/// The ioctl asks stderr before stdout because stdout is so often a pipe here —
/// a pager, or fzf — while stderr is still the terminal.
pub fn term_width() -> usize {
    env_width("FZF_PREVIEW_COLUMNS")
        .or_else(|| env_width("COLUMNS"))
        .or_else(tty_width)
        .unwrap_or(FALLBACK_WIDTH)
}

/// Assumed width when nothing will say: the one every terminal is at least.
const FALLBACK_WIDTH: usize = 80;

fn env_width(var: &str) -> Option<usize> {
    parse_width(std::env::var(var).ok().as_deref())
}

/// A width is only usable if it parses and is non-zero — these variables are
/// routinely set to junk, or exported as empty by a shell that never had a tty.
fn parse_width(raw: Option<&str>) -> Option<usize> {
    raw?.trim().parse::<usize>().ok().filter(|w| *w > 0)
}

#[cfg(unix)]
fn tty_width() -> Option<usize> {
    #[repr(C)]
    struct Winsize {
        rows: u16,
        cols: u16,
        xpixel: u16,
        ypixel: u16,
    }
    // The request number is part of the platform ABI, not a portable constant.
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: u64 = 0x5413;
    #[cfg(not(target_os = "linux"))]
    const TIOCGWINSZ: u64 = 0x4008_7468;

    extern "C" {
        fn ioctl(fd: i32, request: u64, ...) -> i32;
    }
    let mut ws = Winsize { rows: 0, cols: 0, xpixel: 0, ypixel: 0 };
    for fd in [2, 1] {
        // Safe: the kernel writes exactly one Winsize through the pointer, and
        // a non-tty fd fails with -1 rather than touching it.
        if unsafe { ioctl(fd, TIOCGWINSZ, &mut ws) } == 0 && ws.cols > 0 {
            return Some(ws.cols as usize);
        }
    }
    None
}

#[cfg(not(unix))]
fn tty_width() -> Option<usize> {
    None
}

/// Progress and summary lines are diagnostics, so they follow stderr's terminal
/// status rather than stdout's: `cs x > file` on a terminal should still say how
/// much it found, and `cs x | wc -l` in a script should stay silent.
pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

/// One output line, kept as its component fields so sorting happens on the
/// timestamp-first tuple exactly as `sort` did on the original TSV.
#[derive(Default)]
pub struct Row {
    pub ts: String,
    pub project: String,
    /// The git branch the session was on. Carried on every row, but kept out of
    /// the flat columns: those are the interface scripts parse.
    pub branch: String,
    pub role: String,
    pub sid: String,
    pub text: String,
    /// Neighbouring lines from the same block, populated only by -C/-A/-B.
    pub before: Vec<String>,
    pub after: Vec<String>,
}

impl Row {
    /// Context lines are deliberately excluded: two matches differing only in
    /// their surroundings are still the same row for ordering purposes.
    pub fn sort_key(&self) -> (&str, &str, &str, &str, &str) {
        (&self.ts, &self.project, &self.role, &self.sid, &self.text)
    }

    pub fn render(&self, color: bool, hl: Option<&Regex>, width: usize) -> String {
        let proj = fixed(&self.project, width);
        let role = pad(&self.role, 4);
        let mut out = if color {
            format!(
                "{DIM}{}{RESET} {CYAN}{proj}{RESET} {DIM}{role}{RESET} {DIM}{}{RESET}  {}",
                self.ts,
                self.sid,
                highlighted(&self.text, hl),
            )
        } else {
            format!("{} {proj} {role} {}  {}", self.ts, self.sid, self.text)
        };
        self.append_context(&mut out, color, " ".repeat(width + 33));
        out
    }

    /// Context sits under its match, indented past the metadata columns so the
    /// matching line stays the only one starting at the left edge.
    fn append_context(&self, out: &mut String, color: bool, indent: String) {
        for line in self.before.iter().chain(&self.after) {
            if color {
                out.push_str(&format!("\n{indent}{DIM}{line}{RESET}"));
            } else {
                out.push_str(&format!("\n{indent}{line}"));
            }
        }
    }

    fn to_json(&self) -> serde_json::Value {
        json!({
            "ts": self.ts,
            "project": self.project,
            "branch": self.branch,
            "role": self.role,
            "session": self.sid,
            "text": self.text,
            "before": self.before,
            "after": self.after,
        })
    }
}

/// Width of the project column for this result set.
pub fn project_width(rows: &[Row]) -> usize {
    rows.iter()
        .map(|r| r.project.chars().count())
        .max()
        .unwrap_or(MIN_PROJECT)
        .clamp(MIN_PROJECT, MAX_PROJECT)
}

/// Truncate to at most n chars from the middle, then pad right to n.
pub fn fixed(s: &str, n: usize) -> String {
    pad(&elide(s, n), n)
}

/// Shorten to n characters by removing the middle, so both ends stay readable:
/// `dashqard-customer-portal` reads better as `dashqar…r-portal` than as
/// `dashqard-custome`.
pub fn elide(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    if n <= 1 {
        return crate::record::take_chars(s, n).to_owned();
    }
    let keep = n - 1;
    let tail = keep / 2;
    let head = keep - tail;
    let chars: Vec<char> = s.chars().collect();
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// awk's `%-Ns`: pad right to N chars.
pub fn pad(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.to_owned()
    } else {
        format!("{s}{}", " ".repeat(n - len))
    }
}

fn ws() -> &'static Regex {
    static WS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    WS.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// jq's `gsub("\\s+";" ") | ltrimstr(" ")`: collapse whitespace runs to a single
/// space and drop one leading space.
pub fn squash(s: &str) -> String {
    let collapsed = ws().replace_all(s, " ");
    collapsed
        .strip_prefix(' ')
        .map(str::to_owned)
        .unwrap_or_else(|| collapsed.into_owned())
}

/// jq's `clip`: truncate to n chars, appending an ellipsis if anything was cut.
pub fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", crate::record::take_chars(s, n))
    } else {
        s.to_owned()
    }
}

fn highlighted(s: &str, hl: Option<&Regex>) -> String {
    match hl {
        Some(re) => highlight(s, re),
        None => s.to_owned(),
    }
}

/// Replaces the `rg --passthru --color=always` pass at the end of the original
/// pipeline, but highlights only the snippet rather than the metadata columns.
pub fn highlight(s: &str, re: &Regex) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for m in re.find_iter(s) {
        if m.start() < last {
            continue;
        }
        out.push_str(&s[last..m.start()]);
        out.push_str(HIT);
        out.push_str(m.as_str());
        out.push_str(RESET);
        last = m.end();
    }
    out.push_str(&s[last..]);
    out
}

// ------------------------------------------------------------------ printing

/// The original one-line-per-match format. Everything piped gets this.
pub fn print_flat(w: &mut impl Write, rows: &[Row], color: bool, hl: Option<&Regex>) {
    let width = project_width(rows);
    for r in rows {
        let _ = writeln!(w, "{}", r.render(color, hl, width));
    }
}

/// Sessions in the order their first match appears, each with its matches folded
/// to `PER_GROUP` and a pointer to the command that shows the rest.
pub fn print_grouped(w: &mut impl Write, rows: &[Row], color: bool, hl: Option<&Regex>) {
    let (c, d, b, z) = if color {
        (CYAN, DIM, BOLD, RESET)
    } else {
        ("", "", "", "")
    };
    let groups = group_by_session(rows);
    // Counts line up in a column of their own, so "which session has the most
    // hits" is a glance down the right-hand edge rather than a read of every
    // heading. The gutter is measured from the plain text: escape sequences
    // occupy no columns, so padding computed with them in would be wrong.
    let gutter = groups
        .iter()
        .map(|g| heading_width(g[0]))
        .max()
        .unwrap_or(0)
        + 2;

    for (i, g) in groups.iter().enumerate() {
        let head = g[0];
        if i > 0 {
            let _ = writeln!(w);
        }
        let plural = if g.len() == 1 { "match" } else { "matches" };
        let _ = writeln!(
            w,
            "{d}▸{z} {b}{c}{}{z}{d}{}{z} {d}{}{z}  {d}{}{z}{}{d}{} {plural}{z}",
            head.project,
            branch_tag(head),
            head.sid,
            head.ts,
            " ".repeat(gutter.saturating_sub(heading_width(head))),
            g.len(),
        );
        for r in g.iter().take(PER_GROUP) {
            // The date lives in the heading, so rows only need month-day-time.
            let when = crate::record::take_chars(&r.ts, 16);
            let when = when.get(5..).unwrap_or(when);
            let _ = writeln!(
                w,
                "  {d}{when}{z} {d}{}{z} {}",
                pad(&r.role, 4),
                highlighted(&r.text, hl),
            );
            for line in r.before.iter().chain(&r.after) {
                let _ = writeln!(w, "  {d}{:>11} {:4} {line}{z}", "", "");
            }
        }
        if g.len() > PER_GROUP {
            let _ = writeln!(
                w,
                "  {d}… {} more · cs show {}{z}",
                g.len() - PER_GROUP,
                head.sid
            );
        }
    }
}

/// `--chrono`: one line per session, oldest first, quoting the line that first
/// matched in it.
///
/// A search answers "where was this mentioned"; this answers "how did it
/// develop", which is a different shape — one row per session rather than per
/// match, running forwards rather than backwards. Nothing is summarised: the
/// text on each row is a line somebody actually wrote, picked by being the
/// first hit in that session, and the reading of the progression is the
/// reader's to do.
///
/// Rows arrive sorted by timestamp, so grouping them by session already yields
/// the sessions in the order their first match appeared. There is nothing left
/// to sort.
pub fn print_chrono(w: &mut impl Write, rows: &[Row], color: bool, hl: Option<&Regex>) {
    let (c, d, z) = if color { (CYAN, DIM, RESET) } else { ("", "", "") };
    let sessions = per_session(rows);
    let width = project_width(rows);
    let digits = sessions
        .iter()
        .map(|(_, n)| n.to_string().len())
        .max()
        .unwrap_or(1);

    // The one renderer whose contract is a line per session, so the line has to
    // fit: a snippet that wraps three times is no longer a timeline. Only on a
    // terminal — a pipe gets the text it would have got anywhere else.
    let room = color.then(|| {
        term_width().saturating_sub(width + 33 + digits).max(24)
    });

    for (head, n) in &sessions {
        let text = match room {
            Some(r) => clip(&head.text, r),
            None => head.text.clone(),
        };
        let _ = writeln!(
            w,
            "{d}{}{z} {c}{}{z} {d}{}{z} {d}{:>digits$}{z}  {}",
            head.ts,
            fixed(&head.project, width),
            head.sid,
            n,
            highlighted(&text, hl),
        );
    }
}

/// Each session in a result set, as its first match and how many it had.
///
/// Shared by `--chrono` and by `cs history --sessions`, so the two can never
/// disagree about which line stands for a session or how many it stood for.
pub fn per_session(rows: &[Row]) -> Vec<(&Row, usize)> {
    group_by_session(rows)
        .into_iter()
        .map(|g| (g[0], g.len()))
        .collect()
}

/// One JSON object per line: a whole-array encoding would have to be buffered
/// and reformatted to be read, and this stays greppable.
pub fn print_json(w: &mut impl Write, rows: &[Row]) {
    for r in rows {
        let _ = writeln!(w, "{}", r.to_json());
    }
}

/// The branch a session was on, as it rides beside the project name. Empty for
/// a session outside a repository, and for the prompt history, which does not
/// record one.
fn branch_tag(head: &Row) -> String {
    if head.branch.is_empty() {
        String::new()
    } else {
        format!("@{}", elide(&head.branch, MAX_BRANCH))
    }
}

/// A long branch name would push the whole heading grid sideways, and the ticket
/// number that makes one long is rarely the half you need to recognise it.
const MAX_BRANCH: usize = 18;

/// Printed width of a heading up to where its count begins.
fn heading_width(head: &Row) -> usize {
    // "▸ " + project + branch + " " + sid + "  " + ts
    2 + head.project.chars().count()
        + branch_tag(head).chars().count()
        + 1
        + head.sid.chars().count()
        + 2
        + head.ts.chars().count()
}

fn group_by_session(rows: &[Row]) -> Vec<Vec<&Row>> {
    let mut order: HashMap<&str, usize> = HashMap::new();
    let mut groups: Vec<Vec<&Row>> = Vec::new();
    for r in rows {
        match order.get(r.sid.as_str()) {
            Some(&i) => groups[i].push(r),
            None => {
                order.insert(&r.sid, groups.len());
                groups.push(vec![r]);
            }
        }
    }
    groups
}

/// `506 matches · 98 sessions · 15 projects` — the orientation a flat dump of
/// 506 lines does not give you.
pub fn summary(rows: &[Row]) -> String {
    let sessions = distinct(rows, |r| &r.sid);
    let projects = distinct(rows, |r| &r.project);
    format!(
        "{} {} · {} {} · {} {}",
        rows.len(),
        plural(rows.len(), "match", "matches"),
        sessions,
        plural(sessions, "session", "sessions"),
        projects,
        plural(projects, "project", "projects"),
    )
}

fn distinct(rows: &[Row], key: impl Fn(&Row) -> &String) -> usize {
    let mut seen: Vec<&str> = rows.iter().map(|r| key(r).as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 {
        one
    } else {
        many
    }
}

/// Advisory shown when a broad result set looks like it came from a pattern the
/// user meant literally. `cs 'C++'` returns tens of thousands of rows — every
/// line containing a `c` — with nothing to say it was read as a regex.
const WIDE: usize = 1000;

pub fn regex_hint(opts: &Opts, hits: usize) -> Option<String> {
    if opts.fixed || hits < WIDE || !has_meta(&opts.pattern) {
        return None;
    }
    Some(format!(
        "{hits} matches — '{}' was read as a regex; -F searches for it literally",
        opts.pattern
    ))
}

fn has_meta(pat: &str) -> bool {
    pat.chars().any(|c| "\\.+*?()[]{}|^$".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        Row {
            ts: "2026-08-19 03:18".into(),
            project: "proj".into(),
            role: "user".into(),
            sid: "1e59cda9".into(),
            text: "hello world".into(),
            ..Default::default()
        }
    }

    fn row_with(text: &str) -> Row {
        Row { text: text.into(), ..row() }
    }

    fn rendered(rows: &[Row], grouped: bool) -> String {
        let mut buf: Vec<u8> = Vec::new();
        if grouped {
            print_grouped(&mut buf, rows, false, None);
        } else {
            print_flat(&mut buf, rows, false, None);
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn squash_collapses_runs_and_strips_one_leading_space() {
        assert_eq!(squash("a  b"), "a b");
        assert_eq!(squash("a\t\tb"), "a b");
        assert_eq!(squash("a\n b"), "a b");
        assert_eq!(squash("   a"), "a");
        // jq's ltrimstr(" ") removes a single leading space, and nothing trailing.
        assert_eq!(squash("a   "), "a ");
        assert_eq!(squash(""), "");
    }

    #[test]
    fn clip_appends_ellipsis_only_when_it_cuts() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("hello", 5), "hello");
        assert_eq!(clip("hello", 4), "hell…");
        assert_eq!(clip("日本語テスト", 3), "日本語…");
    }

    #[test]
    fn elide_removes_the_middle_and_keeps_both_ends() {
        assert_eq!(elide("short", 10), "short");
        assert_eq!(elide("abcdefgh", 8), "abcdefgh");
        assert_eq!(elide("abcdefgh", 5), "ab…gh");
        // The real motivation: a truncating column made these unreadable.
        assert_eq!(elide("dashqard-customer-portal", 16), "dashqard…-portal");
        assert_eq!(elide("unicare_hostel_manager", 16), "unicare_…manager");
        // Every result is exactly the requested width.
        for n in 2..=16 {
            assert_eq!(elide("abcdefghijklmnop", n).chars().count(), n, "n={n}");
        }
    }

    #[test]
    fn elide_degrades_rather_than_panicking_on_tiny_widths() {
        assert_eq!(elide("abcdef", 1), "a");
        assert_eq!(elide("abcdef", 0), "");
        // Multi-byte characters must not be sliced mid-character.
        assert_eq!(elide("日本語テスト漢字", 5).chars().count(), 5);
    }

    #[test]
    fn fixed_elides_then_pads() {
        assert_eq!(fixed("abc", 5), "abc  ");
        assert_eq!(fixed("abcdefgh", 5), "ab…gh");
        assert_eq!(fixed("", 3), "   ");
        assert_eq!(fixed("日本", 4).chars().count(), 4);
    }

    #[test]
    fn pad_never_truncates() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }

    #[test]
    fn project_width_tracks_the_widest_name_within_bounds() {
        let wide = |p: &str| Row { project: p.into(), ..row() };
        assert_eq!(project_width(&[wide("ab")]), MIN_PROJECT, "short names get a floor");
        assert_eq!(project_width(&[wide("exactly-thirteen")]), 16);
        assert_eq!(
            project_width(&[wide(&"x".repeat(99))]),
            MAX_PROJECT,
            "one absurd name must not push every row off-screen"
        );
        assert_eq!(project_width(&[]), MIN_PROJECT);
    }

    #[test]
    fn plain_render_has_no_escape_sequences() {
        let out = row().render(false, None, 8);
        assert_eq!(out, "2026-08-19 03:18 proj     user 1e59cda9  hello world");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn colour_render_wraps_fields_and_highlights_matches() {
        let re = Regex::new("world").unwrap();
        let out = row().render(true, Some(&re), 8);
        assert!(out.contains(CYAN), "project should be cyan");
        assert!(out.contains(&format!("{HIT}world{RESET}")), "match should stand out");
    }

    #[test]
    fn a_search_row_spends_colour_only_on_the_project_and_the_match() {
        // The role column used to be magenta, immediately left of the match and
        // competing with it. Colour here has to mean "this is what you asked
        // for", so magenta is gone from the row entirely.
        let re = Regex::new("world").unwrap();
        let out = row().render(true, Some(&re), 8);
        assert!(!out.contains(MAGENTA), "the role column must not be magenta: {out:?}");

        let mut buf: Vec<u8> = Vec::new();
        print_grouped(&mut buf, &[row()], true, Some(&re));
        let grouped = String::from_utf8(buf).unwrap();
        assert!(!grouped.contains(MAGENTA), "nor in grouped output: {grouped:?}");
    }

    #[test]
    fn a_width_is_only_taken_when_it_parses_and_is_usable() {
        assert_eq!(parse_width(Some("120")), Some(120));
        assert_eq!(parse_width(Some(" 80 ")), Some(80));
        // A shell with no tty exports these empty or zero; neither is a width.
        assert_eq!(parse_width(Some("")), None);
        assert_eq!(parse_width(Some("0")), None);
        assert_eq!(parse_width(Some("wide")), None);
        assert_eq!(parse_width(Some("-40")), None);
        assert_eq!(parse_width(None), None);
    }

    #[test]
    fn a_width_is_always_produced_even_with_nothing_to_go_on() {
        // Called for every divider, so it must never fail to answer.
        assert!(term_width() > 0);
    }

    #[test]
    fn group_headings_lead_with_a_marker_and_align_their_counts() {
        let mk = |sid: &str, project: &str| Row {
            sid: sid.into(),
            project: project.into(),
            ..row()
        };
        // Deliberately mismatched name lengths: alignment is the whole point.
        let rows = [mk("aaaaaaaa", "cs"), mk("bbbbbbbb", "dashqard-customer-api")];
        let out = rendered(&rows, true);

        let headings: Vec<&str> = out.lines().filter(|l| l.starts_with('▸')).collect();
        assert_eq!(headings.len(), 2, "one marker per group:\n{out}");

        let at = |l: &str| l.find("1 match").expect("every heading carries its count");
        assert_eq!(
            at(headings[0]),
            at(headings[1]),
            "counts must start in the same column:\n{out}"
        );
        // The short-named group is the one that had to be padded out.
        assert!(headings[0].contains("cs "), "{:?}", headings[0]);
    }

    #[test]
    fn the_count_gutter_is_measured_without_escape_sequences() {
        // Padding computed over coloured text would be wrong by however many
        // bytes the escapes take, so plain and coloured must agree.
        let mk = |sid: &str, project: &str| Row {
            sid: sid.into(),
            project: project.into(),
            ..row()
        };
        let rows = [mk("aaaaaaaa", "cs"), mk("bbbbbbbb", "dashqard-customer-api")];

        let plain = rendered(&rows, true);
        let mut buf: Vec<u8> = Vec::new();
        print_grouped(&mut buf, &rows, true, None);
        let painted = String::from_utf8(buf).unwrap();
        let strip = |s: &str| {
            s.replace(DIM, "").replace(CYAN, "").replace(BOLD, "").replace(RESET, "")
        };
        assert_eq!(strip(&painted), plain, "colour must not move the gutter");
    }

    #[test]
    fn context_lines_are_indented_under_their_match() {
        let r = Row {
            before: vec!["line above".into()],
            after: vec!["line below".into()],
            ..row()
        };
        let out = r.render(false, None, 8);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("hello world"));
        assert!(lines[1].trim() == "line above" && lines[1].starts_with(' '));
        assert!(lines[2].trim() == "line below");
        // The match is the only line flush against the left edge.
        assert!(!lines[0].starts_with(' '));
    }

    #[test]
    fn highlight_wraps_every_match_and_preserves_text() {
        let re = Regex::new("(?i)ab").unwrap();
        let out = highlight("ab cd AB", &re);
        assert_eq!(out, format!("{HIT}ab{RESET} cd {HIT}AB{RESET}"));
        // Stripping the escapes must give the original back.
        assert_eq!(out.replace(HIT, "").replace(RESET, ""), "ab cd AB");
    }

    #[test]
    fn highlight_leaves_non_matching_text_alone() {
        let re = Regex::new("zzz").unwrap();
        assert_eq!(highlight("nothing here", &re), "nothing here");
    }

    #[test]
    fn sort_key_orders_by_timestamp_first() {
        let mut rows = [
            Row { ts: "2026-08-19 03:18".into(), ..row_with("b") },
            Row { ts: "2026-01-01 00:00".into(), ..row_with("a") },
        ];
        rows.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(rows[0].ts, "2026-01-01 00:00");
    }

    #[test]
    fn grouping_collects_a_session_even_when_its_matches_are_not_adjacent() {
        let mk = |sid: &str, text: &str| Row {
            sid: sid.into(),
            text: text.into(),
            ..row()
        };
        let rows = [mk("aaa", "one"), mk("bbb", "two"), mk("aaa", "three")];
        let groups = group_by_session(&rows);
        assert_eq!(groups.len(), 2);
        // First-appearance order, so the listing still reads oldest-first.
        assert_eq!(groups[0].len(), 2, "both 'aaa' rows belong to one group");
        assert_eq!(groups[1][0].text, "two");
    }

    #[test]
    fn grouped_output_heads_each_session_once() {
        let rows: Vec<Row> = (0..3).map(|i| row_with(&format!("hit {i}"))).collect();
        let out = rendered(&rows, true);
        assert_eq!(out.matches("1e59cda9").count(), 1, "one heading:\n{out}");
        assert!(out.contains("3 matches"), "{out}");
        for i in 0..3 {
            assert!(out.contains(&format!("hit {i}")), "{out}");
        }
    }

    #[test]
    fn grouped_output_folds_long_sessions_and_says_how_to_see_the_rest() {
        let rows: Vec<Row> = (0..9).map(|i| row_with(&format!("hit {i}"))).collect();
        let out = rendered(&rows, true);
        assert!(out.contains(&format!("… {} more", 9 - PER_GROUP)), "{out}");
        assert!(out.contains("cs show 1e59cda9"), "{out}");
        assert!(out.contains("hit 4"), "the fifth match is still shown:\n{out}");
        assert!(!out.contains("hit 5"), "the sixth is folded away:\n{out}");
    }

    #[test]
    fn a_single_match_is_not_pluralised() {
        let out = rendered(&[row()], true);
        assert!(out.contains("1 match"), "{out}");
        assert!(!out.contains("1 matches"), "{out}");
    }

    #[test]
    fn flat_output_is_one_line_per_row() {
        let rows: Vec<Row> = (0..4).map(|i| row_with(&format!("hit {i}"))).collect();
        assert_eq!(rendered(&rows, false).lines().count(), 4);
    }

    #[test]
    fn summary_counts_distinct_sessions_and_projects() {
        let mk = |sid: &str, project: &str| Row {
            sid: sid.into(),
            project: project.into(),
            ..row()
        };
        let rows = [
            mk("aaa", "alpha"),
            mk("aaa", "alpha"),
            mk("bbb", "alpha"),
            mk("ccc", "beta"),
        ];
        assert_eq!(summary(&rows), "4 matches · 3 sessions · 2 projects");
        assert_eq!(summary(&[row()]), "1 match · 1 session · 1 project");
    }

    #[test]
    fn json_output_is_one_object_per_line() {
        let mut buf: Vec<u8> = Vec::new();
        print_json(&mut buf, &[row(), row_with("second")]);
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["session"], "1e59cda9");
        assert_eq!(v["text"], "hello world");
        assert_eq!(v["project"], "proj");
    }

    #[test]
    fn json_carries_context_lines() {
        let r = Row { before: vec!["above".into()], ..row() };
        let mut buf: Vec<u8> = Vec::new();
        print_json(&mut buf, &[r]);
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8(buf).unwrap()).unwrap();
        assert_eq!(v["before"][0], "above");
        assert!(v["after"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_regex_hint_fires_only_on_a_wide_result_from_a_metacharacter_pattern() {
        let with = |pat: &str, fixed: bool| Opts {
            pattern: pat.into(),
            fixed,
            ..Default::default()
        };
        // The case that motivated it: 'C++' matched every line containing a 'c'.
        assert!(regex_hint(&with("C++", false), 28_942).is_some());
        // A plain word cannot have been misread, however many rows it returns.
        assert!(regex_hint(&with("database", false), 28_942).is_none());
        // A regex that returns a handful of rows did what the user wanted.
        assert!(regex_hint(&with("C++", false), 3).is_none());
        // -F says the pattern is a literal, so there is nothing to warn about.
        assert!(regex_hint(&with("C++", true), 28_942).is_none());
    }
}
