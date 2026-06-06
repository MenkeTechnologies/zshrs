#!/usr/bin/env zshrs
# Catalog — concise tour of 20 zsh features that work in zshrs today.
# Each line cites the upstream Src/*.c port site.

setopt extended_glob

echo "── 1. arithmetic with bases (Src/math.c lexconstant) ──"
echo "0xff=$((0xff)) 2#1010=$((2#1010))"

echo "── 2. brace expansion (Src/glob.c bracecomplete) ──"
echo {a..e} {01..05}

echo "── 3. parameter expansion (Src/subst.c paramsubst) ──"
s=Hello
echo "len=${#s} upper=${s:u} lower=${s:l}"

echo "── 4. modifiers (Src/hist.c) ──"
p=/usr/local/bin/git
echo "h=${p:h} t=${p:t}"

echo "── 5. associative arrays (Src/params.c PM_HASHED) ──"
typeset -A m=(k1 v1 k2 v2)
echo "k1=${m[k1]}"

echo "── 6. functions + locals (Src/exec.c runshfunc) ──"
f() { local x=local; echo "x=$x"; }
f

echo "── 7. anonymous fn (Src/exec.c is_anonymous_function_name) ──"
() { echo "anon: $1"; } hello

echo "── 8. command sub (Src/exec.c getoutput) ──"
echo "today: $(date +%Y-%m-%d 2>/dev/null || echo unknown)"

echo "── 9. process sub (Src/exec.c getoutputfile) ──"
diff <(echo same) <(echo same) > /dev/null && echo "process-sub equal"

echo "── 10. heredoc (Src/exec.c gethere) ──"
cat <<EOF
heredoc body
EOF

echo "── 11. typeset -i base (Src/params.c PM_INTEGER) ──"
typeset -i 16 hex=255; echo "hex=$hex"

echo "── 12. regex match (Src/cond.c cond_match) ──"
[[ "abc123" =~ "([a-z]+)([0-9]+)" ]] && echo "letters=${match[1]} digits=${match[2]}"

echo "── 13. case alt (Src/exec.c execcase) ──"
case foo in foo|bar) echo "yes" ;; *) echo no ;; esac

echo "── 14. ${(@kv)assoc} pairs ──"
for k v in "${(@kv)m}"; do echo "  $k → $v"; done

echo "── 15. ${(P)var} indirection ──"
ref=s; echo "(P)ref=${(P)ref}"

echo "── 16. zmodload math (Src/Modules/mathfunc.c) ──"
zmodload zsh/mathfunc 2>/dev/null
echo "sqrt(2) = $(( sqrt(2) ))"

echo "── 17. zmodload datetime (Src/Modules/datetime.c) ──"
zmodload zsh/datetime 2>/dev/null
TZ=UTC strftime "%Y-%m-%d" 1736942400

echo "── 18. zparseopts (Src/Modules/zutil.c) ──"
set -- -v -d arg1
zparseopts -D -E v=v d=d
echo "v=$v d=$d rest=$*"

echo "── 19. arith assignment on assoc (Src/math.c setmathvar) ──"
typeset -A counts; counts[a]=10; (( counts[a]++ )); echo "counts[a]=${counts[a]}"

echo "── 20. array subtract/intersect (Src/subst.c) ──"
A=(1 2 3 4); B=(2 4 6)
echo "A-B=${A:|B} A&B=${A:*B}"

# === ztest assertions ===
zassert_eq $((0xff))          255   "hex literal"
zassert_eq $((2#1010))         10   "binary literal"
zassert_eq "${#s}"              5   "param expansion length"
zassert_eq "${s:u}"        "HELLO"  "uppercase mod"
zassert_eq "${s:l}"        "hello"  "lowercase mod"
zassert_eq "${p:h}"   "/usr/local/bin"  "head modifier"
zassert_eq "${p:t}"   "git"             "tail modifier"
zassert_eq "${m[k1]}"  "v1"   "assoc lookup"
zassert_eq "${counts[a]}"  11  "arith on assoc"
# zshrs divergence: ${A:|B} word-splits even inside double quotes when passed
# directly to a builtin — use an intermediate scalar to force join.
sub=${A:|B}
inter=${A:*B}
zassert_eq "$sub"   "1 3"   "array subtract"
zassert_eq "$inter" "2 4"   "array intersect"
ztest_run
