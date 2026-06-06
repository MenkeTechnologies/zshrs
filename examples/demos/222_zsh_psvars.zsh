#!/usr/bin/env zshrs
# PS1/PS2/PS3/PS4/RPROMPT prompts — config + expansion.
# Ported from Src/prompt.c (putpromptchar) + Src/init.c default values.

echo "── current prompt vars ──"
echo "PS1=$PS1"
echo "PS2=$PS2"
echo "PS3=$PS3"
echo "PS4=$PS4"
echo "RPROMPT=$RPROMPT"

echo
echo "── set custom PS1 ──"
save_PS1=$PS1
PS1='%n@%m %~> '
echo "PS1=$PS1"
echo "expanded: $(print -P $PS1)"

PS1='%(?:%F{green}✓:%F{red}✗)%f %~ ❯ '
echo "PS1=$PS1"
echo "expanded: $(print -P $PS1)"

echo
echo "── various %escape demos ──"
test_prompts=(
    "%n"
    "%M"
    "%~"
    "%d"
    "%T"
    "%D"
    "%j"
    "%(?.OK.FAIL)"
    "%F{red}red%f"
    "%B bold %b"
    "%U under %u"
    "%%"
)
for p in $test_prompts; do
    printf "  %-20s → %s\n" "'$p'" "$(print -P "$p")"
done

echo
echo "── set RPROMPT (right prompt) ──"
RPROMPT='[%D{%H:%M}]'
echo "RPROMPT=$RPROMPT"
echo "expanded: $(print -P $RPROMPT)"
RPROMPT=""

echo
echo "── PS2 for continued input ──"
PS2='... '
echo "PS2 default: '... '"

echo
echo "── PS3 for select-loop ──"
PS3='Pick: '
echo "PS3: $PS3"

echo
echo "── PS4 for xtrace ──"
PS4='+ '
echo "PS4 default: '+ '"

echo
echo "── compose a fancy prompt ──"
fancy_prompt() {
    local user='%n'
    local host='%m'
    local cwd='%~'
    local prompt_char='❯'
    local rcolor='%(?.green.red)'
    PS1="%F{$rcolor}$prompt_char%f $user@$host $cwd > "
}
fancy_prompt
echo "fancy PS1=$PS1"
echo "expanded: $(print -P $PS1)"

# Restore
PS1=$save_PS1

# === ztest assertions ===
zassert_eq "$(print -P '%(?.OK.FAIL)')" "OK"  "%(?...) true branch"
zassert_eq "$(print -P '%%')"           "%"   "%% literal"
zassert_eq "$(print -P 'plain')"        "plain" "non-escape passthrough"
zassert_ok  "$(print -P '%n')"           "%n expands to non-empty"
zassert_ok  "$(print -P '%d')"           "%d expands to cwd"
zassert_contains "$(print -P '%~')" "/"  "%~ contains slash"
zassert_eq "$PS2" "... "                  "PS2 customized"
zassert_eq "$PS3" "Pick: "                "PS3 customized"
ztest_run
