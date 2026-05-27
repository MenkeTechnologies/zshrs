#!/bin/sh
# Generate corpus_dash_fc_bulk_{al..au} for rounds 1-10 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=1 END_ROUND=10 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
