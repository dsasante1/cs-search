//! `cs stats` — what the corpus is made of.
//!
//! Every assistant record carries a model and a usage block, and nothing in
//! this tool has ever read them. Aggregated they answer the questions a pile of
//! transcripts otherwise cannot: which models did the work, how much of the
//! context was served from cache, and where the time actually went.
//!
//! Cost is deliberately not built in. Prices change, a hardcoded table would go
//! stale silently, and a number that is quietly wrong is worse than no number —
//! so `--prices` takes a table from the user and cost appears only when it is
//! given one.

use crate::cli::Opts;
use crate::record::Record;
use crate::{dates, scan};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Default, Clone, Copy)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }

    /// What share of everything fed to the model came back from cache. The one
    /// number here with an obvious action attached to it.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let fed = self.input + self.cache_read + self.cache_write;
        (fed > 0).then(|| self.cache_read as f64 / fed as f64)
    }
}

#[derive(Default)]
pub struct Stats {
    pub sessions: HashSet<String>,
    pub projects: HashMap<String, usize>,
    pub models: HashMap<String, (usize, Tokens)>,
    pub tokens: Tokens,
    pub user: usize,
    pub assistant: usize,
    pub first: String,
    pub last: String,
}

impl Stats {
    fn merge(&mut self, o: Stats) {
        self.sessions.extend(o.sessions);
        for (k, v) in o.projects {
            *self.projects.entry(k).or_default() += v;
        }
        for (k, (n, t)) in o.models {
            let e = self.models.entry(k).or_default();
            e.0 += n;
            e.1.add(&t);
        }
        self.tokens.add(&o.tokens);
        self.user += o.user;
        self.assistant += o.assistant;
        span(&mut self.first, &mut self.last, &o.first);
        span(&mut self.first, &mut self.last, &o.last);
    }

    pub fn messages(&self) -> usize {
        self.user + self.assistant
    }
}

/// Widen the observed date range to include `ts`.
fn span(first: &mut String, last: &mut String, ts: &str) {
    if ts.is_empty() {
        return;
    }
    if first.is_empty() || ts < first.as_str() {
        *first = ts.to_owned();
    }
    if ts > last.as_str() {
        *last = ts.to_owned();
    }
}

