#!/bin/sh
# Generate corpus_dash_fc_bulk_{tr..xm} for rounds 501-600 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=501 END_ROUND=600 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
