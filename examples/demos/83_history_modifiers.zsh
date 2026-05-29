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
