#!/usr/bin/env zshrs
# URL template (RFC 6570 subset) — substitute {var} placeholders.

typeset -A VARS

expand_template() {
    local tmpl=$1 out=""
    local i=1 len=${#tmpl}
    # NB: every local is declared here — a bare `local n` re-declaration inside
    # the loop echoes "n=VALUE" onto stdout and corrupts the expanded URL.
    local ch close var_start var_end var_name op name v n
    while (( i <= len )); do
        ch="${tmpl[i]}"
        if [[ $ch == "{" ]]; then
            close=$(( i + 1 ))
            local close_char='}'
            while (( close <= len )); do
                local cc="${tmpl[close]}"
                if [[ $cc == $close_char ]]; then
                    break
                fi
                (( close++ ))
            done
            if (( close <= len )); then
                var_start=$(( i + 1 ))
                var_end=$(( close - 1 ))
                var_name="${tmpl[$var_start,$var_end]}"
                op=""
                name="$var_name"
                local first_ch="${var_name[1]}"
                if [[ $first_ch == '?' ]]; then
                    op='?'
                    name="${var_name[2,-1]}"
                elif [[ $first_ch == '&' ]]; then
                    op='&'
                    name="${var_name[2,-1]}"
                elif [[ $first_ch == '+' ]]; then
                    op='+'
                    name="${var_name[2,-1]}"
                elif [[ $first_ch == '#' ]]; then
                    op='#'
                    name="${var_name[2,-1]}"
                elif [[ $first_ch == '/' ]]; then
                    op='/'
                    name="${var_name[2,-1]}"
                fi
                local -a names
                names=( ${(s:,:)name} )
                local first_in_op=1
                for n in "${names[@]}"; do
                    v="${VARS[$n]:-}"
                    if [[ -n $v ]]; then
                        if [[ $op == '?' ]]; then
                            if (( first_in_op )); then
                                out+="?${n}=${v}"
                                first_in_op=0
                            else
                                out+="&${n}=${v}"
                            fi
                        elif [[ $op == '&' ]]; then
                            out+="&${n}=${v}"
                        elif [[ $op == '+' || $op == '#' ]]; then
                            out+="${v}"
                        elif [[ $op == '/' ]]; then
                            out+="/${v}"
                        else
                            if (( ! first_in_op )); then out+=","; fi
                            out+="${v}"
                            first_in_op=0
                        fi
                    fi
                done
                i=$(( close + 1 ))
                continue
            fi
        fi
        out+="$ch"
        (( i++ ))
    done
    echo "$out"
}

echo "── basic substitution ──"
VARS=( user alice id 42 )
templates=(
    "/users/{user}"
    "/users/{id}/profile"
    "/api/{user}/posts/{id}"
    "{user}@example.com"
    "no/placeholders"
)
for t in "${templates[@]}"; do
    printf "  %s → %s\n" "$t" "$(expand_template "$t")"
done

echo
echo "── query string expansion (? operator) ──"
VARS=( page 2 limit 50 q rust )
templates2=(
    "/search{?q}"
    "/search{?q,page}"
    "/search{?q,page,limit}"
    "/items{?page}"
)
for t in "${templates2[@]}"; do
    printf "  %s → %s\n" "$t" "$(expand_template "$t")"
done

echo
echo "── path expansion (/ operator) ──"
VARS=( api v2 resource users action create )
templates3=(
    "{/api,resource}"
    "{/api,resource,action}"
    "/static{/v2}"
)
for t in "${templates3[@]}"; do
    printf "  %s → %s\n" "$t" "$(expand_template "$t")"
done

echo
echo "── fragment expansion (# operator) ──"
VARS=( section setup anchor install )
templates4=(
    "/docs{#section}"
    "/docs{#anchor}"
    "/docs/install{#section}"
)
for t in "${templates4[@]}"; do
    printf "  %s → %s\n" "$t" "$(expand_template "$t")"
done

echo
echo "── undefined variables removed ──"
VARS=( user bob )
templates5=(
    "/u/{user}/{missing}"
    "{?defined,missing,user}"
    "{user}-{missing}"
)
for t in "${templates5[@]}"; do
    printf "  %s → %s\n" "$t" "$(expand_template "$t")"
done

echo
echo "── REST endpoint composition ──"
VARS=(
    base    /api/v2
    user_id 42
    repo    zshrs
    branch  main
    commit  abc123
    page    1
    per     30
)
templates6=(
    "{+base}/users/{user_id}"
    "{+base}/users/{user_id}/repos/{repo}"
    "{+base}/users/{user_id}/repos/{repo}/commits/{commit}"
    "{+base}/repos/{repo}/branches/{branch}{?page,per}"
)
for t in "${templates6[@]}"; do
    printf "  %s\n  → %s\n\n" "$t" "$(expand_template "$t")"
done

# === ztest assertions ===
VARS=( user alice id 42 )
zassert_eq "$(expand_template 'no/placeholders')"        "no/placeholders"        "literal passthrough"
zassert_eq "$(expand_template '/users/{user}')"          "/users/alice"           "{var} substitution"
zassert_eq "$(expand_template '/api/{user}/posts/{id}')" "/api/alice/posts/42"    "two placeholders"
zassert_eq "$(expand_template '{user}@example.com')"     "alice@example.com"      "leading placeholder"
VARS=( user bob )
zassert_eq "$(expand_template '/users/{user}/{missing}')" "/users/bob/"           "unset var expands to empty"
VARS=( q rust page 2 )
zassert_eq "$(expand_template '/search{?q,page}')"       "/search?q=rust&page=2"  "? query operator"
ztest_run
