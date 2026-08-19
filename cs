#!/usr/bin/env bash
# cs — search Claude Code conversation history across all sessions & projects
set -uo pipefail

ROOT="${CLAUDE_HOME:-$HOME/.claude}"
PROJDIR="$ROOT/projects"
HIST="$ROOT/history.jsonl"

usage() {
cat <<'HELP_EOF'
cs — search your Claude Code history across every session and project

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
  -h, --help

EXAMPLES
  cs 'stripe webhook'
  cs -P dashqard -r user 'rate limit'
  cs -s 2026-07-01 -t 'ALTER TABLE'
  cs -i refresh.token
HELP_EOF
}

JQ_PROG='
def clip($n): if length > $n then .[0:$n] + "…" else . end;
def blocks:
  (.message.content // empty) as $c
  | if ($c|type)=="string" then [$c]
    elif ($c|type)=="array" then
      [ $c[]
        | if   .type=="text"        then .text
          elif .type=="thinking"    then (if $think=="1" then .thinking else empty end)
          elif .type=="tool_use"    then (if $tools=="1" then ((.name//"tool")+" "+(.input|tostring)) else empty end)
          elif .type=="tool_result" then (if $tools=="1" then (.content|tostring) else empty end)
          else empty end ]
    else [] end;
select(.type=="user" or .type=="assistant")
| select($role=="" or .type==$role)
| select($nosub=="0" or ((.isSidechain // false)|not))
| select((.isMeta // false)|not)
| select($since=="" or ((.timestamp // "") >= $since))
| select($proj=="" or ((.cwd // "")|ascii_downcase|contains($proj)))
| . as $m
| blocks[] | select(type=="string")
| split("\n")[]
| select(test($pat;"i"))
| [ (($m.timestamp // "")[0:16] | sub("T";" ")),
    (($m.cwd // "?") | split("/") | last),
    ($m.type[0:4]),
    (($m.sessionId // "")[0:8]),
    (gsub("\\s+";" ") | ltrimstr(" ") | clip($chars)) ]
| @tsv
'

fmt() {
  if [ -t 1 ]; then
    awk -F'\t' '{printf "\033[2m%s\033[0m \033[36m%-16.16s\033[0m \033[35m%-4s\033[0m \033[2m%s\033[0m  %s\n",$1,$2,$3,$4,$5}'
  else
    awk -F'\t' '{printf "%s %-16.16s %-4s %s  %s\n",$1,$2,$3,$4,$5}'
  fi
}

find_session() {
  find "$PROJDIR" -name "$1*.jsonl" -print -quit 2>/dev/null
}

cmd_show() {
  local id="${1:-}" f
  [ -z "$id" ] && { echo "cs show <session-id>" >&2; exit 2; }
  f=$(find_session "$id")
  [ -z "$f" ] && { echo "no session matching '$id'" >&2; exit 1; }
  echo "# $f" >&2
  jq -r '
    def blocks:
      (.message.content // empty) as $c
      | if ($c|type)=="string" then [$c]
        elif ($c|type)=="array" then
          [ $c[]
            | if .type=="text" then .text
              elif .type=="thinking" then "[thinking] "+.thinking
              elif .type=="tool_use" then "[tool: "+(.name//"?")+"] "+((.input|tostring)[0:400])
              elif .type=="tool_result" then "[result] "+((.content|tostring)[0:400])
              else empty end ]
        else [] end;
    select(.type=="user" or .type=="assistant")
    | select((.isMeta // false)|not)
    | ((.timestamp//"")[0:16]|sub("T";" ")) as $t
    | (if .type=="user" then "YOU " else "CC  " end) as $r
    | blocks[] | select(type=="string" and (.|length)>0)
    | "\n=== \($r) \($t) ===\n" + .
  ' "$f"
}

cmd_sessions() {
  local filt="${1:-}"
  find "$PROJDIR" -name '*.jsonl' -printf '%T@\t%TY-%Tm-%Td %TH:%TM\t%p\n' 2>/dev/null \
  | sort -rn | cut -f2- \
  | while IFS=$'\t' read -r mt f; do
      local line
      line=$(head -n 400 "$f" 2>/dev/null | jq -r --arg mt "$mt" '
        select(.type=="user" and ((.isMeta//false)|not) and ((.isSidechain//false)|not))
        | (.message.content) as $c
        | (if ($c|type)=="string" then $c
           else ([($c//[])[]? | select(.type=="text") | .text] | join(" ")) end) as $txt
        | select(($txt|type)=="string" and ($txt|gsub("\\s+";"")|length) > 0)
        | [ $mt, ((.cwd // "?")|split("/")|last), "sess",
            ((.sessionId//"")[0:8]), ($txt|gsub("\\s+";" ")|.[0:88]) ] | @tsv' 2>/dev/null | head -1)
      [ -z "$line" ] && continue
      [ -z "$filt" ] || printf '%s' "$line" | grep -qi -- "$filt" || continue
      printf '%s\n' "$line"
    done | fmt
}
# ---- arg parsing ----
[ $# -eq 0 ] && { usage; exit 0; }
case "${1:-}" in
  show)     shift; cmd_show "$@"; exit ;;
  sessions) shift; cmd_sessions "$@"; exit ;;
  -h|--help) usage; exit 0 ;;
esac

proj=""; role=""; tools=0; think=1; since=""; chars=240
files_only=0; nosub=0; prompts=0; interactive=0
while [ $# -gt 0 ]; do
  case "$1" in
    -P|--project) proj=$(printf '%s' "${2:-}" | tr 'A-Z' 'a-z'); shift 2 ;;
    -r|--role)    role="${2:-}"; shift 2 ;;
    -t|--tools)   tools=1; shift ;;
    -T|--no-thinking) think=0; shift ;;
    -s|--since)   since="${2:-}"; shift 2 ;;
    -c|--chars)   chars="${2:-240}"; shift 2 ;;
    -l|--files)   files_only=1; shift ;;
    -n|--no-sub)  nosub=1; shift ;;
    -p|--prompts) prompts=1; shift ;;
    -i|--interactive) interactive=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    --) shift; break ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *)  break ;;
  esac
done
pat="${1:-}"
[ -z "$pat" ] && { usage; exit 2; }

if [ "$prompts" = 1 ]; then
  [ -f "$HIST" ] || { echo "no $HIST" >&2; exit 1; }
  jq -r --arg pat "$pat" --arg proj "$proj" --argjson chars "$chars" '
    select((.display//"")|test($pat;"i"))
    | select($proj=="" or ((.project//"")|ascii_downcase|contains($proj)))
    | [ (.timestamp/1000|strflocaltime("%Y-%m-%d %H:%M")),
        ((.project//"?")|split("/")|last),
        "you ",
        ((.sessionId//"")[0:8]),
        ((.display//"")|gsub("\\s+";" ")|.[0:$chars]) ] | @tsv' "$HIST" \
  | sort | fmt | { [ -t 1 ] && rg --passthru --color=always -i -e "$pat" || cat; }
  exit
fi

mapfile -t FILES < <(rg -l -i --glob '*.jsonl' -e "$pat" "$PROJDIR" 2>/dev/null)
[ "${#FILES[@]}" -eq 0 ] && { echo "no matches" >&2; exit 1; }

if [ "$files_only" = 1 ]; then printf '%s\n' "${FILES[@]}"; exit; fi

run() {
  printf '%s\0' "${FILES[@]}" \
  | xargs -0 jq -r --arg pat "$pat" --arg role "$role" --arg since "$since" \
        --arg proj "$proj" --arg tools "$tools" --arg think "$think" \
        --arg nosub "$nosub" --argjson chars "$chars" "$JQ_PROG" 2>/dev/null \
  | sort
}

if [ "$interactive" = 1 ]; then
  sel=$(run | awk -F'\t' '{printf "%s\t\033[2m%s\033[0m \033[36m%-16.16s\033[0m \033[35m%-4s\033[0m  %s\n",$4,$1,$2,$3,$5}' \
        | fzf --ansi --no-sort --reverse --height=90% --delimiter='\t' --with-nth='2..' \
              --header="Enter = open full session · pattern: $pat" \
              --preview="$0 show {1} 2>/dev/null | head -500" \
              --preview-window=right:55%:wrap)
  [ -z "$sel" ] && exit 0
  exec "$0" show "${sel%%$'\t'*}"
fi
run | fmt | { [ -t 1 ] && rg --passthru --color=always -i -e "$pat" || cat; }
