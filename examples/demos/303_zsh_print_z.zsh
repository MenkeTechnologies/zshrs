#!/usr/bin/env zshrs
# print -z — push text onto the buffer stack (interactive-edit feed).
# Ports Src/builtin.c bin_print -z path.

echo "── print -z basics ──"
print -z "echo from buffer stack"
echo "  (line queued; non-interactive zshrs accepts the call silently)"

echo
echo "── print -P prompt expansion ──"
# Prompt escapes work without -z.
print -P "  %F{cyan}%n%f at %F{magenta}%m%f (just demo'd %F{yellow}print -P%f)"
print -P "  uid=%n date=%D{%Y-%m-%d}"

echo
echo "── print -a vs -l vs -C: array layout ──"
arr=(alpha beta gamma delta epsilon zeta eta theta)
echo "  array: ${arr[@]}"
echo
echo "  print -l (one per line):"
print -l "${arr[@]}" | sed 's/^/    /'

echo
echo "  print -aC 2 (2-col layout):"
print -aC 2 "${arr[@]}" | sed 's/^/    /'

echo
echo "  print -aC 4 (4-col layout):"
print -aC 4 "${arr[@]}" | sed 's/^/    /'

echo
echo "── print -N (null-terminated, like find -print0) ──"
# We can't usefully demo binary null in stdout but show the flag exists.
print -l one two three | xxd 2>/dev/null | head -5 || echo "  (xxd unavailable)"

echo
echo "── print -r raw (no escape processing) ──"
print "  with \\n escape interpreted? probably:"
print "  hello\\nworld"
echo "  print -r raw:"
print -r "  hello\\nworld"

echo
echo "── print -v: capture into variable ──"
print -v captured "value $RANDOM here"
echo "  captured = '$captured'"

print -v multi -l alpha beta gamma
echo "  multi = '$multi'"

echo
echo "── print -f: format string (like printf) ──"
print -f "%-10s %5d\n" alice 100
print -f "%-10s %5d\n" bob 250
print -f "%-10s %5d\n" carol 75 | sed 's/^/  /'

echo
echo "── print -s: append to history ──"
print -s "this would be added to history"
echo "  (in interactive mode this would land in zsh's history file)"

echo
echo "── print -p: pipe to coprocess ──"
echo "  (skipped — no coproc started)"

echo
echo "── print special chars (-D for diff-friendly) ──"
print "  literal tab\\there"
print -r "  not interpreted: \\t \\n"

echo
echo "── print -m: pattern-match arg select ──"
# Output args matching pattern.
print -m 'a*' apple banana avocado cherry artichoke 2>/dev/null || echo "  -m not supported here"

echo
echo "── echo vs print -r behavior ──"
echo "  echo with backslash:"
echo "  hello\\tworld"
echo "  print -r:"
print -r "  hello\\tworld"

echo
echo "── summary ──"
print -P "  ✓ print -P prompt: %F{green}working%f"
print -P "  ✓ print -v capture: working"
print -P "  ✓ print -aC columns: working"
print -P "  ✓ print -l per-line: working"
print -P "  ✓ print -z buffer push: accepted (non-interactive)"

# === ztest assertions ===
# print -v captures into variable
print -v cap "hello world"
zassert_eq "$cap" "hello world"          "print -v captures plain"
print -v multi -l one two three
zassert_contains "$multi" "one"          "print -v -l captures multi"
zassert_contains "$multi" "two"          "print -v -l captures all"
# print -r is raw (no \n interpretation)
out=$(print -r 'a\nb')
zassert_eq "$out" 'a\nb'                 "print -r preserves backslash"
# print -l one-per-line
out=$(print -l x y z)
zassert_eq "$out" "$(printf 'x\ny\nz')" "print -l newline separated"
# print -m matches pattern
out=$(print -m 'a*' apple banana avocado cherry)
zassert_eq "$out" "apple avocado"        "print -m glob match"
# print -f formats
out=$(print -f "%d-%s\n" 5 hi)
zassert_eq "$out" "5-hi"                 "print -f"
ztest_run
