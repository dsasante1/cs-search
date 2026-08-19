//! `cs -i` — pipe results through fzf, with `cs show` as the preview renderer.
//!
//! The session id rides in a hidden first column (`--with-nth=2..`) so the
//! preview command can pass it straight back to this same binary.

use crate::output::{fixed, pad, Row, CYAN, DIM, MAGENTA, RESET};
use crate::show;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn run(rows: &[Row], pattern: &str) -> i32 {
    if rows.is_empty() {
        eprintln!("no matches");
        return 1;
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "cs".into());

    let mut child = match Command::new("fzf")
        .args([
            "--ansi",
            "--no-sort",
            "--reverse",
            "--height=90%",
            "--delimiter=\t",
            "--with-nth=2..",
            &format!("--header=Enter = open full session · pattern: {pattern}"),
            &format!("--preview={exe} show {{1}} 2>/dev/null | head -500"),
            "--preview-window=right:55%:wrap",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cs -i needs fzf on PATH: {e}");
            return 127;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        for r in rows {
            let _ = writeln!(
                stdin,
                "{}\t{DIM}{}{RESET} {CYAN}{}{RESET} {MAGENTA}{}{RESET}  {}",
                r.sid,
                r.ts,
                fixed(&r.project, 16),
                pad(&r.role, 4),
                r.text
            );
        }
    }

    let Ok(out) = child.wait_with_output() else {
        return 1;
    };
    let selection = String::from_utf8_lossy(&out.stdout);
    let Some(sid) = selection.split('\t').next().map(str::trim) else {
        return 0;
    };
    if sid.is_empty() {
        return 0;
    }
    show::run(sid)
}
