//! `cs completions <bash|zsh|fish>` — the shell script to source.
//!
//! Session ids are eight hex characters. Nobody types one from memory, so every
//! `cs show` is really `cs sessions | grep` followed by a copy and a paste. The
//! ids are already listable, which makes this a completion problem rather than
//! an interface one: the same goes for project names, which `-P` expects you to
//! know in advance.
//!
//! The dynamic parts shell out to `cs` itself rather than caching anything —
//! the corpus changes every time you use Claude Code, and `sessions` answers in
//! well under a tenth of a second.

use std::io::Write;

/// Subcommands offered first, in the order the usage text introduces them.
pub const SUBCOMMANDS: &[&str] = &[
    "show",
    "sessions",
    "files",
    "history",
    "activity",
    "handoff",
    "related",
    "export",
    "projects",
    "stats",
    "resume",
    "completions",
];

/// Every flag a search accepts. Kept beside the parser it mirrors: a flag added
/// there and forgotten here simply fails to complete, which is why the test
/// below reads them out of the usage text instead of trusting this list.
pub const FLAGS: &[&str] = &[
    "-F",
    "--fixed",
    "-P",
    "--project",
    "-r",
    "--role",
    "-s",
    "--since",
    "-u",
    "--until",
    "-b",
    "--branch",
    "-t",
    "--tools",
    "-T",
    "--no-thinking",
    "-n",
    "--no-sub",
    "-C",
    "--context",
    "-A",
    "--after",
    "-B",
    "--before",
    "-c",
    "--chars",
    "-l",
    "--files",
    "-p",
    "--prompts",
    "-q",
    "--questions",
    "-i",
    "--interactive",
    "-j",
    "--jobs",
    "--thread",
    "--plain",
    "--group",
    "--no-group",
    "--chrono",
    "--json",
    "--preview",
    "-h",
    "--help",
];

pub fn write(w: &mut impl Write, shell: &str) -> Result<(), String> {
    let script = match shell {
        "bash" => bash(),
        "zsh" => zsh(),
        "fish" => fish(),
        other => return Err(format!("unknown shell '{other}' — want bash, zsh or fish")),
    };
    let _ = write!(w, "{script}");
    Ok(())
}

fn bash() -> String {
    format!(
        r#"# cs completions for bash. Add to ~/.bashrc:
#   eval "$(cs completions bash)"
_cs() {{
  local cur prev sub
  cur="${{COMP_WORDS[COMP_CWORD]}}"
  prev="${{COMP_WORDS[COMP_CWORD-1]}}"
  sub="${{COMP_WORDS[1]}}"

  case "$prev" in
    -P|--project) COMPREPLY=($(compgen -W "$(_cs_projects)" -- "$cur")); return;;
    -r|--role)    COMPREPLY=($(compgen -W "user assistant" -- "$cur")); return;;
    -s|--since|-u|--until)
                  COMPREPLY=($(compgen -W "{dates}" -- "$cur")); return;;
    --preview)    COMPREPLY=($(compgen -W "right bottom" -- "$cur")); return;;
    --format|-f)  COMPREPLY=($(compgen -W "md html json" -- "$cur")); return;;
    --prices)     COMPREPLY=($(compgen -f -- "$cur")); return;;
    completions)  COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur")); return;;
  esac

  # An id is only wanted where one is taken, and listing them is a scan.
  case "$sub" in
    show|resume|export|handoff|related|stats)
      if [ "$COMP_CWORD" -eq 2 ]; then
        COMPREPLY=($(compgen -W "$(_cs_sessions)" -- "$cur")); return
      fi;;
  esac

  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "{subs}" -- "$cur"))
    return
  fi
  case "$cur" in
    -*) COMPREPLY=($(compgen -W "{flags}" -- "$cur"));;
  esac
}}
_cs_sessions() {{ cs sessions 2>/dev/null | awk '{{print $5}}'; }}
_cs_projects() {{ cs projects 2>/dev/null | awk '{{n=split($4,a,"/"); print a[n]}}'; }}
complete -F _cs cs
"#,
        subs = SUBCOMMANDS.join(" "),
        flags = FLAGS.join(" "),
        dates = DATE_WORDS,
    )
}

