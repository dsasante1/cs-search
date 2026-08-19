//! cs — search Claude Code conversation history across every session and project.

mod cli;
mod interactive;
mod output;
mod prompts;
mod record;
mod scan;
mod sessions;
mod show;

use cli::{Parsed, USAGE};
use output::{is_tty, Row};
use regex::Regex;
use std::io::{BufWriter, Write};
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print!("{USAGE}");
        exit(0);
    }

    // Subcommands are matched before flags, as in the original script.
    match args[0].as_str() {
        "show" => exit(show::run(args.get(1).map(String::as_str).unwrap_or(""))),
        "sessions" => {
            let filter = args.get(1).map(String::as_str).unwrap_or("");
            let jobs = cli::Opts::default().jobs;
            let rows = sessions::run(filter, jobs);
            print_rows(&rows, None);
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

    let re = match Regex::new(&format!("(?i){}", opts.pattern)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bad pattern: {e}");
            exit(2);
        }
    };

    if opts.prompts {
        match prompts::run(&opts, &re) {
            Ok(rows) => {
                print_rows(&rows, Some(&re));
                exit(0);
            }
            Err(msg) => {
                eprintln!("{msg}");
                exit(1);
            }
        }
    }

    let hits = scan::search(&opts, &re);

    if hits.rows.is_empty() {
        eprintln!("no matches");
        exit(1);
    }

    if opts.files_only {
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

    if opts.interactive {
        exit(interactive::run(&hits.rows, &opts.pattern));
    }

    print_rows(&hits.rows, Some(&re));
}

fn print_rows(rows: &[Row], highlight: Option<&Regex>) {
    let color = is_tty();
    let hl = if color { highlight } else { None };
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    for r in rows {
        let _ = writeln!(w, "{}", r.render(color, hl));
    }
    let _ = w.flush();
}
