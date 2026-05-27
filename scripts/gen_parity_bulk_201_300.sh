#!/bin/sh
# Generate corpus_dash_fc_bulk_{if..ly} for rounds 201-300 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=201 END_ROUND=300 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
