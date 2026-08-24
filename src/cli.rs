//! Argument parsing. Built on `lexopt` (a zero-dependency argument iterator) so
//! the conventional forms — `--project=x`, `-tT`, `-c60` — come for free, while
//! the usage text, error wording and flag grammar stay ours.
//!
//! The original shell CLI is still accepted verbatim; everything added since is
//! either a new flag or a new subcommand, so old invocations behave as they did.

use lexopt::prelude::*;
use std::ffi::OsString;

pub const USAGE: &str = r#"cs — search your Claude Code history across every session and project

USAGE
  cs [opts] <pattern>       search all conversation text (regex, ignoring case)
  cs -p <pattern>           search only YOUR prompts (fast; uses history.jsonl)
  cs -i [opts] <pattern>    interactive picker (fzf); typing re-searches live
  cs show <session-id>      print one session as a readable transcript
                            -r user|assistant reads one side of it only
  cs sessions [substr]      list sessions newest-first, by title
  cs files <pattern>        which files were edited or read, and when
  cs history <pattern>      when a topic started, when it stopped, and where
                            --sessions lists the sessions, oldest first
  cs activity               sessions and messages per day, and per project
  cs handoff <session-id>   where a session left off: files, tokens, last turns
  cs related <session-id>   sessions sharing this one's distinctive words
                            --limit <n> to see more than ten
  cs export <session-id>    write one session out: --format md|html|json
  cs projects [substr]      list projects with session counts
  cs stats [session-id]     models, tokens and cache use; one session or all
                            -P <proj> scopes it, --prices <file> adds cost
  cs resume <session-id>    reopen that session in Claude Code
  cs completions <shell>    completion script for bash, zsh or fish

  On a terminal, a plain search opens the picker. Piped, it prints rows.

SEARCH
  -F, --fixed               match the pattern literally, not as a regex
  -P, --project <substr>    only sessions whose cwd contains substr
  -r, --role <user|assistant>
  -q, --questions           only lines that ask: a ? ending a clause
  -s, --since <date>        only messages on/after this date
  -u, --until <date>        only messages on/before this date
                            dates: YYYY-MM-DD, today, yesterday, 7d, last-week
  -b, --branch <substr>     only sessions on a git branch containing substr
  -t, --tools               also search tool calls and tool results (noisy)
  -T, --no-thinking         skip thinking blocks
  -n, --no-sub              skip subagent (sidechain) messages

OUTPUT
      --thread              show the turns either side of the match, not lines
  -C, --context <n>         show n lines either side of each match
  -A, --after <n>           show n lines after each match
  -B, --before <n>          show n lines before each match
  -c, --chars <n>           snippet width (default 240)
  -l, --files               list matching session files only
      --plain               print results instead of opening the picker
      --group               group matches by session (the default on a terminal)
      --no-group            one line per match, ungrouped
      --chrono              one line per session, oldest first
      --json                one JSON object per match, one per line
      --preview <right|bottom>  where the picker draws the transcript
  -j, --jobs <n>            worker threads (default: CPU count)
  -h, --help

PICKER KEYS
  enter open · alt-enter resume · alt-t tools · alt-h thinking
  alt-s subagents · alt-r role · alt-x thread · alt-p this project
  alt-c clear filters

EXAMPLES
  Searching
    cs 'stripe webhook'                  # the picker; rows when piped
    cs -F 'useState('                    # literally, not as a regex
    cs -P dashqard -r user 'rate limit'  # your own words, in one project
    cs -p -q 'cloud sql'                 # what you asked, not what you told
    cs -t 'ALTER TABLE'                  # tool calls and their output too
    cs -C 2 timeout --plain              # two lines either side, no picker

  Narrowing
    cs -s 7d 'flaky'                     # the last week
    cs -s 2026-08-01 -u 2026-08-01 rls   # a single day, both ends
    cs -b ui-overhaul 'divider'          # one git branch
    cs --thread 'the fix'                # with the turns either side
    cs 'rate limit' --chrono             # one line per session, oldest first

  One session at a time
    cs sessions dashqard                 # by title, newest first
    cs show 3f2a1b9c                     # as a transcript
    cs handoff 3f2a1b9c                  # where you left off
    cs related 3f2a1b9c                  # what else was about this
    cs stats 3f2a1b9c --prices p.json    # what this one cost
    cs export 3f2a1b9c --format md       # as a document, on stdout
    cs resume 3f2a1b9c                   # reopen it in Claude Code

  Across the corpus
    cs projects                          # what -P can be given
    cs files 'settings/base.py'          # which sessions touched a file
    cs history 'django-celery'           # when it started, when it stopped
    cs activity -s 30d                   # where the month went
    cs stats -P dashqard -s last-month   # models, tokens, cache

  In a script
    cs database --json | jq -r .session  # one object per match
    cs -l migration                      # only the files that matched
    eval "$(cs completions bash)"        # complete ids and projects
