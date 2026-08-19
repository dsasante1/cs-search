//! Argument parsing. Built on `lexopt` (a zero-dependency argument iterator) so
//! the conventional forms — `--project=x`, `-tT`, `-c60` — come for free, while
//! the usage text, error wording and flag grammar stay ours: the CLI surface is
//! byte-identical to the shell version it replaced.

use lexopt::prelude::*;
use std::ffi::OsString;

pub const USAGE: &str = r#"cs — search your Claude Code history across every session and project

USAGE
  cs [opts] <pattern>       search all conversation text (regex, case-insensitive)
  cs -p <pattern>           search only YOUR prompts (fast; uses history.jsonl)
  cs -i [opts] <pattern>    interactive picker (fzf); Enter opens that session
  cs show <session-id>      print one session as a readable transcript
  cs sessions [substr]      list sessions newest-first, with their first prompt

OPTIONS
  -P, --project <substr>    only sessions whose cwd contains substr
  -r, --role <user|assistant>
  -t, --tools               also search tool calls and tool results (noisy)
  -T, --no-thinking         skip thinking blocks
  -s, --since <YYYY-MM-DD>  only messages on/after this date
  -c, --chars <n>           snippet width (default 240)
  -l, --files               list matching session files only
  -n, --no-sub              skip subagent (sidechain) messages
  -j, --jobs <n>            worker threads (default: CPU count)
  -h, --help

EXAMPLES
  cs 'stripe webhook'
  cs -P dashqard -r user 'rate limit'
  cs -s 2026-07-01 -t 'ALTER TABLE'
  cs -i refresh.token
"#;

pub struct Opts {
    pub project: String,
    pub role: String,
    pub tools: bool,
    pub thinking: bool,
    pub since: String,
    pub chars: usize,
    pub files_only: bool,
    pub no_sub: bool,
    pub prompts: bool,
    pub interactive: bool,
    pub jobs: usize,
    pub pattern: String,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            project: String::new(),
            role: String::new(),
            tools: false,
            thinking: true,
            since: String::new(),
            chars: 240,
            files_only: false,
            no_sub: false,
            prompts: false,
            interactive: false,
            jobs: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            pattern: String::new(),
        }
    }
}

pub enum Parsed {
    Help,
    Search(Box<Opts>),
}

/// Take the value belonging to `flag`, reporting it the way the shell version did.
fn value(p: &mut lexopt::Parser, flag: &str) -> Result<String, String> {
    p.value()
        .map_err(|_| format!("{flag} requires a value"))?
        .into_string()
        .map_err(|v| format!("{flag} value is not valid UTF-8: {}", v.to_string_lossy()))
}

