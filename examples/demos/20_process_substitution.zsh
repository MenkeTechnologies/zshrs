#!/usr/bin/env zshrs
# Process substitution — <(cmd) and >(cmd).

echo "── compare two streams ──"
if diff <(printf "a\nb\nc\n") <(printf "a\nb\nc\n") > /dev/null; then
    echo "streams equal"
fi

echo "── differing streams ──"
diff <(printf "x\ny\nz\n") <(printf "x\nY\nz\n") || true

echo "── read multi-source ──"
paste <(printf "1\n2\n3\n") <(printf "a\nb\nc\n")

echo "── command group as input ──"
sort < <(printf "gamma\nalpha\nbeta\n")

# === ztest assertions ===
diff <(printf "a\nb\nc\n") <(printf "a\nb\nc\n") > /dev/null && r=1 || r=0
zassert_eq "$r" 1 "<(...) diff on equal streams"
diff <(printf "x\ny\n") <(printf "x\nY\n") > /dev/null 2>&1 && r=1 || r=0
zassert_eq "$r" 0 "<(...) diff on differing streams"
zassert_eq "$(paste <(printf '1\n2\n') <(printf 'a\nb\n') | head -1)" "1	a"  "paste with two <() inputs"
zassert_eq "$(sort < <(printf 'gamma\nalpha\nbeta\n') | head -1)" "alpha" "sort < <() first"
zassert_eq "$(sort < <(printf 'gamma\nalpha\nbeta\n') | tail -1)" "gamma" "sort < <() last"
ztest_run
