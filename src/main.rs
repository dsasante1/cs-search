//! cs — search Claude Code conversation history across every session and project.

mod cli;
mod completions;
mod dates;
mod export;
mod files;
mod interactive;
mod output;
mod picker;
mod projects;
mod prompts;
mod record;
mod resume;
mod scan;
mod sessions;
mod show;
mod stats;

use cli::{Opts, Parsed, USAGE};
use output::Row;
use regex::Regex;
use std::ffi::OsString;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

fn main() {
    // args_os, so a pattern that is not valid UTF-8 is reported by the parser
    // rather than panicking before it ever gets there.
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        print!("{USAGE}");
        exit(0);
    }

    // Subcommands are matched before flags, as in the original script.
    let sub = |i: usize| args.get(i).and_then(|a| a.to_str()).unwrap_or("");
    match sub(0) {
        "show" => exit(show_command(&args)),
        "sessions" => {
            let rows = sessions::run(sub(1), Opts::default().jobs);
            let stdout = std::io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            output::print_flat(&mut w, &rows, output::is_tty(), None);
            let _ = w.flush();
            exit(0);
        }
        "projects" => exit(projects::run(sub(1), Opts::default().jobs)),
        "files" => exit(files_command(&args)),
        "stats" => exit(stats_command(&args)),
        "export" => exit(export_command(&args)),
        "resume" => exit(resume::run(sub(1))),
        // Internal, and spelled so: these exist for the picker's key bindings to
        // call back into, and are not part of the CLI.
        "__rows" => exit(picker::rows(Path::new(sub(1)), sub(2))),
        "__toggle" => {
            picker::toggle(Path::new(sub(1)), sub(2), sub(3));
            exit(0);
        }
        "__header" => {
            println!("{}", picker::header(Path::new(sub(1)), sub(2)));
            exit(0);
        }
        "completions" => {
            let stdout = std::io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            let code = match completions::write(&mut w, sub(1)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    2
                }
            };
            let _ = w.flush();
            exit(code);
        }
        "-h" | "--help" => {
            print!("{USAGE}");
            exit(0);
        }
        _ => {}
    }

    let opts = match cli::parse(&args) {
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            exit(0);
        }
        Ok(Parsed::Search(o)) => o,
        Err(msg) => {
            if msg.is_empty() {
                print!("{USAGE}");
                exit(2);
            }
            eprintln!("{msg}");
            exit(2);
        }
    };

    let (re, note) = match scan::compile(&opts) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            exit(2);
        }
    };
    if let Some(note) = note {
        eprintln!("{note}");
    }

    let rows = if opts.prompts {
        match prompts::run(&opts, &re) {
            Ok(rows) => rows,
            Err(msg) => {
                eprintln!("{msg}");
                exit(1);
            }
        }
    } else {
        let hits = scan::search(&opts, &re);
        if opts.files_only {
            if hits.files.is_empty() {
                eprintln!("no matches");
                exit(1);
            }
            // Unlike the shell version, these are files that actually produced a
            // match after decoding, not files whose raw JSON happened to contain it.
            let stdout = std::io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            for f in &hits.files {
                let _ = writeln!(w, "{}", f.display());
            }
            // exit() skips destructors, so the buffer has to be flushed by hand.
            let _ = w.flush();
            exit(0);
        }
        hits.rows
    };

    exit(present(&opts, &rows, &re));
}

/// Choose how to show the results.
///
/// A pipe always gets the original one-line-per-match format, so scripts built
/// on this are unaffected by everything below. A terminal gets the picker, since
/// that is the interface people actually want and requiring `-i` to reach it
/// only hid it; `--plain` opts back out, and grouped output is what it falls
/// back to when fzf is not installed.
fn present(opts: &Opts, rows: &[Row], re: &Regex) -> i32 {
    if rows.is_empty() {
        return no_matches(opts, re);
    }
    if let Some(hint) = output::regex_hint(opts, rows.len()) {
        eprintln!("{hint}");
    }

    let stdout = std::io::stdout();
    if opts.json {
        let mut w = BufWriter::new(stdout.lock());
        output::print_json(&mut w, rows);
        let _ = w.flush();
        return 0;
    }

    let tty = output::is_tty();
    if opts.interactive || (tty && !opts.plain) {
        match interactive::run(rows, opts, re) {
            Some(code) => return code,
            None if opts.interactive => {
                eprintln!("cs -i needs fzf on PATH");
                return 127;
            }
            None => eprintln!("fzf is not on PATH; printing results instead"),
        }
    }

    if output::stderr_is_tty() {
        eprintln!("{}", output::summary(rows));
    }
    let mut w = BufWriter::new(stdout.lock());
    let hl = tty.then_some(re);
    if opts.grouping.applies(tty) {
        output::print_grouped(&mut w, rows, tty, hl);
    } else {
        output::print_flat(&mut w, rows, tty, hl);
    }
    let _ = w.flush();
    0
}

