#!/usr/bin/env zshrs
# Word-modifier syntax on variables — :h :t :r :e :s :gs combined.
# Same dispatch as history substitutions (Src/hist.c histsubchar +
# Src/utils.c word modifier callbacks).

paths=(
    /usr/local/bin/git
    /Users/wizard/.zshrc
    /var/log/system/auth.log
    /tmp/test.tar.gz
    /etc/passwd
)

echo "── batch tail (:t) ──"
print -l ${paths:t}

echo "── batch head (:h) ──"
print -l ${paths:h}

echo "── batch root (:r) ──"
print -l ${paths:r}

echo "── batch ext (:e) ──"
print -l ${paths:e}

echo "── chained :t:r — strip path then extension ──"
print -l ${paths:t:r}

echo "── chained :h:t — get last-dir-name ──"
print -l ${paths:h:t}

echo "── :s substitution per element ──"
print -l ${paths:s|/usr|/USR|}

echo "── chained :t:r:s — name minus ext, then substitute ──"
print -l ${paths:t:r:s/git/GIT/}

echo "── combine via fn ──"
basenames_no_ext() {
    print -l ${1:t:r}
}
basenames_no_ext /home/user/notes.txt
basenames_no_ext /tmp/foo.tar.gz

echo "── on cmd output (paths from command sub) ──"
files=( $(printf '/tmp/a.txt\n/tmp/b.log\n/tmp/c.csv\n') )
echo "names (tail):"
print -l ${files:t}
echo "roots (no ext):"
print -l ${files:r}

# === ztest assertions ===
p=/usr/local/bin/git
zassert_eq "${p:t}" "git"            ":t basename"
zassert_eq "${p:h}" "/usr/local/bin" ":h dirname"
zassert_eq "${p:r}" "/usr/local/bin/git" ":r no-ext on extensionless"
arch=/tmp/test.tar.gz
zassert_eq "${arch:t}"   "test.tar.gz" ":t on dotted name"
zassert_eq "${arch:r}"   "/tmp/test.tar" ":r strips shortest ext"
zassert_eq "${arch:e}"   "gz"           ":e is last ext"
zassert_eq "${arch:t:r}" "test.tar"     ":t:r chain"
# Batch tail on array
paths_arr=(
    /usr/local/bin/git
    /Users/wizard/.zshrc
    /tmp/test.tar.gz
)
tails=("${paths_arr[@]:t}")
heads=("${paths_arr[@]:h}")
zassert_eq "${tails[*]}" "git .zshrc test.tar.gz" "batch :t over array"
zassert_eq "${heads[*]}" "/usr/local/bin /Users/wizard /tmp" "batch :h over array"
# :s substitution
sub_result="${p:s|/usr|/USR|}"
zassert_eq "$sub_result" "/USR/local/bin/git" ":s simple substitution"
# Helper fn
basenames_no_ext() { print ${1:t:r}; }
zassert_eq "$(basenames_no_ext /home/user/notes.txt)" "notes" "fn :t:r"
ztest_run

