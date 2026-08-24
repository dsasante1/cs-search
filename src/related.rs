//! `cs related <session-id>` — other sessions about the same thing.
//!
//! Work on one problem scatters across sessions, and nothing joins them up: the
//! session that hit the bug and the session that fixed it share a subject but
//! not a word you would think to search for. This looks for the words they do
//! share.
//!
//! The measure is ordinary and old: a term is worth something in proportion to
//! how *rare* it is across the corpus, and two sessions are related in
//! proportion to the rare terms they have in common. That has one property
//! worth the whole design — it needs no list of stopwords. "the" appears in
//! every session, so `ln(sessions / sessions)` is zero and it counts for
//! nothing on its own; nobody has to maintain a list of words to ignore, and
//! there is no threshold tuned by hand to go stale.
//!
//! What it is not: an understanding of either session. It is a claim about
//! vocabulary, so the words that earned each result are printed beside it and
//! you can see immediately when they are the wrong ones. Only conversation text
//! is read — tool calls and their output are full of paths and file contents
//! that would drown the subject in incidentals.
//!
//! It is the one command here with no prefilter to hide behind: which terms are
//! rare is not knowable until every record has been read, so every record is
//! read. Expect it to cost about what `--thread` costs.

use crate::output::{fixed, CYAN, DIM, RESET};
use crate::record::{BlockOpts, Record};
use crate::{scan, sessions, show};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Terms shorter than this are mostly noise; longer than this are mostly
/// base64, hashes and minified code.
const MIN_LEN: usize = 3;
const MAX_LEN: usize = 40;

/// A word said once in a session is as likely a typo as a subject.
const MIN_COUNT: usize = 2;

/// How many of the target's terms to carry into the sweep. Only a bound on
/// memory: the sweep costs the same either way.
const MAX_TERMS: usize = 2000;

/// How many of the terms behind a result to name.
const SHOWN_TERMS: usize = 6;

pub const DEFAULT_LIMIT: usize = 10;

/// Conversation only. See the note above on why tool blocks are excluded.
const TEXT: BlockOpts = BlockOpts { thinking: false, tools: false };

/// One session as the sweep saw it.
struct Seen {
    path: PathBuf,
    sid: String,
    project: String,
    last: String,
    /// Indices into the candidate term list that this session used.
    present: HashSet<u32>,
    /// How many words the session contains, for the length correction below.
    words: usize,
}

pub struct Related {
    pub sid: String,
    pub project: String,
    pub last: String,
    pub title: String,
    /// How many weighted terms the two sessions have in common. Counted before
    /// the list below is cut, so the column keeps meaning something once there
    /// are more than `SHOWN_TERMS` of them.
    pub shared: usize,
    pub terms: Vec<String>,
    pub score: f64,
}

/// Split text the way a reader would: on everything that is not part of a word.
/// `_` and `-` are kept, because `cache_read` and `ui-overhaul` are single
/// terms in a corpus like this one.
fn split(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
}

/// Lowercase `raw` into `buf`, or say it is not a word worth counting.
///
/// Anything with no letter in it is dropped: a corpus of transcripts is full of
/// line numbers, byte counts and hex, and none of them are subjects.
fn word(raw: &str, buf: &mut String) -> bool {
    let n = raw.chars().count();
    if !(MIN_LEN..=MAX_LEN).contains(&n) || !raw.chars().any(char::is_alphabetic) {
        return false;
    }
    buf.clear();
    buf.extend(raw.chars().flat_map(char::to_lowercase));
    true
}

/// Every conversation record in a file, as decoded text.
fn text_of(path: &Path, mut each: impl FnMut(&Record, String)) {
    let Ok(fh) = File::open(path) else { return };
    for line in BufReader::with_capacity(1 << 20, fh).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let r = Record::new(&v);
        if !r.is_conversation() || r.is_meta() {
            continue;
        }
        for block in r.blocks(TEXT) {
            each(&r, block);
        }
    }
}

/// The terms this session actually uses, commonest first.
pub fn candidates(path: &Path) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut buf = String::new();
    text_of(path, |_, block| {
        for raw in split(&block) {
            if word(raw, &mut buf) {
                *counts.entry(buf.clone()).or_default() += 1;
            }
        }
    });

    let mut v: Vec<(String, usize)> = counts.into_iter().filter(|(_, n)| *n >= MIN_COUNT).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(MAX_TERMS);
    v.into_iter().map(|(t, _)| t).collect()
}