/// Report an empty result set, and say what to do about it.
///
/// Five filters can each independently empty a search, and "no matches" named
/// none of them. Each active filter is dropped in turn and the search re-run, so
/// the report can say which one cost what. That is up to five extra scans — paid
/// only when the result was empty, and only for a human at a terminal, so
/// scripts see the same one-line answer at the same speed as before.
fn no_matches(opts: &Opts, re: &Regex) -> i32 {
    eprintln!("no matches for '{}'", opts.pattern);
    if !output::stderr_is_tty() || opts.prompts {
        return 1;
    }

    for l in no_match_report(&widenings(opts, re), &widening_hints(opts)) {
        eprintln!("{l}");
    }
    1
}

/// The advice under "no matches", as lines. Split out from printing it so the
/// wording and alignment can be tested without a corpus or a terminal.
fn no_match_report(widened: &[(usize, String)], hints: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if !widened.is_empty() {
        let digits = widened.iter().map(|(n, _)| n.to_string().len()).max().unwrap_or(1);
        out.push(String::new());
        for (n, flag) in widened {
            let plural = if *n == 1 { "match " } else { "matches" };
            out.push(format!("  {n:>digits$} {plural} without  {flag}"));
        }
    }
    if !hints.is_empty() {
        out.push(String::new());
        out.extend(hints.iter().map(|h| format!("  {h}")));
    }
    out
}

/// Which searches to re-run: one per filter that is actually narrowing this
/// search, each with that one filter lifted.
///
/// Kept separate from running them so the choice can be tested without a corpus.
fn probe_set(opts: &Opts) -> Vec<(String, Opts)> {
    let mut probes: Vec<(String, Opts)> = Vec::new();
    if !opts.project.is_empty() {
        probes.push((
            format!("-P {}", opts.project),
            Opts { project: String::new(), ..opts.clone() },
        ));
    }
    if !opts.role.is_empty() {
        probes.push((
            format!("-r {}", opts.role),
            Opts { role: String::new(), ..opts.clone() },
        ));
    }
    if !opts.since.is_empty() {
        probes.push((
            format!("-s {}", opts.since),
            Opts { since: String::new(), ..opts.clone() },
        ));
    }
    if !opts.until.is_empty() {
        probes.push((
            format!("-u {}", opts.until),
            Opts { until: String::new(), ..opts.clone() },
        ));
    }
    if !opts.branch.is_empty() {
        probes.push((
            format!("-b {}", opts.branch),
            Opts { branch: String::new(), ..opts.clone() },
        ));
    }
    if opts.no_sub {
        probes.push(("-n".into(), Opts { no_sub: false, ..opts.clone() }));
    }
    if !opts.thinking {
        probes.push(("-T".into(), Opts { thinking: true, ..opts.clone() }));
    }
    probes
}

/// Hints that need no second search: ways to widen the corpus rather than lift
/// a filter.
fn widening_hints(opts: &Opts) -> Vec<String> {
    let mut hints = Vec::new();
    if !opts.tools {
        hints.push("-t  also searches tool calls and results".into());
    }
    if !opts.fixed && opts.pattern.chars().any(|c| "\\.+*?()[]{}|^$".contains(c)) {
        hints.push("-F  searches the pattern literally".into());
    }
    hints
}

/// How many matches each active filter is costing, for the ones costing any.
fn widenings(opts: &Opts, re: &Regex) -> Vec<(usize, String)> {
    probe_set(opts)
        .into_iter()
        .filter_map(|(flag, o)| {
            let found = scan::search(&o, re).rows.len();
            (found > 0).then_some((found, flag))
        })
        .collect()
}