"#;

#[derive(Clone)]
pub struct Opts {
    pub project: String,
    pub role: String,
    pub tools: bool,
    pub thinking: bool,
    pub since: String,
    /// `--until`: the far end of the range `--since` opens.
    pub until: String,
    /// `-b`: substring of the git branch the session was on.
    pub branch: String,
    pub chars: usize,
    pub files_only: bool,
    pub no_sub: bool,
    pub prompts: bool,
    pub interactive: bool,
    pub jobs: usize,
    pub pattern: String,
    /// `-F`: the pattern is a literal, escaped before it reaches the engine.
    pub fixed: bool,
    /// `--plain`: never hand off to the picker, even on a terminal.
    pub plain: bool,
    pub json: bool,
    pub grouping: Grouping,
    /// Which edge of the picker the preview pane sits on.
    pub preview: String,
    pub before: usize,
    pub after: usize,
    /// `--thread`: surround a match with the turns either side of it rather
    /// than with more lines of the message it sits in.
    pub thread: bool,
    /// `-q`: keep only lines that ask something. See `record::is_question` for
    /// what counts, which is deliberately less than "everything interrogative".
    pub questions: bool,
    /// One session, by id prefix. Not a search flag — `stats` and `handoff`
    /// take an id positionally and narrow the walk to the file it names.
    pub session: String,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            project: String::new(),
            role: String::new(),
            tools: false,
            thinking: true,
            since: String::new(),
            until: String::new(),
            branch: String::new(),
            chars: 240,
            files_only: false,
            no_sub: false,
            prompts: false,
            interactive: false,
            jobs: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            pattern: String::new(),
            fixed: false,
            plain: false,
            json: false,
            grouping: Grouping::Auto,
            preview: "right".into(),
            before: 0,
            after: 0,
            thread: false,
            questions: false,
            session: String::new(),
        }
    }
}

/// Whether matches are folded under one heading per session. `Auto` means "when
/// a human is reading", which is the only case where it helps: a pipe wants the
/// original one-line-per-match format.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grouping {
    Auto,
    Always,
    Never,
    /// `--chrono`: one line per session, oldest first. A third rendering rather
    /// than a degree of the first two, so it answers `applies` with a flat no.
    Chrono,
}

impl Grouping {
    pub fn applies(self, tty: bool) -> bool {
        match self {
            Grouping::Auto => tty,
            Grouping::Always => true,
            Grouping::Never | Grouping::Chrono => false,
        }
    }

