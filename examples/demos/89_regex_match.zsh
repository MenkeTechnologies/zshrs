#!/usr/bin/env zshrs
# Regex matching via [[ str =~ pattern ]] + $match / $MATCH globals.
# Ported from Src/cond.c cond_match + Src/Modules/regex.c (zsh/regex).

zmodload zsh/regex 2>/dev/null || true

echo "── simple anchored ──"
if [[ "hello world" =~ "^hello" ]]; then echo "yes: starts with hello"; fi
if [[ "hello world" =~ "world$" ]]; then echo "yes: ends with world"; fi
if [[ "hello world" =~ "xyz" ]]; then echo "ERR"; else echo "no: xyz not found"; fi

echo "── captures ──"
[[ "alice 30 admin" =~ "^([a-z]+) ([0-9]+) ([a-z]+)$" ]]
echo "match count: ${#match[@]}"
echo "match[1]: ${match[1]}"
echo "match[2]: ${match[2]}"
echo "match[3]: ${match[3]}"
echo "full: $MATCH"

echo "── key=value ──"
[[ "name=Alice" =~ "^([^=]+)=(.*)$" ]]
echo "key=${match[1]} val=${match[2]}"

echo "── IP address ──"
[[ "192.168.1.1" =~ "^([0-9]+)\.([0-9]+)\.([0-9]+)\.([0-9]+)$" ]]
echo "octets: ${match[1]} ${match[2]} ${match[3]} ${match[4]}"

echo "── email-ish ──"
[[ "user@example.com" =~ "^([^@]+)@([^.]+)\.(.+)$" ]]
echo "user=${match[1]} host=${match[2]} tld=${match[3]}"

echo "── extract numbers from text ──"
data="The year is 2026 and the month is 5"
[[ $data =~ "year is ([0-9]+).*month is ([0-9]+)" ]]
echo "year=${match[1]} month=${match[2]}"

echo "── repeated match (consume + loop) ──"
log="ERROR foo bar | ERROR baz qux | INFO hello | ERROR last"
count=0
while [[ $log =~ "ERROR ([^|]+)" ]]; do
    (( count++ ))
    echo "match $count: ${match[1]}"
    log=${log#*ERROR ${match[1]}}
done
echo "found $count ERROR occurrences"
