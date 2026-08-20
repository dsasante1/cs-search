//! `cs resume <session-id>` — reopen a session in Claude Code.
//!
//! Searching usually ends in wanting to be back inside the conversation, not
//! merely reading it. Claude Code resolves `--resume` relative to the project it
//! is launched from, so the session's own cwd is recovered from the transcript
//! and used as the working directory; resuming from wherever you happened to be
//! standing would silently fail to find the session.

use crate::show;
use std::process::Command;

pub fn run(id: &str) -> i32 {
    if id.is_empty() {
        eprintln!("cs resume <session-id>");
        return 2;
    }
    let matches = show::resolve(id);
    let Some(path) = matches.first() else {
        eprintln!("no session matching '{id}'");
        return 1;
    };
    if matches.len() > 1 {
        eprintln!("# {} sessions match '{id}', resuming the first", matches.len());
    }
    // --resume wants the full id; search output only carries the first 8 chars.
    let Some(full) = path.file_stem().and_then(|s| s.to_str()) else {
        eprintln!("cannot read a session id from {}", path.display());
        return 1;
    };

    let mut cmd = Command::new("claude");
    cmd.arg("--resume").arg(full);
    if let Some(cwd) = show::session_cwd(path) {
        if std::path::Path::new(&cwd).is_dir() {
            cmd.current_dir(&cwd);
        } else {
            eprintln!("# {cwd} no longer exists; resuming from here instead");
        }
    }

    match cmd.status() {
        Ok(s) => s.code().unwrap_or(0),
        Err(e) => {
            eprintln!("cs resume needs `claude` on PATH: {e}");
            127
        }
    }
}