    pub fn chrono(self) -> bool {
        matches!(self, Grouping::Chrono)
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

fn count(p: &mut lexopt::Parser, flag: &str) -> Result<usize, String> {
    let v = value(p, flag)?;
    v.parse()
        .map_err(|_| format!("bad {flag} value: {v}"))
}

pub fn parse(args: &[OsString]) -> Result<Parsed, String> {
    let mut o = Opts::default();
    let mut p = lexopt::Parser::from_args(args.iter());

    // Flags are read wherever they appear. The shell version stopped at the
    // first bare word, which meant `cs database --json` searched for `database`
    // and dropped the flag without a word — including in four of this program's
    // own documented examples. The pattern is still the first bare word; a
    // second one is an error rather than something else silently ignored.
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
            Short('s') | Long("since") => {
                o.since = crate::dates::resolve(&value(&mut p, &name)?)
                    .map_err(|e| format!("{name}: {e}"))?
            }
            Short('u') | Long("until") => {
                o.until = crate::dates::resolve(&value(&mut p, &name)?)
                    .map_err(|e| format!("{name}: {e}"))?
            }
            Short('c') | Long("chars") => o.chars = count(&mut p, "--chars")?,
            Short('j') | Long("jobs") => o.jobs = count(&mut p, "--jobs")?.max(1),
            Short('C') | Long("context") => {
                let n = count(&mut p, "--context")?;
                o.before = n;
                o.after = n;
            }
            Short('A') | Long("after") => o.after = count(&mut p, "--after")?,
            Short('B') | Long("before") => o.before = count(&mut p, "--before")?,
            Short('b') | Long("branch") => o.branch = value(&mut p, &name)?.to_lowercase(),
            Short('t') | Long("tools") => o.tools = true,
            Short('T') | Long("no-thinking") => o.thinking = false,
            Short('l') | Long("files") => o.files_only = true,
            Short('n') | Long("no-sub") => o.no_sub = true,
            Short('p') | Long("prompts") => o.prompts = true,
            Short('q') | Long("questions") => o.questions = true,
            Short('i') | Long("interactive") => o.interactive = true,
            Short('F') | Long("fixed") => o.fixed = true,
            Long("plain") => o.plain = true,
            Long("group") => o.grouping = Grouping::Always,
            Long("no-group") => o.grouping = Grouping::Never,
            Long("chrono") => o.grouping = Grouping::Chrono,
            Long("json") => o.json = true,
            Long("thread") => o.thread = true,
            Long("preview") => o.preview = value(&mut p, "--preview")?,
            Short('h') | Long("help") => return Ok(Parsed::Help),
            Value(v) => {
                let text = v.into_string().map_err(|v| {
                    format!("pattern is not valid UTF-8: {}", v.to_string_lossy())
                })?;
                if !o.pattern.is_empty() {
                    return Err(format!(
                        "unexpected argument '{text}' after the pattern '{}' — \
                         quote them if they are one pattern",
                        o.pattern
                    ));
                }
                o.pattern = text;
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

    if o.preview != "right" && o.preview != "bottom" {
        return Err(format!("--preview must be 'right' or 'bottom', got: {}", o.preview));
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
        // New surface, defaulted so the original CLI is unchanged.
        assert!(!o.fixed && !o.plain && !o.json);
        assert_eq!(o.grouping, Grouping::Auto, "grouping follows the terminal");
        assert_eq!((o.before, o.after), (0, 0));
    }

    #[test]
    fn boolean_flags_set_their_fields() {
        let o = opts(&["-t", "-T", "-l", "-n", "-p", "-i", "-F", "needle"]);
        assert!(o.tools && !o.thinking && o.files_only && o.no_sub && o.prompts);
        assert!(o.interactive && o.fixed);
    }

    #[test]
    fn long_flag_spellings_are_accepted() {
        let o = opts(&["--tools", "--no-thinking", "--files", "--no-sub", "needle"]);
        assert!(o.tools && !o.thinking && o.files_only && o.no_sub);
    }

    #[test]
    fn output_mode_flags_are_parsed() {
        let o = opts(&["--plain", "--json", "--no-group", "--fixed", "needle"]);
        assert!(o.plain && o.json && o.fixed);
        assert_eq!(o.grouping, Grouping::Never);
        assert_eq!(opts(&["--group", "needle"]).grouping, Grouping::Always);
    }

    #[test]
    fn preview_placement_is_right_unless_asked_otherwise() {
        assert_eq!(opts(&["needle"]).preview, "right");
        assert_eq!(opts(&["--preview", "bottom", "needle"]).preview, "bottom");
        assert!(err(&["--preview", "sideways", "needle"]).contains("--preview"));
    }

    #[test]
    fn grouping_defaults_to_whoever_is_reading() {
        assert!(Grouping::Auto.applies(true), "a terminal gets groups");
        assert!(!Grouping::Auto.applies(false), "a pipe gets the original format");
        // Both explicit forms override that, in either direction.
        assert!(Grouping::Always.applies(false));
        assert!(!Grouping::Never.applies(true));
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
    fn context_flags_set_both_sides_or_one() {
        let both = opts(&["-C", "3", "needle"]);
        assert_eq!((both.before, both.after), (3, 3));

        let one_sided = opts(&["-A", "2", "-B", "5", "needle"]);
        assert_eq!((one_sided.before, one_sided.after), (5, 2));

        // -C is just a shorthand, so a later one-sided flag still wins.
        let overridden = opts(&["-C", "3", "-A", "1", "needle"]);
        assert_eq!((overridden.before, overridden.after), (3, 1));
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
        let o = opts(&["-c60", "-j3", "-Pdashqard", "-C2", "needle"]);
        assert_eq!(o.chars, 60);
        assert_eq!(o.jobs, 3);
        assert_eq!(o.project, "dashqard");
        assert_eq!(o.before, 2);
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

    /// The shell version stopped reading flags at the pattern, so
    /// `cs database --json` quietly searched without the flag — as four of the
    /// examples shipped with this program did.
    #[test]
    fn flags_are_read_on_either_side_of_the_pattern() {
        let after = opts(&["needle", "-t", "--json"]);
        assert_eq!(after.pattern, "needle");
        assert!(after.tools && after.json);

        let before = opts(&["-t", "--json", "needle"]);
        assert_eq!(before.pattern, "needle");
        assert!(before.tools && before.json);
    }

    /// Two bare words used to mean the first one and a silence. Whatever the
    /// user meant, it was not that.
    #[test]
    fn a_second_bare_word_is_an_error_not_an_argument_dropped() {
        let e = err(&["stripe", "webhook"]);
        assert!(e.contains("unexpected argument \'webhook\'"), "{e}");
        assert!(e.contains("quote them"), "the fix, not just the complaint: {e}");
    }

    /// A flag's own value is not a second pattern.
    #[test]
    fn a_flag_after_the_pattern_still_takes_its_value() {
        let o = opts(&["needle", "-P", "dashqard", "-C", "2"]);
        assert_eq!(o.project, "dashqard");
        assert_eq!(o.before, 2);
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
        assert!(err(&["-C", "some", "needle"]).contains("--context"));
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
            "-F", "-C", "-A", "-B", "-q", "--plain", "--group", "--no-group",
            "--chrono", "--json", "--preview", "--sessions", "--limit",
        ] {
            assert!(USAGE.contains(flag), "usage text is missing {flag}");
        }
    }

    /// The examples exist to be copied, so a flag appearing in one that the
    /// usage never describes is a typo waiting to be pasted into a shell.
    #[test]
    fn every_flag_shown_in_an_example_is_documented_above_it() {
        let examples = USAGE.split("EXAMPLES").nth(1).expect("usage has examples");
        let described = USAGE.split("EXAMPLES").next().unwrap();
        for token in examples.split_whitespace() {
            let flag = token.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
            if !flag.starts_with('-') || flag.len() < 2 {
                continue;
            }
            assert!(
                described.contains(flag),
                "{flag} is used in an example but never documented"
            );
        }
    }

    /// The README's examples are copied at least as often as the help page's,
    /// and drift the same way — `--prices` was documented there and nowhere
    /// else until this test went looking. Only `cs`'s own invocations are
    /// checked: a line's flags stop at the first pipe, so `sort -rn` further
    /// down a pipeline is somebody else's business.
    #[test]
    fn every_flag_shown_in_the_readme_is_documented_too() {
        let readme = include_str!("../README.md");
        let described = USAGE.split("EXAMPLES").next().unwrap();
        for block in readme.split("```").skip(1).step_by(2) {
            for line in block.lines() {
                let line = line.trim();
                let line = line.strip_prefix("$ ").unwrap_or(line);
                let Some(line) = line.strip_prefix("cs ") else { continue };
                let line = line.split('#').next().unwrap();
                let line = line.split('|').next().unwrap();
                for token in line.split_whitespace() {
                    let flag = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                    if !flag.starts_with('-') || flag.len() < 2 {
                        continue;
                    }
                    // -C 2 and friends: a digit is an argument, not a flag.
                    if flag.as_bytes()[1].is_ascii_digit() {
                        continue;
                    }
                    assert!(
                        described.contains(flag),
                        "the README shows {flag} but the help page never describes it"
                    );
                }
            }
        }
    }

    /// Every subcommand earns an example: the synopsis says a command exists,
    /// an example says what it is for.
    #[test]
    fn every_subcommand_appears_in_an_example() {
        let examples = USAGE.split("EXAMPLES").nth(1).expect("usage has examples");
        for sub in [
            "show", "sessions", "files", "export", "projects", "stats", "resume",
            "completions", "history", "activity", "handoff", "related",
        ] {
            assert!(examples.contains(sub), "no example uses {sub}");
        }
    }

    /// A help page that wraps is harder to read than a shorter one, and the
    /// example column is the part that grows sideways as flags are added.
    #[test]
    fn the_usage_fits_a_standard_terminal() {
        for line in USAGE.lines() {
            assert!(
                line.chars().count() <= 80,
                "{} columns: {line}",
                line.chars().count()
            );
        }
    }

    #[test]
    fn usage_documents_every_subcommand() {
        for sub in [
            "show", "sessions", "projects", "resume", "history", "activity",
            "handoff", "related",
        ] {
            assert!(USAGE.contains(&format!("cs {sub}")), "usage is missing {sub}");
        }
    }
}
