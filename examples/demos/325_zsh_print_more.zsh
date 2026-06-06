#!/usr/bin/env zshrs
# print builtin — more flags + edge cases.
# Ports Src/builtin.c bin_print.

echo "── print -r (raw, no escape interpretation) ──"
print -r "tab\there"
print -r "newline\nhere (NOT interpreted)"
print -r "literal $RANDOM (var expanded but escape kept)"

echo
echo "── print -R (raw + no leading -opt parsing) ──"
print -R - "looks like an option"
print -R --x "starts with --"

echo
echo "── print -- end-of-options ──"
print -- -not-an-option
print -l -- -x -y -z

echo
echo "── print -n (no trailing newline) ──"
print -n "no_newline"
print "+continues"

echo
echo "── print -nr combined ──"
print -nr "raw\nno-newline"
print "+after"

echo
echo "── print -l list mode (one per line) ──"
print -l alpha beta gamma delta

echo
echo "── print -aC: column layout ──"
echo "  -aC 2:"
print -aC 2 one two three four five six seven eight | sed 's/^/    /'
echo "  -aC 4:"
print -aC 4 one two three four five six seven eight | sed 's/^/    /'
echo "  -aC 3 with longer items:"
print -aC 3 short medium-length much-longer-item tiny enormous-string compact | sed 's/^/    /'

echo
echo "── print -P (prompt expansion) ──"
print -P "  %F{green}%n%f@%F{cyan}%m%f"
print -P "  exit_status: %?"
print -P "  pid: %!"
print -P "  date: %D"
print -P "  time: %T"

echo
echo "── print -D (directory abbreviation) ──"
print -D "$HOME/projects/zshrs"
print -D "/usr/local/bin/zsh"
print -D "$HOME"

echo
echo "── print -f (format like printf) ──"
print -f "%-10s | %5d | %.2f\n" alice 1000 1.23
print -f "%-10s | %5d | %.2f\n" bob 250 9.87
print -f "%-10s | %5d | %.2f\n" carol 9999 0.001

echo
echo "── print -v (capture to var) ──"
print -v captured "hello $USER"
echo "  captured = '$captured'"

print -v multi -l a b c d
echo "  multi (joined with \\n): "
printf '%s' "$multi" | sed 's/^/    /'
echo

echo
echo "── print -u (write to specific fd) ──"
exec 3>/tmp/print_u_demo.$$
print -u3 "to fd 3"
exec 3>&-
if [[ -e /tmp/print_u_demo.$$ ]]; then
    echo "  fd-3 captured: $(cat /tmp/print_u_demo.$$)"
    command rm -f /tmp/print_u_demo.$$
fi

echo
echo "── print -s (push to history) ──"
print -s "history-injected-cmd-1"
print -s "history-injected-cmd-2"
echo "  (would land in HISTFILE in interactive mode)"

echo
echo "── print -z (push to editor buffer) ──"
print -z "buffer-line-1"
echo "  (would appear pre-typed in next prompt)"

echo
echo "── print -N (null terminator like find -print0) ──"
print -N alpha beta gamma | xxd 2>/dev/null | head -3 | sed 's/^/  /' || echo "  (xxd unavailable)"

echo
echo "── print -m (pattern arg filter) ──"
print -m 'a*' apple alfalfa banana avocado cherry artichoke 2>/dev/null | sed 's/^/  /' || echo "  (-m unsupported)"

echo
echo "── print -o / -O (sort args) ──"
print -o zebra apple mango banana | sed 's/^/  asc:  /'
print -O zebra apple mango banana | sed 's/^/  desc: /'

echo
echo "── print -i (case-insensitive sort) ──"
print -oi Zebra apple Mango banana | sed 's/^/  /'

echo
echo "── print -e (enable escape, default in some shells) ──"
print -e "tab\there\nnewline"

echo
echo "── print -E (disable escape, like -r) ──"
print -E "literal: \\t\\n"

echo
echo "── echo vs print -- escape default ──"
echo "  echo:        " "$(echo "tab\there")"
echo "  print:       " "$(print "tab\there")"
echo "  print -r:    " "$(print -r "tab\there")"
echo "  print -e:    " "$(print -e "tab\there")"

# === ztest assertions ===
zassert_eq "$(print -r 'a\tb')" 'a\tb'    "print -r raw"
zassert_eq "$(print -l a b c)"  "$(printf 'a\nb\nc')" "print -l one per line"
zassert_eq "$(print -n hi)"     "hi"      "print -n no newline"
zassert_eq "$(print -o c a b)"  "a b c"   "print -o sort asc"
zassert_eq "$(print -O c a b)"  "c b a"   "print -O sort desc"
print -v captured "hello world"
zassert_eq "$captured"          "hello world" "print -v captures"
zassert_eq "$(print -m 'a*' apple banana avocado)" "apple avocado" "print -m filter"
ztest_run
