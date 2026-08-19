//! Argument parsing. Hand-rolled to keep the CLI surface byte-identical to the
//! shell version (and to avoid pulling in a parser crate for nine flags).

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

pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Opts::default();
    let mut i = 0;

    // Consume flags up to the first bare word, matching the original loop.
    while i < args.len() {
        let a = args[i].as_str();
        let need = |o: &mut String, i: &mut usize| -> Result<(), String> {
            let v = args
                .get(*i + 1)
                .ok_or_else(|| format!("{a} requires a value"))?;
            *o = v.clone();
            *i += 2;
            Ok(())
        };
        match a {
            "-P" | "--project" => {
                let mut v = String::new();
                need(&mut v, &mut i)?;
                o.project = v.to_lowercase();
            }
            "-r" | "--role" => need(&mut o.role, &mut i)?,
            "-s" | "--since" => need(&mut o.since, &mut i)?,
            "-c" | "--chars" => {
                let mut v = String::new();
                need(&mut v, &mut i)?;
                o.chars = v.parse().map_err(|_| format!("bad --chars value: {v}"))?;
            }
            "-j" | "--jobs" => {
                let mut v = String::new();
                need(&mut v, &mut i)?;
                o.jobs = v
                    .parse::<usize>()
                    .map_err(|_| format!("bad --jobs value: {v}"))?
                    .max(1);
            }
            "-t" | "--tools" => {
                o.tools = true;
                i += 1;
            }
            "-T" | "--no-thinking" => {
                o.thinking = false;
                i += 1;
            }
            "-l" | "--files" => {
                o.files_only = true;
                i += 1;
            }
            "-n" | "--no-sub" => {
                o.no_sub = true;
                i += 1;
            }
            "-p" | "--prompts" => {
                o.prompts = true;
                i += 1;
            }
            "-i" | "--interactive" => {
                o.interactive = true;
                i += 1;
            }
            "-h" | "--help" => return Ok(Parsed::Help),
            "--" => {
                i += 1;
                break;
            }
            _ if a.starts_with('-') && a.len() > 1 => {
                return Err(format!("unknown option: {a}"))
            }
            _ => break,
        }
    }

    match args.get(i) {
        Some(p) if !p.is_empty() => o.pattern = p.clone(),
        _ => return Err(String::new()), // empty message => print usage
    }

    if !o.role.is_empty() && o.role != "user" && o.role != "assistant" {
        return Err(format!("--role must be 'user' or 'assistant', got: {}", o.role));
    }

    Ok(Parsed::Search(Box::new(o)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse and unwrap to Opts, failing the test on Help or Err.
    fn opts(args: &[&str]) -> Opts {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match parse(&owned) {
            Ok(Parsed::Search(o)) => *o,
            Ok(Parsed::Help) => panic!("expected a search, got help: {args:?}"),
            Err(e) => panic!("expected a search, got error {e:?}: {args:?}"),
        }
    }

    fn err(args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match parse(&owned) {
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
    fn unknown_options_are_rejected() {
        assert!(err(&["-Z", "needle"]).contains("unknown option"));
        assert!(err(&["--nope", "needle"]).contains("unknown option"));
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
    }

    #[test]
    fn no_pattern_requests_usage() {
        // An empty message is the signal to print usage rather than an error.
        assert_eq!(err(&[]), "");
        assert_eq!(err(&["-t"]), "");
    }

    #[test]
    fn help_short_circuits() {
        let owned = vec!["-h".to_string(), "needle".to_string()];
        assert!(matches!(parse(&owned), Ok(Parsed::Help)));
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
