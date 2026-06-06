#!/usr/bin/env zshrs
# zsh history file parser — extended/raw formats, frequency analysis.

# zsh history extended format: ": timestamp:duration;command"
# raw format:                 "command"

parse_history_line() {
    local line=$1
    typeset -gA H
    H=()
    if [[ $line == :*\;* ]]; then
        # Extended.
        H[format]="extended"
        local meta="${line#:}"
        meta="${meta%%;*}"
        local cmd="${line#*;}"
        local ts=${meta% *}
        local dur=${meta#* }
        # Trim whitespace.
        ts="${ts## }"; ts="${ts%% }"
        dur="${dur## }"; dur="${dur%% }"
        H[timestamp]=$ts
        H[duration]=$dur
        H[cmd]=$cmd
    else
        H[format]="raw"
        H[cmd]=$line
    fi
}

# Sample history (mix of extended + raw).
sample_history=(
    ": 1717000000:0;ls -la"
    ": 1717000050:1;cd /tmp"
    ": 1717000100:3;vim README.md"
    ": 1717000200:0;git status"
    ": 1717000250:2;git add -A"
    ": 1717000300:1;git commit -m \"update\""
    ": 1717000400:5;git push"
    "ls"
    "pwd"
    ": 1717000500:0;cd ~/projects"
    ": 1717000600:10;cargo build"
    ": 1717000700:120;cargo test"
    ": 1717000900:1;exit"
)

echo "── parse + display ──"
for entry in "${sample_history[@]}"; do
    parse_history_line "$entry"
    if [[ ${H[format]} == extended ]]; then
        printf "  [%s] (%ss) %s\n" "${H[timestamp]}" "${H[duration]}" "${H[cmd]}"
    else
        printf "  [raw]            %s\n" "${H[cmd]}"
    fi
done

echo
echo "── command frequency ──"
typeset -A cmd_count
for entry in "${sample_history[@]}"; do
    parse_history_line "$entry"
    local cmd_word="${H[cmd]%% *}"
    (( cmd_count[$cmd_word]++ ))
done

# Sort by count desc.
sorted_cmds=( "${(@k)cmd_count}" )
n=${#sorted_cmds}
for ((i=1; i<=n; i++)); do
    for ((j=i+1; j<=n; j++)); do
        if (( cmd_count[${sorted_cmds[i]}] < cmd_count[${sorted_cmds[j]}] )); then
            tmp=${sorted_cmds[i]}
            sorted_cmds[i]=${sorted_cmds[j]}
            sorted_cmds[j]=$tmp
        fi
    done
done

for cmd in "${sorted_cmds[@]}"; do
    c=${cmd_count[$cmd]}
    bar=""
    for ((b=0; b<c; b++)); do bar+="█"; done
    printf "  %-15s %2d  %s\n" "$cmd" "$c" "$bar"
done

echo
echo "── duration analysis ──"
total_dur=0
count_dur=0
max_dur=0
max_cmd=""
for entry in "${sample_history[@]}"; do
    parse_history_line "$entry"
    if [[ ${H[format]} == extended ]]; then
        d=${H[duration]:-0}
        (( total_dur += d ))
        (( count_dur++ ))
        if (( d > max_dur )); then
            max_dur=$d
            max_cmd=${H[cmd]}
        fi
    fi
done
if (( count_dur > 0 )); then
    avg=$(( total_dur / count_dur ))
    echo "  total commands w/ duration: $count_dur"
    echo "  total seconds: $total_dur"
    echo "  avg duration:  $avg s"
    echo "  slowest:       '$max_cmd' ($max_dur s)"
fi

echo
echo "── unique commands ──"
echo "  total entries: ${#sample_history}"
echo "  unique cmds:   ${#cmd_count}"
echo "  uniqueness:    $(( ${#cmd_count} * 100 / ${#sample_history} ))%"

echo
echo "── timeline ──"
echo "  first ts: $(parse_history_line "${sample_history[1]}"; echo ${H[timestamp]})"
parse_history_line "${sample_history[-1]}"
last_ts=${H[timestamp]:-N/A}
echo "  last ts:  $last_ts"

echo
echo "── filter: commands containing 'git' ──"
for entry in "${sample_history[@]}"; do
    parse_history_line "$entry"
    if [[ ${H[cmd]} == *git* ]]; then
        printf "    %s\n" "${H[cmd]}"
    fi
done

echo
echo "── format diversity ──"
ext_count=0
raw_count=0
for entry in "${sample_history[@]}"; do
    parse_history_line "$entry"
    if [[ ${H[format]} == extended ]]; then
        (( ext_count++ ))
    else
        (( raw_count++ ))
    fi
done
echo "  extended: $ext_count"
echo "  raw:      $raw_count"

echo
echo "── notable zsh HISTFILE patterns ──"
echo "  Src/hist.c:"
echo "    histsave()       — write entries"
echo "    readhistfile()   — parse on startup"
echo "    histstrcmp()     — dedup"
echo "    HIST_FCNTL_LOCK  — atomic write w/ flock"
echo "  format flags:"
echo "    EXTENDED_HISTORY — w/ ts + duration"
echo "    INC_APPEND_HISTORY — write each cmd immediately"
echo "    HIST_IGNORE_DUPS  — skip duplicates"
echo "    SHARE_HISTORY     — across sessions"

# === ztest assertions ===
# Re-run parse for a known entry to assert.
parse_history_line ": 1717000000:0;ls -la"
zassert_eq "${H[format]}" "extended" "ext fmt detected"
zassert_eq "${H[cmd]}"    "ls -la"   "ext cmd extracted"
parse_history_line "ls"
zassert_eq "${H[format]}" "raw"      "raw fmt detected"
zassert_eq "${H[cmd]}"    "ls"       "raw cmd"
zassert_eq "${#sample_history}" 13   "sample size"
zassert_eq "$ext_count" 11           "extended count"
zassert_eq "$raw_count" 2            "raw count"
zassert_eq "${cmd_count[git]}" 4     "git frequency"
zassert_eq "${cmd_count[cd]}"  2     "cd frequency"
ztest_run
