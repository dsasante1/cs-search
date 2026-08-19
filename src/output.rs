//! Row formatting. Mirrors the awk formatter in the original script: colour only
//! when stdout is a terminal, so piping into another command yields clean text.

use regex::Regex;
use std::io::IsTerminal;
use std::sync::OnceLock;

pub const DIM: &str = "\x1b[2m";
pub const CYAN: &str = "\x1b[36m";
pub const MAGENTA: &str = "\x1b[35m";
pub const HIT: &str = "\x1b[1;31m";
pub const RESET: &str = "\x1b[0m";

pub fn is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// One output line, kept as its component fields so sorting happens on the
/// timestamp-first tuple exactly as `sort` did on the original TSV.
pub struct Row {
    pub ts: String,
    pub project: String,
    pub role: String,
    pub sid: String,
    pub text: String,
}

impl Row {
    pub fn sort_key(&self) -> (&str, &str, &str, &str, &str) {
        (&self.ts, &self.project, &self.role, &self.sid, &self.text)
    }

    pub fn render(&self, color: bool, hl: Option<&Regex>) -> String {
        let proj = fixed(&self.project, 16);
        let role = pad(&self.role, 4);
        if color {
            let text = match hl {
                Some(re) => highlight(&self.text, re),
                None => self.text.clone(),
            };
            format!(
                "{DIM}{}{RESET} {CYAN}{proj}{RESET} {MAGENTA}{role}{RESET} {DIM}{}{RESET}  {text}",
                self.ts, self.sid
            )
        } else {
            format!("{} {proj} {role} {}  {}", self.ts, self.sid, self.text)
        }
    }
}

/// awk's `%-N.Ns`: truncate to N chars, then pad right to N.
pub fn fixed(s: &str, n: usize) -> String {
    let t = crate::record::take_chars(s, n);
    pad(t, n)
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
    static WS: OnceLock<Regex> = OnceLock::new();
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

/// Replaces the `rg --passthru --color=always` pass at the end of the original
/// pipeline, but highlights only the snippet rather than the metadata columns.
fn highlight(s: &str, re: &Regex) -> String {
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
        }
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
    fn fixed_truncates_then_pads_like_awk() {
        // awk's %-5.5s
        assert_eq!(fixed("abc", 5), "abc  ");
        assert_eq!(fixed("abcdefgh", 5), "abcde");
        assert_eq!(fixed("", 3), "   ");
        // Padding is by character, so a multi-byte name still lines up.
        assert_eq!(fixed("日本", 4).chars().count(), 4);
    }

    #[test]
    fn pad_never_truncates() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }

    #[test]
    fn plain_render_has_no_escape_sequences() {
        let out = row().render(false, None);
        assert_eq!(out, "2026-08-19 03:18 proj             user 1e59cda9  hello world");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn colour_render_wraps_fields_and_highlights_matches() {
        let re = Regex::new("world").unwrap();
        let out = row().render(true, Some(&re));
        assert!(out.contains(CYAN), "project should be cyan");
        assert!(out.contains(&format!("{HIT}world{RESET}")), "match should stand out");
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

    fn row_with(text: &str) -> Row {
        Row { text: text.into(), ..row() }
    }
}
