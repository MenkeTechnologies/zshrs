#!/usr/bin/env zshrs
# Shell-aware word split via ${(z)var} — tokenizes like the parser.
# Ported from Src/lex.c (subst_string + ssub for shell word splitting).

echo "── basic shell split ──"
cmd='echo hello world "quoted phrase"'
tokens=( ${(z)cmd} )
echo "token count: ${#tokens[@]}"
for ((i=1; i<=${#tokens[@]}; i++)); do
    echo "  [$i] = '${tokens[i]}'"
done

echo "── handles escaped chars ──"
cmd2='ls -la /path/with\ space file.txt'
t2=( ${(z)cmd2} )
echo "token count: ${#t2[@]}"
print -l "${t2[@]}"

echo "── handles single-quoted ──"
cmd3="echo 'hello world' 'with spaces'"
t3=( ${(z)cmd3} )
echo "token count: ${#t3[@]}"
print -l "${t3[@]}"

echo "── handles \$var (lexically — doesn't expand) ──"
cmd4='echo $HOME $USER "abc def"'
t4=( ${(z)cmd4} )
echo "token count: ${#t4[@]}"
print -l "${t4[@]}"

echo "── parse a simple alias body ──"
alias_body='for f in *.txt; do echo "$f"; done'
parts=( ${(z)alias_body} )
echo "parts: ${#parts[@]}"
print -l "${parts[@]}"

echo "── chain of commands ──"
chain='echo a && echo b || echo c | wc -l'
parts=( ${(z)chain} )
echo "tokens (operators included):"
print -l "${parts[@]}"

echo "── compare with naive $= split ──"
text='one two three "four five" six'
echo "via (z):"
print -l ${(z)text}
echo "via \$=:"
print -l $=text

# === ztest assertions ===
cmd='echo hello world "quoted phrase"'
tokens=( ${(z)cmd} )
zassert_eq "${#tokens[@]}" 4         "(z) splits into 4 tokens"
zassert_eq "${tokens[1]}" "echo"     "(z) token 1"
zassert_eq "${tokens[2]}" "hello"    "(z) token 2"
zassert_eq "${tokens[3]}" "world"    "(z) token 3"
zassert_eq "${tokens[4]}" '"quoted phrase"' "(z) token 4 keeps quotes"
# Single-quoted phrase stays as single token.
cmd3="echo 'hello world' 'with spaces'"
t3=( ${(z)cmd3} )
zassert_eq "${#t3[@]}" 3             "(z) single-quoted is one token"
# Chain operators tokenized separately.
chain='echo a && echo b'
parts=( ${(z)chain} )
zassert_eq "${#parts[@]}" 5          "(z) keeps && as own token"
zassert_eq "${parts[3]}" "&&"        "(z) && is token 3"
# (z) vs $= behaviour difference: (z) respects quoting, $= does not.
text='one "two three" four'
zsplit=( ${(z)text} )
naive=( $=text )
zassert_eq "${#zsplit[@]}" 3         "(z) quoting yields 3 tokens"
zassert_eq "${#naive[@]}"  4         "\$= naive split yields 4 tokens"
ztest_run