fn zsh() -> String {
    format!(
        r#"#compdef cs
# cs completions for zsh. Add to ~/.zshrc:
#   eval "$(cs completions zsh)"
_cs_sessions() {{
  local -a ids
  ids=(${{(f)"$(cs sessions 2>/dev/null | awk '{{print $5" "substr($0, index($0,$6))}}')"}})
  _describe -t sessions 'session' ids
}}
_cs_projects() {{
  local -a names
  names=(${{(f)"$(cs projects 2>/dev/null | awk '{{n=split($4,a,"/"); print a[n]}}')"}})
  _describe -t projects 'project' names
}}
_cs() {{
  local context state line
  _arguments -C \
    '1: :->first' \
    '*:: :->rest'

  case $state in
    first) _describe -t commands 'cs command' '({subs})' ;;
    rest)
      case $words[1] in
        show|resume|export|handoff|related|stats) _cs_sessions ;;
        completions) _values 'shell' bash zsh fish ;;
      esac
      _arguments \
        '(-P --project)'{{-P,--project}}'[project substring]:project:_cs_projects' \
        '(-r --role)'{{-r,--role}}'[speaker]:role:(user assistant)' \
        '(-s --since)'{{-s,--since}}'[from date]:date:({dates})' \
        '(-u --until)'{{-u,--until}}'[to date]:date:({dates})' \
        '(-b --branch)'{{-b,--branch}}'[git branch substring]:branch:' \
        '--format[export format]:format:(md html json)' \
        '--preview[preview pane edge]:edge:(right bottom)' \
        '--prices[price table]:file:_files' \
        '*:flag:({flags})'
      ;;
  esac
}}
_cs "$@"
"#,
        subs = SUBCOMMANDS.join(" "),
        flags = FLAGS.join(" "),
        dates = DATE_WORDS,
    )
}

fn fish() -> String {
    let mut out = String::from(
        "# cs completions for fish. Add to ~/.config/fish/config.fish:\n\
         #   cs completions fish | source\n\
         function __cs_sessions\n    cs sessions 2>/dev/null | awk '{print $5}'\nend\n\
         function __cs_projects\n    cs projects 2>/dev/null | awk '{n=split($4,a,\"/\"); print a[n]}'\nend\n\
         function __cs_no_sub\n    not __fish_seen_subcommand_from ",
    );
    out.push_str(&SUBCOMMANDS.join(" "));
    out.push_str("\nend\n\n");

    for (name, help) in SUBCOMMAND_HELP {
        out.push_str(&format!("complete -c cs -n __cs_no_sub -a {name} -d '{help}'\n"));
    }
    out.push_str(
        "\ncomplete -c cs -n '__fish_seen_subcommand_from show resume export handoff related stats' -f -a '(__cs_sessions)'\n\
         complete -c cs -n '__fish_seen_subcommand_from completions' -f -a 'bash zsh fish'\n\n",
    );
    out.push_str(
        "complete -c cs -s P -l project -x -a '(__cs_projects)' -d 'project substring'\n\
         complete -c cs -s r -l role -x -a 'user assistant' -d 'speaker'\n\
         complete -c cs -s b -l branch -x -d 'git branch substring'\n",
    );
    for (flag, help) in [("s", "since"), ("u", "until")] {
        out.push_str(&format!(
            "complete -c cs -s {flag} -l {help} -x -a '{DATE_WORDS}' -d 'date or 7d/last-week'\n"
        ));
    }
    out.push_str(
        "complete -c cs -l format -x -a 'md html json' -d 'export format'\n\
         complete -c cs -l preview -x -a 'right bottom' -d 'preview pane edge'\n\
         complete -c cs -l prices -r -d 'price table'\n",
    );
    for f in FLAGS.iter().filter(|f| f.starts_with("--")) {
        out.push_str(&format!("complete -c cs -l {} \n", &f[2..]));
    }
    out
}

