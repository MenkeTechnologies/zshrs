#!/bin/sh
# Generate corpus_dash_fc_bulk_{aaa..adv} for rounds 601-700 (wrapper).

set -e
dir=$(dirname "$0")
START_ROUND=601 END_ROUND=700 TESTS_PER_ROUND=${TESTS_PER_ROUND:-48} \
  exec "$dir/gen_parity_bulk_rounds.sh" "$@"