pub fn parse(args: &[OsString]) -> Result<Parsed, String> {
    let mut o = Opts::default();
    let mut p = lexopt::Parser::from_args(args.iter());

    // Consume flags up to the first bare word, matching the original loop:
    // the pattern ends option parsing, so anything after it is left alone.
    loop {
        let arg = match p.next() {
            Ok(Some(a)) => a,
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        };
        // The flag as the user spelled it, so errors quote back what they typed.
        let name = match &arg {
            Short(c) => format!("-{c}"),
            Long(s) => format!("--{s}"),
            Value(_) => String::new(),
        };
        match arg {
            Short('P') | Long("project") => o.project = value(&mut p, &name)?.to_lowercase(),
            Short('r') | Long("role") => o.role = value(&mut p, &name)?,
            Short('s') | Long("since") => o.since = value(&mut p, &name)?,
            Short('c') | Long("chars") => {
                let v = value(&mut p, &name)?;
                o.chars = v.parse().map_err(|_| format!("bad --chars value: {v}"))?;
            }
            Short('j') | Long("jobs") => {
                let v = value(&mut p, &name)?;
                o.jobs = v
                    .parse::<usize>()
                    .map_err(|_| format!("bad --jobs value: {v}"))?
                    .max(1);
            }
            Short('t') | Long("tools") => o.tools = true,
            Short('T') | Long("no-thinking") => o.thinking = false,
            Short('l') | Long("files") => o.files_only = true,
            Short('n') | Long("no-sub") => o.no_sub = true,
            Short('p') | Long("prompts") => o.prompts = true,
            Short('i') | Long("interactive") => o.interactive = true,
            Short('h') | Long("help") => return Ok(Parsed::Help),
            Value(v) => {
                o.pattern = v.into_string().map_err(|v| {
                    format!("pattern is not valid UTF-8: {}", v.to_string_lossy())
                })?;
                break;
            }
            _ => return Err(format!("unknown option: {name}")),
        }
    }

    if o.pattern.is_empty() {
        return Err(String::new()); // empty message => print usage
    }

    if !o.role.is_empty() && o.role != "user" && o.role != "assistant" {
        return Err(format!("--role must be 'user' or 'assistant', got: {}", o.role));
    }

    Ok(Parsed::Search(Box::new(o)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// Parse and unwrap to Opts, failing the test on Help or Err.
    fn opts(args: &[&str]) -> Opts {
        match parse(&owned(args)) {
            Ok(Parsed::Search(o)) => *o,
            Ok(Parsed::Help) => panic!("expected a search, got help: {args:?}"),
            Err(e) => panic!("expected a search, got error {e:?}: {args:?}"),
        }
    }

    fn err(args: &[&str]) -> String {
        match parse(&owned(args)) {
            Err(e) => e,
            _ => panic!("expected an error: {args:?}"),
        }
    }

    #[test]
    fn bare_pattern_uses_defaults() {
        let o = opts(&["needle"]);
        assert_eq!(o.pattern, "needle");
        assert_eq!(o.chars, 240);
        assert!(o.thinking, "thinking blocks are searched unless -T");
        assert!(!o.tools, "tool blocks are skipped unless -t");
        assert!(!o.files_only && !o.no_sub && !o.prompts && !o.interactive);
        assert!(o.role.is_empty() && o.project.is_empty() && o.since.is_empty());
        assert!(o.jobs >= 1);
    }

    #[test]
    fn boolean_flags_set_their_fields() {
        let o = opts(&["-t", "-T", "-l", "-n", "-p", "-i", "needle"]);
        assert!(o.tools && !o.thinking && o.files_only && o.no_sub && o.prompts && o.interactive);
    }

    #[test]
    fn long_flag_spellings_are_accepted() {
        let o = opts(&["--tools", "--no-thinking", "--files", "--no-sub", "needle"]);
        assert!(o.tools && !o.thinking && o.files_only && o.no_sub);
    }

    #[test]
    fn short_boolean_flags_can_be_bundled() {
        let o = opts(&["-tTln", "needle"]);
        assert!(o.tools && !o.thinking && o.files_only && o.no_sub);
        assert_eq!(o.pattern, "needle");
    }

    #[test]
    fn project_filter_is_lowercased_for_case_insensitive_matching() {
        assert_eq!(opts(&["-P", "DashQard", "needle"]).project, "dashqard");
    }

    #[test]
    fn value_flags_are_parsed() {
        let o = opts(&["-r", "user", "-s", "2026-07-01", "-c", "60", "-j", "3", "needle"]);
        assert_eq!(o.role, "user");
        assert_eq!(o.since, "2026-07-01");
        assert_eq!(o.chars, 60);
        assert_eq!(o.jobs, 3);
        assert_eq!(o.pattern, "needle");
    }

    #[test]
    fn long_flags_take_an_attached_value() {
        let o = opts(&["--project=DashQard", "--chars=60", "--role=user", "needle"]);
        assert_eq!(o.project, "dashqard");
        assert_eq!(o.chars, 60);
        assert_eq!(o.role, "user");
        assert_eq!(o.pattern, "needle");
    }

    #[test]
    fn short_flags_take_an_attached_value() {
        let o = opts(&["-c60", "-j3", "-Pdashqard", "needle"]);
        assert_eq!(o.chars, 60);
        assert_eq!(o.jobs, 3);
        assert_eq!(o.project, "dashqard");
    }

    #[test]
    fn a_bundle_can_end_in_a_value_flag() {
        let o = opts(&["-tc", "60", "needle"]);
        assert!(o.tools);
        assert_eq!(o.chars, 60);
        assert_eq!(o.pattern, "needle");
    }

    #[test]
    fn jobs_is_clamped_to_at_least_one() {
        assert_eq!(opts(&["-j", "0", "needle"]).jobs, 1);
    }

    #[test]
    fn role_must_be_a_known_speaker() {
        assert_eq!(opts(&["-r", "assistant", "needle"]).role, "assistant");
        assert!(err(&["-r", "robot", "needle"]).contains("--role"));
    }

    #[test]
    fn double_dash_allows_a_pattern_that_looks_like_a_flag() {
        assert_eq!(opts(&["--", "-t"]).pattern, "-t");
    }

    #[test]
    fn a_lone_dash_is_a_pattern_not_a_flag() {
        assert_eq!(opts(&["-"]).pattern, "-");
    }

    #[test]
    fn flags_after_the_pattern_are_left_alone() {
        // The pattern ends option parsing, as in the shell version.
        let o = opts(&["needle", "-t"]);
        assert_eq!(o.pattern, "needle");
        assert!(!o.tools);
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(err(&["-Z", "needle"]).contains("unknown option"));
        assert!(err(&["--nope", "needle"]).contains("unknown option"));
        // A bundle names the offending letter, not the whole bundle.
        assert_eq!(err(&["-tZ", "needle"]), "unknown option: -Z");
    }

    #[test]
    fn bad_numeric_values_are_rejected() {
        assert!(err(&["-c", "many", "needle"]).contains("--chars"));
        assert!(err(&["-j", "lots", "needle"]).contains("--jobs"));
    }

    #[test]
    fn a_flag_missing_its_value_is_rejected() {
        assert!(err(&["-P"]).contains("requires a value"));
        assert!(err(&["-r"]).contains("requires a value"));
        // The message quotes the spelling the user actually typed.
        assert_eq!(err(&["--project"]), "--project requires a value");
    }

    #[test]
    fn a_value_on_a_boolean_flag_is_rejected() {
        assert!(!err(&["--tools=yes", "needle"]).is_empty());
    }

    #[test]
    fn no_pattern_requests_usage() {
        // An empty message is the signal to print usage rather than an error.
        assert_eq!(err(&[]), "");
        assert_eq!(err(&["-t"]), "");
        assert_eq!(err(&["--"]), "");
    }

    #[test]
    fn help_short_circuits() {
        assert!(matches!(parse(&owned(&["-h", "needle"])), Ok(Parsed::Help)));
    }

    #[test]
    fn usage_documents_every_flag_the_parser_accepts() {
        for flag in [
            "-P", "-r", "-t", "-T", "-s", "-c", "-l", "-n", "-j", "-h", "-p", "-i",
        ] {
            assert!(USAGE.contains(flag), "usage text is missing {flag}");
        }
    }
}
