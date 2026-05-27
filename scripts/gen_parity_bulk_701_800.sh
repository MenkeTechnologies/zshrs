#!/bin/sh
# Generate corpus_dash_fc_bulk_{adw..ahr} for rounds 701-800 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=701 END_ROUND=800 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
