#!/usr/bin/env zshrs
# extended_glob — comprehensive zsh extended-glob feature pin.
# Ports Src/pattern.c::patcompile + Src/glob.c::patcompile.

setopt extended_glob 2>/dev/null

tmpdir=$(mktemp -d)
# Build a fixture tree.
touch $tmpdir/a.txt $tmpdir/b.txt $tmpdir/c.log $tmpdir/d.bak
touch $tmpdir/foo.py $tmpdir/bar.py $tmpdir/baz.rb
touch $tmpdir/test1.zsh $tmpdir/test2.zsh $tmpdir/test10.zsh
touch $tmpdir/.hidden_file
mkdir -p $tmpdir/sub1 $tmpdir/sub2 $tmpdir/.hidden_dir
touch $tmpdir/sub1/nested.txt $tmpdir/sub2/another.txt

cd $tmpdir

echo "── basic globbing ──"
echo "  *.txt: $(echo *.txt)"
echo "  *.py:  $(echo *.py)"
echo "  *:     $(echo *)"

echo
echo "── ^pattern (not matching) ──"
echo "  ^*.txt files:"
echo "    $(echo ^*.txt | tr ' ' '\n' | sort -u | tr '\n' ' ')"

echo "  ^*.{txt,log}:"
echo "    $(echo ^*.{txt,log})"

echo
echo "── ~pattern (exclusion) ──"
echo "  *.txt~b*: $(echo *.txt~b*)"
echo "  *~*.txt~*.py: $(echo *~*.txt~*.py 2>/dev/null)"

echo
echo "── # (zero or more) ──"
echo "  test#.zsh (zero or more test): $(echo (test)#.zsh 2>/dev/null)"

echo
echo "── ## (one or more) ──"
echo "  [0-9]##.zsh (one+ digits): no direct match in tree"

echo
echo "── alternation (a|b) ──"
echo "  *.(txt|log): $(echo *.(txt|log))"
echo "  (foo|baz).*: $(echo (foo|baz).*)"

echo
echo "── numeric range <a-b> ──"
echo "  test<1-5>.zsh: $(echo test<1-5>.zsh)"
echo "  test<-5>.zsh:  $(echo test<-5>.zsh)"
echo "  test<10->.zsh: $(echo test<10->.zsh)"
echo "  test<5-15>.zsh: $(echo test<5-15>.zsh)"

echo
echo "── ksh-style patterns (req kshglob) ──"
setopt kshglob 2>/dev/null
echo "  ?(*.txt): $(echo ?(*.txt) 2>/dev/null || echo n/a)"
echo "  +(*.zsh): $(echo +(*.zsh) 2>/dev/null || echo n/a)"
echo "  *(*.log): $(echo *(*.log) 2>/dev/null || echo n/a)"
echo "  @(*.py|*.rb): $(echo @(*.py|*.rb) 2>/dev/null || echo n/a)"
echo "  !(*.zsh): $(echo !(*.zsh) 2>/dev/null || echo n/a)"
unsetopt kshglob 2>/dev/null

echo
echo "── glob qualifiers ──"
echo "  *(.):     regular files only"
ls -1 *(.) 2>/dev/null | sed 's/^/    /'

echo "  *(/):     directories only"
ls -d *(/) 2>/dev/null | sed 's/^/    /'

echo "  *(*):     executable"
ls *(*N) 2>/dev/null | sed 's/^/    /'

echo "  *(@):     symlinks"
ls *(@N) 2>/dev/null | sed 's/^/    /' || echo "    (none)"

echo "  *(L0):    size 0"
ls *(L0N) 2>/dev/null | sed 's/^/    /'

echo "  *(om):    sort by mtime (newest first)"
ls *(omN) 2>/dev/null | head -5 | sed 's/^/    /'

echo "  *(oL):    sort by size"
ls *(oLN) 2>/dev/null | head -5 | sed 's/^/    /'

echo
echo "── recursive **/ ──"
echo "  **/*.txt:"
echo "    $(echo **/*.txt)"

echo "  **/*.zsh: (none in subdirs)"
echo "    $(echo **/*.zsh)"

echo
echo "── multiple qualifiers ──"
echo "  *(.L0)    (regular + size 0):"
ls *(.L0N) 2>/dev/null | sed 's/^/    /'

echo "  *(.om[1]) (newest regular):"
ls *(.om[1]N) 2>/dev/null | sed 's/^/    /'

echo
echo "── nullglob option ──"
setopt local_options nullglob 2>/dev/null
echo "  no-match returns empty array:"
arr=( *.xyz )
echo "    arr size: ${#arr}"

echo
echo "── glob via case-insensitive (#i) ──"
touch $tmpdir/Lowercase.txt $tmpdir/UPPERCASE.txt
echo "  files (case mixed):"
ls *case* 2>/dev/null | sed 's/^/    /'
echo "  (#i)lower*: $(echo (#i)lower* 2>/dev/null)"
echo "  (#i)*case*.txt:"
echo "    $(echo (#i)*case*.txt 2>/dev/null)"

echo
echo "── glob via approximate matching (#a) ──"
echo "  (#a1)test1.zsh allow 1 error:"
echo "    $(echo (#a1)test1.zsh 2>/dev/null || echo n/a)"

echo
echo "── backrefs (#m) ──"
echo "  capture letters - check first match:"
# (zsh-specific match data var.)
for f in test1.zsh; do
    if [[ $f =~ test([0-9])\.zsh ]]; then
        echo "    file=$f match[1]=${match[1]}"
    fi
done

cd /tmp
command rm -rf $tmpdir

echo
echo "── stats ──"
echo "  extended_glob enables:"
echo "    ^pat       — negation"
echo "    pat~pat    — exclusion"
echo "    (a|b)      — alternation (more powerful than {a,b})"
echo "    #          — zero or more (one of the most useful in compsys)"
echo "    ##         — one or more"
echo "    <a-b>      — numeric range"
echo "    [^x]       — char negation"
echo "    (#i)       — case-insensitive"
echo "    (#a1)      — approximate match with 1 error"
echo "  glob qualifiers handle file metadata in the pattern."

# === ztest assertions ===
# Build fresh fixture (the demo's tmpdir was already torn down).
ztd=$(mktemp -d)
touch $ztd/a.txt $ztd/b.txt $ztd/c.log
touch $ztd/foo.py $ztd/bar.py
touch $ztd/t1.zsh $ztd/t2.zsh $ztd/t10.zsh
cd $ztd
setopt extended_glob 2>/dev/null
txts=( *.txt )
zassert_eq "${#txts}" 2 "txt count"
pys=( *.py )
zassert_eq "${#pys}" 2 "py count"
notxt=( ^*.txt )
zassert_contains "${notxt[*]}" "foo.py" "negation includes py"
alts=( *.(txt|log) )
zassert_eq "${#alts}" 3 "alternation txt|log"
nrange=( t<1-5>.zsh )
zassert_contains "${nrange[*]}" "t1.zsh"  "range includes t1"
zassert_contains "${nrange[*]}" "t2.zsh"  "range includes t2"
# case-insensitive
touch $ztd/Cap.txt
ci=( (#i)cap.txt )
zassert_contains "${ci[*]}" "Cap.txt" "case-insensitive match"
cd /tmp
command rm -rf $ztd
ztest_run
