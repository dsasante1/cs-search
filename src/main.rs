//! cs — search Claude Code conversation history across every session and project.

mod cli;
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

use cli::{Opts, Parsed, USAGE};
use output::Row;
use regex::Regex;
use std::ffi::OsString;
use std::io::{BufWriter, Write};
use std::path::Path;
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
        eprintln!("no matches");
        return 1;
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

/// `cs show <id> [--highlight <pat>] [--at <pat>] [--color] [--no-pager]`.
///
/// Hand-parsed rather than run through `cli::parse`, because `show` has never
/// shared the search flag grammar and the extra flags here are mostly for the
/// picker to pass to itself.
fn show_command(args: &[OsString]) -> i32 {
    let mut id = String::new();
    let mut o = show::ShowOpts { pager: true, ..Default::default() };
    let mut rest = args.iter().skip(1);

    while let Some(arg) = rest.next() {
        let mut take = || rest.next().and_then(|v| v.to_str()).and_then(show::pattern);
        match arg.to_str().unwrap_or("") {
            "--highlight" => o.highlight = take(),
            "--at" => o.at = take(),
            "--color" => o.color = true,
            "--no-pager" => o.pager = false,
            value if id.is_empty() => id = value.to_owned(),
            _ => {}
        }
    }
    show::run_with(&id, &o)
}
