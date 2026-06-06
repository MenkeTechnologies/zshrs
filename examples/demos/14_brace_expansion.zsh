#!/usr/bin/env zshrs
# Brace expansion — comma-list, numeric range, letter range, stepped, zero-pad.

echo "── comma list ──"
for x in {red,green,blue}; do echo $x; done

echo "── numeric range ──"
echo {1..5}

echo "── numeric range descending ──"
echo {5..1}

echo "── stepped range ──"
echo {0..20..5}

echo "── zero-padded ──"
echo {01..10}

echo "── letter range ──"
echo {a..f}

echo "── cross product ──"
echo {a,b}{1,2,3}

echo "── nested ──"
echo {{a,b},{c,d}}

echo "── prefix/suffix ──"
echo file_{01..05}.txt

# === ztest assertions ===
zassert_eq "$(echo {1..5})"           "1 2 3 4 5"             "numeric range asc"
zassert_eq "$(echo {5..1})"           "5 4 3 2 1"             "numeric range desc"
zassert_eq "$(echo {0..20..5})"       "0 5 10 15 20"          "stepped range"
zassert_eq "$(echo {01..10})"         "01 02 03 04 05 06 07 08 09 10" "zero-padded range"
zassert_eq "$(echo {a..f})"           "a b c d e f"           "letter range"
zassert_eq "$(echo {a,b}{1,2,3})"     "a1 a2 a3 b1 b2 b3"     "cross product"
zassert_eq "$(echo {{a,b},{c,d}})"    "a b c d"               "nested braces flatten"
zassert_eq "$(echo file_{01..05}.txt)" "file_01.txt file_02.txt file_03.txt file_04.txt file_05.txt" "prefix/suffix"
words=( {red,green,blue} )
zassert_eq "${#words[@]}" "3"          "comma-list yields 3 words"
zassert_eq "${words[2]}"  "green"      "comma-list[2]"
ztest_run
