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