/// Which of the candidate terms each session in the corpus uses.
fn sweep(terms: &[String], jobs: usize) -> Vec<Seen> {
    let index: HashMap<&str, u32> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i as u32))
        .collect();

    let queue = Arc::new(Mutex::new(scan::transcripts()));
    let out: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|s| {
        for _ in 0..jobs {
            let queue = Arc::clone(&queue);
            let out = Arc::clone(&out);
            let index = &index;
            s.spawn(move || {
                let mut local = Vec::new();
                let mut buf = String::new();
                loop {
                    let next = queue.lock().unwrap().pop();
                    let Some(path) = next else { break };

                    let mut seen = Seen {
                        sid: path.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_owned(),
                        project: String::new(),
                        last: String::new(),
                        present: HashSet::new(),
                        words: 0,
                        path,
                    };
                    let mut any = false;
                    text_of(&seen.path, |r, block| {
                        any = true;
                        if seen.project.is_empty() {
                            let p = r.cwd().rsplit('/').next().unwrap_or("?");
                            seen.project = if p.is_empty() { "?" } else { p }.to_owned();
                        }
                        if !r.timestamp().is_empty() {
                            seen.last = crate::dates::day_of(r.timestamp()).to_owned();
                        }
                        for raw in split(&block) {
                            if word(raw, &mut buf) {
                                seen.words += 1;
                                if let Some(&i) = index.get(buf.as_str()) {
                                    seen.present.insert(i);
                                }
                            }
                        }
                    });
                    // Every session that has text counts toward the corpus size,
                    // whether or not it shares anything: that denominator is
                    // what makes a term rare.
                    if any {
                        local.push(seen);
                    }
                }
                out.lock().unwrap().append(&mut local);
            });
        }
    });

    Arc::try_unwrap(out).ok().unwrap().into_inner().unwrap()
}

/// How much each term is worth: `ln(sessions / sessions using it)`.
///
/// A term every session uses is worth exactly nothing, which is what makes the
/// stopword list unnecessary.
fn weights(terms: &[String], seen: &[Seen]) -> Vec<f64> {
    let n = seen.len() as f64;
    let mut df = vec![0usize; terms.len()];
    for s in seen {
        for &i in &s.present {
            df[i as usize] += 1;
        }
    }
    df.into_iter()
        .map(|d| if d == 0 { 0.0 } else { (n / d as f64).ln() })
        .collect()
}

/// Rank the corpus against one session's vocabulary.
fn rank(terms: &[String], seen: &[Seen], target: &str, limit: usize) -> Vec<Related> {
    let idf = weights(terms, seen);
    let mut out: Vec<Related> = seen
        .iter()
        .filter(|s| s.sid != target)
        .filter_map(|s| {
            // Terms carrying no weight are dropped rather than counted: a
            // result whose only evidence is words everyone uses is not one.
            let mut shared: Vec<(usize, f64)> = s
                .present
                .iter()
                .map(|&i| (i as usize, idf[i as usize]))
                .filter(|(_, w)| *w > 0.0)
                .collect();
            if shared.is_empty() {
                return None;
            }
            shared.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| terms[a.0].cmp(&terms[b.0]))
            });
            // A long session shares more of everything — including with
            // sessions it has nothing to do with — so the total is divided by
            // the square root of how many words it holds. Without this the
            // ranking measures length as much as subject.
            let raw: f64 = shared.iter().map(|(_, w)| w).sum();
            let score = raw / (s.words as f64).sqrt().max(1.0);
            Some(Related {
                sid: crate::record::take_chars(&s.sid, 8).to_owned(),
                project: s.project.clone(),
                last: s.last.clone(),
                title: sessions::last_title(&s.path).unwrap_or_default(),
                shared: shared.len(),
                terms: shared
                    .iter()
                    .take(SHOWN_TERMS)
                    .map(|(i, _)| terms[*i].clone())
                    .collect(),
                score,
            })
        })
        .collect();

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.last.cmp(&a.last))
            .then_with(|| a.sid.cmp(&b.sid))
    });
    out.truncate(limit);
    out
}

