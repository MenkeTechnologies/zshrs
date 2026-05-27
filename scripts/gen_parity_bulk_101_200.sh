#!/bin/sh
# Generate corpus_dash_fc_bulk_{eh..ic} for rounds 101-200 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=101 END_ROUND=200 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
