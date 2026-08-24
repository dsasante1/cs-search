//! `cs activity` — where the time went, by day and by project.
//!
//! `stats` totals the corpus; this cuts the same records by day, which is the
//! only axis that answers "what happened last month" rather than "what is in
//! here". Both walk every record, and neither infers anything: a day's row is a
//! count of messages that carry that date.
//!
//! Days with no activity are absent rather than shown as zero. A range of a
//! year would otherwise be three hundred empty rows, and the gaps between the
//! dates say the same thing.

use crate::cli::Opts;
use crate::record::Record;
use crate::stats::thousands;
use crate::{dates, scan};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// How many days the human table prints before folding the rest away. `--json`
/// is never folded: a program asked for the range it asked for.
const DAYS: usize = 30;

/// How many projects to name under the table.
const TOP: usize = 8;

/// Widest a bar may be drawn, in columns.
const BAR: usize = 22;

#[derive(Default)]
struct DayTally {
    messages: usize,
    sessions: HashSet<String>,
    projects: HashSet<String>,
}

#[derive(Default)]
pub struct Activity {
    days: HashMap<String, DayTally>,
    projects: HashMap<String, usize>,
    sessions: HashSet<String>,
    messages: usize,
}

/// One row of the table, once the sets behind it have been counted.
pub struct Day {
    pub day: String,
    pub messages: usize,
    pub sessions: usize,
    pub projects: usize,
}

impl Activity {
    fn merge(&mut self, o: Activity) {
        for (day, t) in o.days {
            let e = self.days.entry(day).or_default();
            e.messages += t.messages;
            e.sessions.extend(t.sessions);
            e.projects.extend(t.projects);
        }
        for (p, n) in o.projects {
            *self.projects.entry(p).or_default() += n;
        }
        self.sessions.extend(o.sessions);
        self.messages += o.messages;
    }

    /// Days newest first, which is the order the rest of this tool lists things
    /// in and the end of the range you are usually asking about.
    pub fn days(&self) -> Vec<Day> {
        let mut out: Vec<Day> = self
            .days
            .iter()
            .map(|(day, t)| Day {
                day: day.clone(),
                messages: t.messages,
                sessions: t.sessions.len(),
                projects: t.projects.len(),
            })
            .collect();
        out.sort_by(|a, b| b.day.cmp(&a.day));
        out
    }

    pub fn projects(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = self
            .projects
            .iter()
            .map(|(p, n)| (p.clone(), *n))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.messages == 0
    }
}

pub fn collect(opts: &Opts) -> Activity {
    let queue = Arc::new(Mutex::new(scan::transcripts()));
    let out: Arc<Mutex<Activity>> = Arc::new(Mutex::new(Activity::default()));

    std::thread::scope(|s| {
        for _ in 0..opts.jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            s.spawn(move || {
                let mut local = Activity::default();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };
                    read(&path, opts, &mut local);
                }
                out.lock().unwrap().merge(local);
            });
        }
    });

    Arc::try_unwrap(out).ok().unwrap().into_inner().unwrap()
}

fn read(path: &Path, opts: &Opts, into: &mut Activity) {
    let Ok(fh) = File::open(path) else { return };
    for line in BufReader::with_capacity(1 << 20, fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        if !opts.project.is_empty() && !r.cwd().to_lowercase().contains(&opts.project) {
            continue;
        }
        if !opts.branch.is_empty() && !r.git_branch().to_lowercase().contains(&opts.branch) {
            continue;
        }
        if !opts.since.is_empty() && r.timestamp() < opts.since.as_str() {
            continue;
        }
        if !opts.until.is_empty() && dates::day_of(r.timestamp()) > opts.until.as_str() {
            continue;
        }
        let day = dates::day_of(r.timestamp());
        if day.is_empty() {
            continue;
        }
        let project = r.cwd().rsplit('/').next().unwrap_or("?");
        let project = if project.is_empty() { "?" } else { project };

        let e = into.days.entry(day.to_owned()).or_default();
        e.messages += 1;
        e.sessions.insert(r.session_id().to_owned());
        e.projects.insert(project.to_owned());

        *into.projects.entry(project.to_owned()).or_default() += 1;
        into.sessions.insert(r.session_id().to_owned());
        into.messages += 1;
    }
}

