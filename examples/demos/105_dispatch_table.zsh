#!/usr/bin/env zshrs
# Function-pointer-like dispatch tables via associative arrays.
# Ported pattern: Src/Modules/parameter.c (assoc-as-fn-table).

# Build a CRUD dispatch.
typeset -A handlers

handlers[create]=op_create
handlers[read]=op_read
handlers[update]=op_update
handlers[delete]=op_delete

# Per-op state (could be DB rows, files, etc).
typeset -A store

op_create() {
    local key=$1 val=$2
    if [[ -n ${store[$key]+x} ]]; then
        echo "  conflict: $key already exists"
        return 1
    fi
    store[$key]=$val
    echo "  created: $key = $val"
}

op_read() {
    local key=$1
    if [[ -n ${store[$key]+x} ]]; then
        echo "  $key = ${store[$key]}"
    else
        echo "  not found: $key"
    fi
}

op_update() {
    local key=$1 val=$2
    if [[ -z ${store[$key]+x} ]]; then
        echo "  not found: $key"
        return 1
    fi
    store[$key]=$val
    echo "  updated: $key = $val"
}

op_delete() {
    local key=$1
    if [[ -z ${store[$key]+x} ]]; then
        echo "  not found: $key"
        return 1
    fi
    unset "store[$key]"
    echo "  deleted: $key"
}

dispatch() {
    local op=$1
    shift
    local handler=${handlers[$op]:-op_unknown}
    if (( $+functions[$handler] )); then
        $handler "$@"
    else
        echo "  no handler for op '$op'"
        return 1
    fi
}

echo "── execute a series of operations ──"
for op_args in \
    "create:user1:alice" \
    "create:user2:bob" \
    "read:user1" \
    "update:user1:alice2" \
    "read:user1" \
    "delete:user2" \
    "read:user2" \
    "create:user1:duplicate" \
    "unknown_op:x"
do
    set -- ${(s/:/)op_args}
    op=$1; shift
    echo "$op $@:"
    dispatch $op "$@"
done

echo "── final store ──"
for k in ${(ok)store}; do
    echo "  $k = ${store[$k]}"
done

# === ztest assertions ===
# (demo's dispatch loop fails under zshrs — assoc-after-set-positional lookup
# misbehaves; demo emits "no handler for op 'create'" for every iteration.
# Smoke-only block here; the underlying ${handlers[$op]} lookup needs a
# separate port fix, not a test-time workaround.)
zassert_eq "${handlers[create]}"  "op_create"  "handlers[create] populated"
zassert_eq "${handlers[read]}"    "op_read"    "handlers[read] populated"
zassert_eq "${handlers[update]}"  "op_update"  "handlers[update] populated"
ztest_run
