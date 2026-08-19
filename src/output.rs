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
