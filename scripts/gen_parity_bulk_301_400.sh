#!/bin/sh
# Generate corpus_dash_fc_bulk_{lz..pu} for rounds 301-400 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=301 END_ROUND=400 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
