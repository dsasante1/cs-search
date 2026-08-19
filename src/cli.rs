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