pub fn print(w: &mut impl Write, rows: &[Related], color: bool) {
    let (c, d, z) = if color { (CYAN, DIM, RESET) } else { ("", "", "") };
    let width = rows
        .iter()
        .map(|r| r.project.chars().count())
        .max()
        .unwrap_or(8)
        .clamp(8, 20);
    let weights: Vec<String> = rows.iter().map(|r| format!("{:.2}", r.score)).collect();
    let digits = weights.iter().map(|n| n.len()).max().unwrap_or(6).max(6);
    // weight + 2 + day(10) + 2 + project + 2 + sid(8) + 2
    let indent = " ".repeat(26 + digits + width);

    // The column is named because the number needs it: a weight is the sum of
    // how rare the shared words are, which orders the list and means nothing on
    // its own. Naming it is cheaper than letting it be read as a percentage.
    let _ = writeln!(
        w,
        "{d}{:>digits$}  {:<10}  {}  {:8}  title{z}",
        "weight",
        "last",
        crate::output::pad("project", width),
        "session",
    );

    for (r, weight) in rows.iter().zip(&weights) {
        let _ = writeln!(
            w,
            "{d}{weight:>digits$}{z}  {d}{:<10}{z}  {c}{}{z}  {d}{}{z}  {}",
            r.last,
            fixed(&r.project, width),
            r.sid,
            r.title,
        );
        // The evidence, under the claim. A result whose words are obviously the
        // wrong ones can be dismissed without opening it.
        let rest = match r.shared.saturating_sub(r.terms.len()) {
            0 => String::new(),
            n => format!("  +{n} more"),
        };
        let _ = writeln!(w, "{indent}{d}↳ {}{rest}{z}", r.terms.join(", "));
    }
}

pub fn print_json(w: &mut impl Write, rows: &[Related]) {
    for r in rows {
        let _ = writeln!(
            w,
            "{}",
            json!({"session": r.sid, "project": r.project, "last": r.last,
                   "title": r.title, "shared": r.shared, "terms": r.terms,
                   "weight": r.score})
        );
    }
}

