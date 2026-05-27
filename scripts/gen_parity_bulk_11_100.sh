#!/bin/sh
# Generate corpus_dash_fc_bulk_{av..eg} for rounds 11-100 (wrapper).
# See gen_parity_bulk_rounds.sh for arbitrary START_ROUND/END_ROUND.

set -e
dir=$(dirname "$0")
START_ROUND=11 END_ROUND=100 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
