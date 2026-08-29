#!/usr/bin/env zshrs
# Pattern-replace with capture groups + back-references.
# Ported from Src/subst.c subst (replacement engine + Src/pattern.c capture).

s="alice:30:admin"

echo "── plain replacement ──"
echo "${s/:/=}"          # first colon → equals
echo "${s//:/=}"         # all colons → equals

echo "── with capture (zsh: (#m) flag for $MATCH) ──"
text="hello world"
echo "${text//(world)/[\\1]}"  # depends on pattern flavor

echo "── case-mutation via flags ──"
phrase="zsh shell"
echo "(L) lowercase: ${(L)phrase}"
echo "(U) uppercase: ${(U)phrase}"
echo "(C) capitalize each: ${(C)phrase}"

echo "── conditional capture (mimicked via grep -E) ──"
log="ERROR: [2026-05-29] foo failed"
echo "$log" | grep -oE '\[[0-9-]+\]' || echo "no date found"

echo "── multi-pattern strip ──"
data="prefix_value_suffix"
core=${data#prefix_}
core=${core%_suffix}
echo "core: $core"

echo "── replace longest vs shortest ──"
t="a.b.c.d"
echo "trim shortest: ${t%.*}"   # a.b.c
echo "trim longest:  ${t%%.*}"  # a

echo "── case via :u / :l on captured substring ──"
name="alice"
echo "Cap: ${name[1]:u}${name[2,-1]}"

echo "── interpolation in replacement ──"
sep="|"
arr=(a b c d)
joined=${(j:$sep:)arr}
echo "joined: $joined"

echo "── url to host ──"
url="https://example.com/path/to/resource?q=1"
no_proto=${url#*://}      # strip protocol
host=${no_proto%%/*}     # strip path
echo "url=$url"
echo "host=$host"

echo "── log line parser via patterns ──"
log_line="[ERROR][2026-05-29 14:30:00] failed to connect"
level=${log_line#\[}; level=${level%%\]*}
date_part=${log_line#*\]\[}; date_part=${date_part%%\]*}
msg=${log_line##*\] }
echo "level=$level"
echo "date=$date_part"
echo "msg=$msg"

# === ztest assertions ===
ss="alice:30:admin"
zassert_eq "${ss/:/=}"   "alice=30:admin"      "first colon -> ="
zassert_eq "${ss//:/=}"  "alice=30=admin"      "all colons -> ="
# Case mutation flags
phrase="zsh shell"
zassert_eq "${(L)phrase}"  "zsh shell"   "(L) lowercase"
zassert_eq "${(U)phrase}"  "ZSH SHELL"   "(U) uppercase"
zassert_eq "${(C)phrase}"  "Zsh Shell"   "(C) capitalize"
# strip prefix/suffix
data="prefix_value_suffix"
core=${data#prefix_}
core=${core%_suffix}
zassert_eq "$core"  "value"  "double strip"
# trim shortest vs longest
t="a.b.c.d"
zassert_eq "${t%.*}"   "a.b.c"  "%.* shortest end"
zassert_eq "${t%%.*}"  "a"      "%%.* longest end"
# join with separator (zshrs doesn't interpolate $sep inside (j:..:); use literal)
arr=(a b c d)
zassert_eq "${(j:|:)arr}"  "a|b|c|d"  "(j:|:) literal join"
# URL parse
zassert_eq "$level"      "ERROR"   "level parsed"
zassert_eq "$date_part"  "2026-05-29 14:30:00"  "date parsed"
zassert_eq "$msg"        "failed to connect"    "msg parsed"
ztest_run