/// `cs export <id> [--format md|html|json] [-r user|assistant]`.
///
/// Hand-parsed for the same reason `show` is: it takes an id and a couple of
/// flags of its own, and has never shared the search grammar.
fn export_command(args: &[OsString]) -> i32 {
    let mut id = String::new();
    let mut role = String::new();
    let mut format = export::Format::Markdown;
    let mut rest = args.iter().skip(1);

    while let Some(arg) = rest.next() {
        match arg.to_str().unwrap_or("") {
            "--format" | "-f" => {
                let v = rest.next().and_then(|v| v.to_str()).unwrap_or("");
                match export::Format::parse(v) {
                    Ok(f) => format = f,
                    Err(e) => {
                        eprintln!("{e}");
                        return 2;
                    }
                }
            }
            "--role" | "-r" => {
                role = rest.next().and_then(|v| v.to_str()).unwrap_or("").to_owned();
                if role != "user" && role != "assistant" {
                    eprintln!("--role must be 'user' or 'assistant', got: {role}");
                    return 2;
                }
            }
            value if id.is_empty() => id = value.to_owned(),
            _ => {}
        }
    }
    export::run(&id, &role, format)
}

/// `cs stats [-P proj] [-b branch] [-s date] [-u date] [--prices f] [--json]`.
///
/// Hand-parsed because it is the one search-shaped command with no pattern:
/// `cli::parse` treats a missing one as a usage error, which is right
/// everywhere else.
fn stats_command(args: &[OsString]) -> i32 {
    let mut o = Opts::default();
    let mut json = false;
    let mut prices_at: Option<PathBuf> = None;
    let mut rest = args.iter().skip(1);

    while let Some(arg) = rest.next() {
        let mut take = || rest.next().and_then(|v| v.to_str()).unwrap_or("").to_owned();
        match arg.to_str().unwrap_or("") {
            "-P" | "--project" => o.project = take().to_lowercase(),
            "-b" | "--branch" => o.branch = take().to_lowercase(),
            "-j" | "--jobs" => o.jobs = take().parse().unwrap_or(o.jobs).max(1),
            "--json" => json = true,
            "--prices" => prices_at = Some(PathBuf::from(take())),
            "-s" | "--since" | "-u" | "--until" => {
                let flag = arg.to_str().unwrap_or("");
                let spec = take();
                match dates::resolve(&spec) {
                    Ok(d) if flag.starts_with("-s") || flag == "--since" => o.since = d,
                    Ok(d) => o.until = d,
                    Err(e) => {
                        eprintln!("{flag}: {e}");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("unknown option: {other}");
                return 2;
            }
        }
    }

    let prices = match prices_at.as_deref().map(stats::load_prices) {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => {
            eprintln!("{e}");
            return 2;
        }
        None => None,
    };

    let s = stats::collect(&o);
    if s.messages() == 0 {
        eprintln!("no messages matched");
        return 1;
    }
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    if json {
        stats::report_json(&mut w, &s, prices.as_ref());
    } else {
        stats::report(&mut w, &s, prices.as_ref());
    }
    let _ = w.flush();
    0
}

/// `cs files <pattern>` — the search flags, applied to paths that were worked
/// on rather than to anything anyone said.
///
/// Shares the flag grammar so `-P`, `-b`, `-s`, `-u` and `-F` mean here exactly
/// what they mean in a search; the flags that shape a *text* result (-C, -t,
/// grouping) have nothing to act on and are ignored.
fn files_command(args: &[OsString]) -> i32 {
    let opts = match cli::parse(&args[1..]) {
        Ok(Parsed::Search(o)) => o,
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return 0;
        }
        Err(msg) => {
            eprintln!("{}", if msg.is_empty() { "cs files <pattern>".into() } else { msg });
            return 2;
        }
    };
    let (re, note) = match scan::compile(&opts) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    if let Some(note) = note {
        eprintln!("{note}");
    }

    let hits = files::run(&opts, &re);
    if hits.is_empty() {
        eprintln!("no files matching '{}'", opts.pattern);
        return 1;
    }
    if output::stderr_is_tty() {
        eprintln!("{}", files::summary(&hits));
    }
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    if opts.json {
        files::print_json(&mut w, &hits);
    } else {
        files::print(&mut w, &hits, output::is_tty());
    }
    let _ = w.flush();
    0
}

/// `cs show <id> [-r <role>] [--highlight <pat>] [--at <pat>] [--color] [--no-pager]`.
///
/// Hand-parsed rather than run through `cli::parse`, because `show` has never
/// shared the search flag grammar and the extra flags here are mostly for the
/// picker to pass to itself.
fn show_command(args: &[OsString]) -> i32 {
    let mut id = String::new();
    let mut o = show::ShowOpts { pager: true, ..Default::default() };
    let mut rest = args.iter().skip(1);

    while let Some(arg) = rest.next() {
        #[allow(clippy::redundant_closure_call)]
        let mut take = || rest.next().and_then(|v| v.to_str()).and_then(show::pattern);
        match arg.to_str().unwrap_or("") {
            "--highlight" => o.highlight = take(),
            "--at" => o.at = take(),
            "--role" | "-r" => {
                o.role = rest.next().and_then(|v| v.to_str()).unwrap_or("").to_owned();
                // Without this a typo silently produced the *unfiltered*
                // transcript, which looks exactly like a filtered one.
                if o.role != "user" && o.role != "assistant" {
                    eprintln!("--role must be 'user' or 'assistant', got: {}", o.role);
                    return 2;
                }
            }
            "--color" => o.color = true,
            "--no-pager" => o.pager = false,
            value if id.is_empty() => id = value.to_owned(),
            _ => {}
        }
    }
    show::run_with(&id, &o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(opts: &Opts) -> Vec<String> {
        probe_set(opts).into_iter().map(|(f, _)| f).collect()
    }

    #[test]
    fn an_unfiltered_search_has_nothing_to_widen() {
        // Nothing was narrowing it, so an empty result costs no extra scans.
        let bare = Opts { pattern: "needle".into(), ..Default::default() };
        assert!(probe_set(&bare).is_empty());
    }

    #[test]
    fn every_active_filter_becomes_one_probe() {
        let narrow = Opts {
            pattern: "needle".into(),
            project: "dashqard".into(),
            role: "user".into(),
            since: "2026-08-01".into(),
            until: "2026-08-09".into(),
            branch: "ui-overhaul".into(),
            no_sub: true,
            thinking: false,
            ..Default::default()
        };
        assert_eq!(
            flags(&narrow),
            vec![
                "-P dashqard",
                "-r user",
                "-s 2026-08-01",
                "-u 2026-08-09",
                "-b ui-overhaul",
                "-n",
                "-T"
            ]
        );
    }

    #[test]
    fn a_probe_lifts_exactly_one_filter() {
        let narrow = Opts {
            pattern: "needle".into(),
            project: "dashqard".into(),
            role: "user".into(),
            ..Default::default()
        };
        let probes = probe_set(&narrow);
        let (_, without_project) = &probes[0];
        assert!(without_project.project.is_empty(), "the named filter is gone");
        assert_eq!(without_project.role, "user", "the others are untouched");
        assert_eq!(without_project.pattern, "needle");
    }

    #[test]
    fn the_report_names_each_filter_and_what_it_costs() {
        let widened = vec![(14, "-P callout".to_owned()), (1, "-r user".to_owned())];
        let lines = no_match_report(&widened, &["-t  also searches tools".into()]);
        let text = lines.join("\n");
        assert!(text.contains("14 matches without  -P callout"), "{text}");
        assert!(text.contains(" 1 match  without  -r user"), "{text}");
        assert!(text.contains("-t  also searches tools"), "{text}");
    }

    #[test]
    fn the_counts_line_up_whatever_their_magnitude() {
        let widened = vec![(9, "-n".to_owned()), (1234, "-P x".to_owned())];
        let lines = no_match_report(&widened, &[]);
        let counted: Vec<&String> = lines.iter().filter(|l| l.contains("without")).collect();
        let at = |l: &str| l.find("without").unwrap();
        assert_eq!(at(counted[0]), at(counted[1]), "{counted:?}");
    }

    #[test]
    fn a_report_with_nothing_to_say_says_nothing() {
        // No filter was costing anything and there is nothing left to widen:
        // printing a bare blank line under "no matches" would be worse.
        assert!(no_match_report(&[], &[]).is_empty());
    }

    #[test]
    fn hints_offer_the_widenings_that_need_no_second_search() {
        let plain = Opts { pattern: "needle".into(), ..Default::default() };
        assert_eq!(widening_hints(&plain).len(), 1, "just -t");

        // A pattern full of metacharacters may have been meant literally.
        let regexy = Opts { pattern: "useState(".into(), ..Default::default() };
        let hints = widening_hints(&regexy);
        assert!(hints.iter().any(|h| h.starts_with("-F")), "{hints:?}");

        // Nothing left to suggest once both are already in force.
        let widest = Opts { pattern: "needle".into(), tools: true, ..Default::default() };
        assert!(widening_hints(&widest).is_empty());
    }
}