/// The relative date words worth offering; the absolute form cannot be
/// completed and does not need to be.
const DATE_WORDS: &str = "today yesterday last-week last-month 7d 14d 2w 3m 1y";

const SUBCOMMAND_HELP: &[(&str, &str)] = &[
    ("show", "print one session as a transcript"),
    ("sessions", "list sessions, newest first"),
    ("files", "which files were edited or read"),
    ("history", "when a topic started and stopped"),
    ("activity", "sessions and messages per day"),
    ("handoff", "where a session left off"),
    ("related", "sessions sharing this one's words"),
    ("export", "write a session out as md, html or json"),
    ("projects", "list projects"),
    ("stats", "models, tokens and cache use"),
    ("resume", "reopen a session in Claude Code"),
    ("completions", "print a shell completion script"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn script(shell: &str) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write(&mut buf, shell).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn an_unknown_shell_is_named_in_the_error() {
        let mut buf: Vec<u8> = Vec::new();
        let e = write(&mut buf, "tcsh").unwrap_err();
        assert!(e.contains("tcsh") && e.contains("bash"), "{e}");
        assert!(buf.is_empty(), "nothing should be written for a shell we cannot serve");
    }

    #[test]
    fn every_shell_offers_every_subcommand() {
        for shell in ["bash", "zsh", "fish"] {
            let s = script(shell);
            for sub in SUBCOMMANDS {
                assert!(s.contains(sub), "{shell} is missing {sub}");
            }
        }
    }

    /// The point of the feature: the commands that take an id complete one.
    #[test]
    fn the_id_taking_commands_complete_session_ids() {
        for shell in ["bash", "zsh", "fish"] {
            let s = script(shell);
            assert!(s.contains("cs sessions"), "{shell} should list ids to complete them");
            for sub in ["show", "resume", "export"] {
                assert!(s.contains(sub), "{shell} should complete ids after {sub}");
            }
        }
    }

    #[test]
    fn project_and_role_values_are_completed_too() {
        for shell in ["bash", "zsh", "fish"] {
            let s = script(shell);
            assert!(s.contains("cs projects"), "{shell}");
            assert!(s.contains("user assistant") || s.contains("user' 'assistant"), "{shell}");
        }
    }

    #[test]
    fn the_relative_date_words_are_offered_where_a_date_is_taken() {
        for shell in ["bash", "zsh", "fish"] {
            let s = script(shell);
            assert!(s.contains("last-week"), "{shell} should suggest relative dates");
            assert!(s.contains("yesterday"), "{shell}");
        }
    }

    /// Each script says how to install itself: printed to a terminal with no
    /// explanation, a completion script looks like a mistake.
    #[test]
    fn each_script_carries_its_own_install_line() {
        assert!(script("bash").contains("eval \"$(cs completions bash)\""));
        assert!(script("zsh").contains("eval \"$(cs completions zsh)\""));
        assert!(script("fish").contains("cs completions fish | source"));
    }

    /// The flag list is duplicated from the parser, so it is checked against
    /// the usage text rather than trusted: a flag documented there and missing
    /// here would silently stop completing.
    #[test]
    fn the_flag_list_covers_what_the_usage_documents() {
        let usage = crate::cli::USAGE;
        let documented: Vec<&str> = usage
            .split_whitespace()
            .map(|w| w.trim_end_matches(&[',', '<', '>'][..]))
            .filter(|w| w.starts_with("--") && w.len() > 2)
            .filter(|w| w.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
            .collect();
        for flag in documented {
            // Subcommand-only flags live on their own commands, not on a search.
            if [
                "--format",
                "--prices",
                "--highlight",
                "--at",
                "--color",
                "--no-pager",
                "--sessions",
                "--limit",
            ]
            .contains(&flag)
            {
                continue;
            }
            assert!(FLAGS.contains(&flag), "{flag} is documented but never completed");
        }
    }
}