/// `cs related <id> [--limit n] [--json]`.
pub fn run(id: &str, limit: usize, json: bool, jobs: usize) -> i32 {
    let Some(path) = show::pick(id, "reading") else {
        return 1;
    };
    let terms = candidates(&path);
    if terms.is_empty() {
        eprintln!("'{id}' says too little to compare against");
        return 1;
    }

    let sid = path.file_stem().and_then(|x| x.to_str()).unwrap_or("").to_owned();
    let seen = sweep(&terms, jobs);
    let rows = rank(&terms, &seen, &sid, limit);
    if rows.is_empty() {
        eprintln!("nothing else in the corpus shares its distinctive words");
        return 1;
    }

    let stdout = std::io::stdout();
    let mut w = std::io::BufWriter::new(stdout.lock());
    if json {
        print_json(&mut w, &rows);
    } else {
        let title = sessions::last_title(&path).unwrap_or_default();
        let short = crate::record::take_chars(&sid, 8);
        if title.is_empty() {
            let _ = writeln!(w, "related to {short}\n");
        } else {
            let _ = writeln!(w, "related to {short} · {title}\n");
        }
        print(&mut w, &rows, crate::output::is_tty());
    }
    let _ = w.flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(sid: &str, present: &[u32]) -> Seen {
        sized(sid, present, 1)
    }

    /// The same, for the tests that care how much the session says.
    fn sized(sid: &str, present: &[u32], words: usize) -> Seen {
        Seen {
            path: PathBuf::from(format!("/tmp/{sid}.jsonl")),
            sid: sid.to_owned(),
            project: "app".into(),
            last: "2026-08-20".into(),
            present: present.iter().copied().collect(),
            words,
        }
    }

    fn terms(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("term{i}")).collect()
    }

    #[test]
    fn words_are_split_on_everything_that_is_not_one() {
        let got: Vec<&str> = split("cache_read, ui-overhaul; foo.bar (baz)")
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(got, ["cache_read", "ui-overhaul", "foo", "bar", "baz"]);
    }

    #[test]
    fn a_word_is_lowercased_and_bounded() {
        let mut buf = String::new();
        assert!(word("Redis", &mut buf) && buf == "redis");
        assert!(word("café", &mut buf) && buf == "café", "not ascii-only");
        assert!(!word("a", &mut buf), "too short");
        assert!(!word(&"x".repeat(41), &mut buf), "too long");
    }

    /// Line numbers, byte counts and hex offsets are not subjects.
    #[test]
    fn a_token_with_no_letter_in_it_is_not_a_word() {
        let mut buf = String::new();
        assert!(!word("12345", &mut buf));
        assert!(!word("---", &mut buf));
        // One letter is enough, though: a version or a hash name is a subject.
        assert!(word("sha256", &mut buf) && buf == "sha256");
    }

    /// The property the whole design rests on: a term everybody uses is worth
    /// nothing, so no list of stopwords has to be kept anywhere.
    #[test]
    fn a_term_every_session_uses_carries_no_weight() {
        let corpus = vec![seen("a", &[0]), seen("b", &[0]), seen("c", &[0])];
        let w = weights(&terms(1), &corpus);
        assert_eq!(w[0], 0.0);
    }

    #[test]
    fn a_rare_term_outweighs_a_common_one() {
        // term0 is everywhere, term1 is in one session out of four.
        let corpus = vec![
            seen("a", &[0, 1]),
            seen("b", &[0]),
            seen("c", &[0]),
            seen("d", &[0]),
        ];
        let w = weights(&terms(2), &corpus);
        assert!(w[1] > w[0], "{w:?}");
    }

    #[test]
    fn sessions_sharing_only_universal_words_are_not_related() {
        let corpus = vec![seen("target", &[0]), seen("other", &[0])];
        assert!(rank(&terms(1), &corpus, "target", 10).is_empty());
    }

    #[test]
    fn the_session_asked_about_is_never_related_to_itself() {
        let corpus = vec![
            seen("target", &[0, 1]),
            seen("other", &[0, 1]),
            seen("third", &[0]),
        ];
        let out = rank(&terms(2), &corpus, "target", 10);
        assert!(out.iter().all(|r| r.sid != "target"), "{:?}", out.len());
    }

    #[test]
    fn results_are_ordered_by_the_weight_of_what_they_share() {
        // term2 is the rarest, so the session that shares it ranks first even
        // though the other shares more terms.
        let corpus = vec![
            seen("target", &[0, 1, 2]),
            seen("many", &[0, 1]),
            seen("rare", &[2]),
            seen("filler1", &[0, 1]),
            seen("filler2", &[0, 1]),
            seen("filler3", &[0, 1]),
        ];
        let out = rank(&terms(3), &corpus, "target", 10);
        assert_eq!(out[0].sid, "rare", "rarity beats count");
        assert!(out[0].score > out[1].score);
    }

    #[test]
    fn the_terms_behind_a_result_are_named_rarest_first() {
        // term0 is in four of the five sessions, term1 in two: both carry some
        // weight, and the rarer one has more of it.
        let corpus = vec![
            seen("target", &[0, 1]),
            seen("other", &[0, 1]),
            seen("filler1", &[0]),
            seen("filler2", &[0]),
            seen("filler3", &[]),
        ];
        let out = rank(&terms(2), &corpus, "target", 10);
        assert_eq!(out[0].terms, ["term1", "term0"], "the rarer one leads");
        assert_eq!(out[0].shared, 2, "and both are counted");
    }

    /// Otherwise the longest sessions crowd the top of every list, having
    /// shared a little of everything with everybody.
    #[test]
    fn a_long_session_is_not_related_merely_by_being_long() {
        let corpus = vec![
            sized("target", &[0, 1], 100),
            sized("short", &[0, 1], 100),
            sized("sprawling", &[0, 1], 40_000),
            sized("filler", &[], 100),
        ];
        let out = rank(&terms(2), &corpus, "target", 10);
        assert_eq!(out[0].sid, "short", "same words, less padding");
        assert!(out[0].score > out[1].score);
    }

    #[test]
    fn the_limit_is_honoured() {
        let corpus = vec![
            seen("target", &[0]),
            seen("a", &[0]),
            seen("b", &[0]),
            seen("filler", &[]),
        ];
        assert_eq!(rank(&terms(1), &corpus, "target", 1).len(), 1);
    }
}
