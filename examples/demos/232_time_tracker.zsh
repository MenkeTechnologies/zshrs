#!/usr/bin/env zshrs
# Time tracker — log work sessions, summarize by project.

zmodload zsh/datetime 2>/dev/null

typeset -a SESSIONS
typeset -A PROJECT_TOTALS

log_session() {
    local project=$1 duration_min=$2 task=$3
    local now=$EPOCHSECONDS
    SESSIONS+=("$now|$project|$duration_min|$task")
    (( PROJECT_TOTALS[$project] += duration_min ))
}

echo "── log sessions ──"
log_session zshrs 90 "write batch 9 demos"
log_session zshrs 60 "review BUGS.md"
log_session strykelang 45 "design parser"
log_session zshrs 120 "ship demos"
log_session admin 30 "email + reviews"
log_session strykelang 75 "implement lexer"
log_session learning 60 "read paper"
log_session zshrs 50 "fix bugs"
log_session admin 20 "meetings"

echo "── total sessions: ${#SESSIONS[@]} ──"

echo
echo "── by project (minutes) ──"
for proj in ${(ko)PROJECT_TOTALS}; do
    printf "  %-12s %3d min (%dh %dm)\n" $proj ${PROJECT_TOTALS[$proj]} \
        $((${PROJECT_TOTALS[$proj]} / 60)) $((${PROJECT_TOTALS[$proj]} % 60))
done

echo
echo "── grand total ──"
total=0
for v in ${(v)PROJECT_TOTALS}; do (( total += v )); done
echo "  total: $total minutes (= $((total / 60))h $((total % 60))m)"

echo
echo "── distribution ──"
for proj in ${(ko)PROJECT_TOTALS}; do
    pct=$(( PROJECT_TOTALS[$proj] * 100 / total ))
    local bar=""
    local i
    for ((i=0; i<pct/2; i++)); do bar+="█"; done
    printf "  %-12s %3d%% %s\n" $proj $pct "$bar"
done

echo
echo "── session log ──"
for s in "${SESSIONS[@]}"; do
    local parts=( ${(s/|/)s} )
    printf "  %s | %-12s | %4d min | %s\n" "${parts[1]:0:10}" "${parts[2]}" "${parts[3]}" "${parts[4]}"
done | head -10

echo
echo "── longest session ──"
max_min=0
max_proj=""
max_task=""
for s in "${SESSIONS[@]}"; do
    local parts=( ${(s/|/)s} )
    if (( parts[3] > max_min )); then
        max_min=${parts[3]}
        max_proj=${parts[2]}
        max_task=${parts[4]}
    fi
done
echo "  $max_proj: $max_min min — $max_task"

echo
echo "── average session length ──"
echo "  $(( total / ${#SESSIONS[@]} )) min"

# === ztest assertions ===
zassert_eq "${#SESSIONS[@]}" 9 "9 sessions logged"
zassert_eq "${PROJECT_TOTALS[zshrs]}"       320  "zshrs total = 90+60+120+50"
zassert_eq "${PROJECT_TOTALS[strykelang]}"  120  "strykelang total = 45+75"
zassert_eq "${PROJECT_TOTALS[admin]}"        50  "admin total = 30+20"
zassert_eq "${PROJECT_TOTALS[learning]}"     60  "learning total = 60"
zassert_eq "$total"          550  "grand total"
zassert_eq "$max_min"        120  "longest session = 120"
zassert_eq "$max_proj"       "zshrs" "longest session project"
zassert_eq "$max_task"       "ship demos" "longest session task"
ztest_run
