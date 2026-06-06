#!/usr/bin/env zshrs
# Simple template renderer — {{var}} substitution from assoc.

render() {
    local template=$1
    local -A ctx
    shift
    # Args come as key=value.
    for kv in "$@"; do
        ctx[${kv%%=*}]=${kv#*=}
    done
    local out=$template
    for k v in "${(@kv)ctx}"; do
        out=${out//\{\{${k}\}\}/$v}
    done
    echo "$out"
}

echo "── basic ──"
render 'Hello, {{name}}!' name=Alice

echo "── multi-var ──"
render 'User: {{user}}, Email: {{email}}, Age: {{age}}' \
    user=bob email=bob@example.com age=30

echo "── repeated keys ──"
render '{{greeting}}, {{name}}. {{greeting}} again!' \
    greeting=Hello name=World

echo "── multi-line template ──"
template='# {{title}}

Welcome to {{site}}!

- Author: {{author}}
- Year: {{year}}

Read more at {{url}}.'

render "$template" \
    title="zshrs demo" \
    site="examples" \
    author="MenkeTechnologies" \
    year=2026 \
    url="https://github.com/MenkeTechnologies/zshrs"

echo "── missing var → literal placeholder ──"
render '{{a}} {{missing}} {{c}}' a=1 c=3

echo "── escape mode (skip rendering inside <code>) ──"
echo '(escape-mode not implemented in this naive renderer)'

echo "── batch render ──"
template='User {{id}}: {{name}} ({{role}})'
data=(
    'id=1|name=Alice|role=admin'
    'id=2|name=Bob|role=user'
    'id=3|name=Carol|role=guest'
)
for line in "${data[@]}"; do
    args=(${(s/|/)line})
    render "$template" "${args[@]}"
done

# === ztest assertions ===
zassert_eq "$(render 'Hello, {{name}}!' name=Alice)" \
           "Hello, Alice!"                                  "basic 1-var"
zassert_eq "$(render 'X={{a}} Y={{b}}' a=1 b=2)" \
           "X=1 Y=2"                                        "multi-var"
zassert_eq "$(render '{{g}}, {{n}}. {{g}} again!' g=Hi n=World)" \
           "Hi, World. Hi again!"                           "repeated key"
zassert_eq "$(render '{{a}} {{missing}} {{c}}' a=1 c=3)" \
           "1 {{missing}} 3"                                "missing key kept literal"
zassert_eq "$(render 'User {{id}}: {{name}} ({{role}})' id=1 name=Alice role=admin)" \
           "User 1: Alice (admin)"                          "batch row 1"
zassert_eq "$(render 'noop')" "noop"                       "no placeholders"
ztest_run