pub fn collect(opts: &Opts) -> Stats {
    // A session id narrows the walk to the one file that holds it rather than
    // filtering every record in the corpus against it: the transcripts are
    // named by session, so the answer is in the filename.
    let files = if opts.session.is_empty() {
        scan::transcripts()
    } else {
        crate::show::resolve(&opts.session)
    };
    let queue = Arc::new(Mutex::new(files));
    let out: Arc<Mutex<Stats>> = Arc::new(Mutex::new(Stats::default()));

    std::thread::scope(|s| {
        for _ in 0..opts.jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            s.spawn(move || {
                let mut local = Stats::default();
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

fn read(path: &Path, opts: &Opts, into: &mut Stats) {
    let Ok(fh) = File::open(path) else { return };
    // One label for the whole transcript: see `projects::label`.
    let project = crate::projects::label(path).unwrap_or_else(|| crate::projects::UNKNOWN.into());
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

        into.sessions.insert(r.session_id().to_owned());
        *into.projects.entry(project.clone()).or_default() += 1;
        span(&mut into.first, &mut into.last, dates::day_of(r.timestamp()));

        if r.kind() == "user" {
            into.user += 1;
            continue;
        }
        into.assistant += 1;

        let t = usage(&v);
        into.tokens.add(&t);
        let model =
            v.pointer("/message/model").and_then(Value::as_str).unwrap_or("unknown").to_owned();
        let e = into.models.entry(model).or_default();
        e.0 += 1;
        e.1.add(&t);
    }
}

/// The usage block, read defensively: it is an internal field that has gained
/// members over time, and a missing one means zero rather than an error.
pub fn usage(v: &Value) -> Tokens {
    let Some(u) = v.pointer("/message/usage") else {
        return Tokens::default();
    };
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Tokens {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
    }
}

// ------------------------------------------------------------------- prices

/// Dollars per million tokens, per model, as supplied by the user.
pub type Prices = HashMap<String, Tokens4>;

#[derive(Clone, Copy, Default)]
pub struct Tokens4 {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

pub fn load_prices(path: &Path) -> Result<Prices, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    let Value::Object(map) = v else {
        return Err(format!("{}: expected an object of model → prices", path.display()));
    };
    Ok(map
        .into_iter()
        .map(|(model, p)| {
            let n = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            (
                model,
                Tokens4 {
                    input: n("input"),
                    output: n("output"),
                    cache_read: n("cache_read"),
                    cache_write: n("cache_write"),
                },
            )
        })
        .collect())
}

/// What the recorded usage would have cost at the given prices.
///
/// A model with no entry in the table contributes nothing and is reported as
/// unpriced, rather than being silently counted at zero.
pub fn cost(stats: &Stats, prices: &Prices) -> (f64, Vec<String>) {
    let mut total = 0.0;
    let mut unpriced = Vec::new();
    for (model, (_, t)) in &stats.models {
        let Some(p) = prices.get(model) else {
            unpriced.push(model.clone());
            continue;
        };
        let per = |n: u64, rate: f64| n as f64 / 1_000_000.0 * rate;
        total += per(t.input, p.input)
            + per(t.output, p.output)
            + per(t.cache_read, p.cache_read)
            + per(t.cache_write, p.cache_write);
    }
    unpriced.sort();
    (total, unpriced)
}

// ------------------------------------------------------------------ report

/// How many rows of the per-project table to print. The tail of a long tail is
/// one session each and says nothing.
const TOP: usize = 8;

pub fn report(w: &mut impl Write, s: &Stats, prices: Option<&Prices>) {
    let n = |count: usize, word: &str| {
        format!("{} {word}{}", thousands(count as u64), if count == 1 { "" } else { "s" })
    };
    let _ = writeln!(
        w,
        "{} · {} · {}",
        n(s.sessions.len(), "session"),
        n(s.messages(), "message"),
        n(s.projects.len(), "project"),
    );
    if !s.first.is_empty() {
        let _ = writeln!(w, "{} → {}", s.first, s.last);
    }
    let _ = writeln!(
        w,
        "{} yours · {} assistant",
        thousands(s.user as u64),
        thousands(s.assistant as u64)
    );

    let mut models: Vec<(&String, &(usize, Tokens))> = s.models.iter().collect();
    models.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(b.0)));
    if !models.is_empty() {
        let width = models.iter().map(|(m, _)| m.chars().count()).max().unwrap_or(5).min(34);
        let _ = writeln!(
            w,
            "\nMODEL{} replies    input   output    cached",
            " ".repeat(width.saturating_sub(5))
        );
        for (model, (n, t)) in &models {
            let _ = writeln!(
                w,
                "{:width$} {:>7} {:>8} {:>8} {:>9}",
                crate::output::elide(model, width),
                thousands(*n as u64),
                short(t.input),
                short(t.output),
                short(t.cache_read),
            );
        }
    }

    let t = &s.tokens;
    let _ = writeln!(w, "\nTOKENS");
    for (label, n) in [
        ("input", t.input),
        ("output", t.output),
        ("cache read", t.cache_read),
        ("cache write", t.cache_write),
    ] {
        let _ = writeln!(w, "  {label:<12} {:>8}", short(n));
    }
    if let Some(rate) = t.cache_hit_rate() {
        let _ = writeln!(w, "  {:<12} {:>7.1}%", "from cache", rate * 100.0);
    }

    if let Some(prices) = prices {
        let (total, unpriced) = cost(s, prices);
        let _ = writeln!(w, "\nCOST\n  {:<12} {:>8}", "estimated", format!("${total:.2}"));
        if !unpriced.is_empty() {
            let _ = writeln!(w, "  not in the price table: {}", unpriced.join(", "));
        }
    }

    let mut projects: Vec<(&String, &usize)> = s.projects.iter().collect();
    projects.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    if !projects.is_empty() {
        let _ = writeln!(w, "\nPROJECTS");
        for (name, n) in projects.iter().take(TOP) {
            let _ = writeln!(w, "  {:>8}  {name}", thousands(**n as u64));
        }
        if projects.len() > TOP {
            let _ = writeln!(w, "  {:>8}  … {} more", "", projects.len() - TOP);
        }
    }
}

pub fn report_json(w: &mut impl Write, s: &Stats, prices: Option<&Prices>) {
    let mut models: Vec<Value> = s
        .models
        .iter()
        .map(|(model, (n, t))| {
            json!({"model": model, "replies": n, "input": t.input, "output": t.output,
                   "cache_read": t.cache_read, "cache_write": t.cache_write})
        })
        .collect();
    models.sort_by_key(|m| m["model"].as_str().unwrap_or("").to_owned());

    let mut projects: Vec<Value> =
        s.projects.iter().map(|(name, n)| json!({"project": name, "messages": n})).collect();
    projects.sort_by_key(|p| p["project"].as_str().unwrap_or("").to_owned());

    let mut out = json!({
        "sessions": s.sessions.len(),
        "messages": s.messages(),
        "user": s.user,
        "assistant": s.assistant,
        "first": s.first,
        "last": s.last,
        "tokens": {
            "input": s.tokens.input,
            "output": s.tokens.output,
            "cache_read": s.tokens.cache_read,
            "cache_write": s.tokens.cache_write,
            "cache_hit_rate": s.tokens.cache_hit_rate(),
        },
        "models": models,
        "projects": projects,
    });
    if let Some(prices) = prices {
        let (total, unpriced) = cost(s, prices);
        out["cost"] = json!({"estimated": total, "unpriced": unpriced});
    }
    let _ = writeln!(w, "{out}");
}

/// `1234567` as `1,234,567`. Counts are compared against each other, and digits
/// in an unbroken run of seven are not.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Token counts run to billions, where the exact figure is noise and the
/// magnitude is the point.
pub fn short(n: u64) -> String {
    match n {
        0..=9_999 => thousands(n),
        10_000..=999_999 => format!("{:.0}K", n as f64 / 1_000.0),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1_000_000.0),
        _ => format!("{:.1}B", n as f64 / 1_000_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separates_every_third_digit() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn short_switches_unit_with_magnitude() {
        assert_eq!(short(999), "999");
        assert_eq!(short(9_999), "9,999");
        assert_eq!(short(12_400), "12K");
        assert_eq!(short(12_600), "13K");
        assert_eq!(short(1_500_000), "1.5M");
        assert_eq!(short(2_000_000_000), "2.0B");
    }

    #[test]
    fn the_cache_rate_is_a_share_of_everything_fed_in() {
        let t = Tokens { input: 100, output: 999, cache_read: 300, cache_write: 100 };
        assert_eq!(t.cache_hit_rate(), Some(0.6), "output is not fed in, so it does not count");
        assert_eq!(Tokens::default().cache_hit_rate(), None, "nothing fed in has no rate");
    }

    #[test]
    fn usage_reads_the_four_counters_and_defaults_the_rest() {
        let v = json!({"message": {"usage": {"input_tokens": 5, "output_tokens": 7,
                                             "cache_read_input_tokens": 11}}});
        let t = usage(&v);
        assert_eq!((t.input, t.output, t.cache_read, t.cache_write), (5, 7, 11, 0));
        let empty = usage(&json!({"message": {}}));
        assert_eq!((empty.input, empty.output), (0, 0));
    }

    fn priced() -> (Stats, Prices) {
        let mut s = Stats::default();
        s.models.insert(
            "m1".into(),
            (1, Tokens { input: 1_000_000, output: 1_000_000, cache_read: 0, cache_write: 0 }),
        );
        s.models.insert(
            "m2".into(),
            (1, Tokens { input: 2_000_000, output: 0, cache_read: 0, cache_write: 0 }),
        );
        let mut p = Prices::new();
        p.insert("m1".into(), Tokens4 { input: 3.0, output: 15.0, ..Default::default() });
        (s, p)
    }

    #[test]
    fn cost_is_dollars_per_million_tokens() {
        let (s, p) = priced();
        let (total, _) = cost(&s, &p);
        assert!((total - 18.0).abs() < 1e-9, "1M in at $3 + 1M out at $15: {total}");
    }

    /// A model the table does not price contributes nothing and is named, so
    /// the total is never quietly short.
    #[test]
    fn an_unpriced_model_is_reported_rather_than_counted_as_free() {
        let (s, p) = priced();
        let (_, unpriced) = cost(&s, &p);
        assert_eq!(unpriced, ["m2"]);
    }

    #[test]
    fn merging_widens_the_date_range_from_both_ends() {
        let mut a =
            Stats { first: "2026-05-01".into(), last: "2026-05-09".into(), ..Default::default() };
        a.merge(Stats {
            first: "2026-04-01".into(),
            last: "2026-04-02".into(),
            ..Default::default()
        });
        a.merge(Stats {
            first: "2026-09-01".into(),
            last: "2026-09-02".into(),
            ..Default::default()
        });
        assert_eq!((a.first.as_str(), a.last.as_str()), ("2026-04-01", "2026-09-02"));
    }

    #[test]
    fn merging_sums_the_counters_per_key() {
        let mut a = Stats::default();
        a.models.insert("m".into(), (1, Tokens { input: 10, ..Default::default() }));
        a.projects.insert("p".into(), 2);
        a.sessions.insert("s1".into());

        let mut b = Stats::default();
        b.models.insert("m".into(), (2, Tokens { input: 5, ..Default::default() }));
        b.projects.insert("p".into(), 3);
        b.sessions.insert("s1".into());
        b.sessions.insert("s2".into());

        a.merge(b);
        assert_eq!(a.models["m"].0, 3);
        assert_eq!(a.models["m"].1.input, 15);
        assert_eq!(a.projects["p"], 5);
        assert_eq!(a.sessions.len(), 2, "the same session seen twice is one session");
    }
}