/// A bar `n` long relative to `max`, in at most `width` columns.
///
/// Magnitude is the one thing a column of numbers is bad at, and a bar is read
/// at a glance. The half block earns its place at the bottom of the scale: a
/// quiet day next to a busy one would otherwise round to nothing at all, which
/// reads as no activity rather than a little.
pub fn bar(n: usize, max: usize, width: usize) -> String {
    if max == 0 || n == 0 {
        return String::new();
    }
    let eighths = (n as f64 / max as f64 * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let rest = eighths % 8;
    let mut out = "\u{2588}".repeat(full);
    if rest >= 4 {
        out.push('\u{258c}');
    } else if out.is_empty() {
        out.push('\u{258f}');
    }
    out
}

pub fn report(w: &mut impl Write, a: &Activity, bars: bool) {
    let days = a.days();
    let busiest = days.iter().map(|d| d.messages).max().unwrap_or(0);

    let _ = writeln!(w, "DAY         sessions  messages");
    for d in days.iter().take(DAYS) {
        let _ = writeln!(
            w,
            "{}  {:>8}  {:>8}  {}",
            d.day,
            thousands(d.sessions as u64),
            thousands(d.messages as u64),
            if bars { bar(d.messages, busiest, BAR) } else { String::new() },
        );
    }
    if days.len() > DAYS {
        let _ = writeln!(w, "  … {} earlier days", days.len() - DAYS);
    }

    let _ = writeln!(
        w,
        "\n{} active {} · {} sessions · {} messages",
        days.len(),
        if days.len() == 1 { "day" } else { "days" },
        thousands(a.sessions.len() as u64),
        thousands(a.messages as u64),
    );

    let projects = a.projects();
    if projects.len() > 1 {
        let _ = writeln!(w, "\nPROJECTS");
        for (name, n) in projects.iter().take(TOP) {
            let _ = writeln!(w, "  {:>8}  {name}", thousands(*n as u64));
        }
        if projects.len() > TOP {
            let _ = writeln!(w, "  {:>8}  … {} more", "", projects.len() - TOP);
        }
    }
}

pub fn report_json(w: &mut impl Write, a: &Activity) {
    for d in a.days() {
        let _ = writeln!(
            w,
            "{}",
            json!({"day": d.day, "sessions": d.sessions, "messages": d.messages,
                   "projects": d.projects})
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tallied(rows: &[(&str, &str, &str)]) -> Activity {
        let mut a = Activity::default();
        for (day, sid, project) in rows {
            let e = a.days.entry((*day).to_owned()).or_default();
            e.messages += 1;
            e.sessions.insert((*sid).to_owned());
            e.projects.insert((*project).to_owned());
            *a.projects.entry((*project).to_owned()).or_default() += 1;
            a.sessions.insert((*sid).to_owned());
            a.messages += 1;
        }
        a
    }

    #[test]
    fn a_day_counts_messages_but_distinct_sessions() {
        let a = tallied(&[
            ("2026-08-20", "aaaa", "api"),
            ("2026-08-20", "aaaa", "api"),
            ("2026-08-20", "bbbb", "cs"),
        ]);
        let days = a.days();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].messages, 3, "three messages");
        assert_eq!(days[0].sessions, 2, "but only two sessions");
        assert_eq!(days[0].projects, 2);
    }

    #[test]
    fn days_run_newest_first() {
        let a = tallied(&[
            ("2026-08-18", "aaaa", "api"),
            ("2026-08-20", "bbbb", "api"),
            ("2026-08-19", "cccc", "api"),
        ]);
        let order: Vec<String> = a.days().into_iter().map(|d| d.day).collect();
        assert_eq!(order, ["2026-08-20", "2026-08-19", "2026-08-18"]);
    }

    /// A session spanning midnight is one session on each of the two days, and
    /// one session overall — the totals are not the columns added up.
    #[test]
    fn a_session_spanning_midnight_is_counted_once_overall() {
        let a = tallied(&[
            ("2026-08-20", "aaaa", "api"),
            ("2026-08-21", "aaaa", "api"),
        ]);
        assert_eq!(a.days().len(), 2);
        assert_eq!(a.sessions.len(), 1);
    }

    #[test]
    fn a_bar_is_drawn_in_proportion_to_the_busiest_day() {
        assert_eq!(bar(100, 100, 8).chars().count(), 8, "the maximum fills it");
        assert_eq!(bar(50, 100, 8).chars().count(), 4, "half is half");
        assert!(bar(0, 100, 8).is_empty(), "nothing draws nothing");
        assert!(bar(5, 0, 8).is_empty(), "no scale, no bar");
    }

    /// Rounding a quiet day to zero columns would read as a day off.
    #[test]
    fn a_day_with_any_activity_at_all_still_shows_something() {
        assert!(!bar(1, 100_000, 22).is_empty());
    }
}
