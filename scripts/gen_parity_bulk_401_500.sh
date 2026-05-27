#!/bin/sh
# Generate corpus_dash_fc_bulk_{pv..tq} for rounds 401-500 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=401 END_ROUND=500 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
