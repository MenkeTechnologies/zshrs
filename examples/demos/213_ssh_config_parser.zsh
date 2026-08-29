#!/usr/bin/env zshrs
# SSH-config-style parser — Host blocks with nested key=value.

typeset -A SSH_CONFIG  # composite key: "host|param"

parse_ssh_config() {
    # NB: quotes inside an assoc subscript are stored literally, so build the
    # composite key in a variable first (declared here, not inside the loop).
    local current_host="" key val ckey
    while IFS= read -r line; do
        # Strip comments and whitespace.
        line=${line%%\#*}   # NB: EXTENDED_GLOB makes a bare `#` a closure op
        line=${line##[[:space:]]##}
        line=${line%%[[:space:]]##}
        [[ -z $line ]] && continue
        if [[ $line == Host\ * ]]; then
            current_host=${line#Host }
            ckey="${current_host}|__exists__"
            SSH_CONFIG[$ckey]=1
        elif [[ -n $current_host && $line == *\ * ]]; then
            key=${line%% *}
            val=${line#* }
            val=${val##[[:space:]]##}
            ckey="${current_host}|${key}"
            SSH_CONFIG[$ckey]=$val
        fi
    done
}

setopt extended_glob

config=$(cat <<'EOF'
# Default settings
Host *
    User defaultuser
    Port 22
    IdentityFile ~/.ssh/id_rsa

# Production server
Host prod
    HostName prod.example.com
    User admin
    Port 2222
    IdentityFile ~/.ssh/prod_key
    ForwardAgent yes

# Staging
Host staging
    HostName 10.0.1.50
    User devops
    Port 22
    LogLevel DEBUG

# Bastion
Host bastion
    HostName bastion.example.com
    User jumper
    ProxyCommand none
EOF
)

echo "── parse config ──"
echo "$config" | parse_ssh_config
echo "entries: ${#SSH_CONFIG[@]}"

echo "── list hosts ──"
for k in ${(ko)SSH_CONFIG}; do
    if [[ $k == *__exists__ ]]; then
        echo "  ${k%|*}"
    fi
done

echo "── lookup specific host ──"
show_host() {
    local host=$1
    echo "Host: $host"
    for k in ${(ko)SSH_CONFIG}; do
        if [[ $k == ${host}\|* && $k != *__exists__ ]]; then
            local param=${k#*|}
            printf "  %-15s = %s\n" "$param" "${SSH_CONFIG[$k]}"
        fi
    done
}

show_host prod
show_host staging
show_host bastion

# === ztest assertions ===
zassert_eq "${#SSH_CONFIG[@]}" 19                     "19 parsed entries"
zassert_eq "${SSH_CONFIG[prod|HostName]}" "prod.example.com" "prod HostName"
zassert_eq "${SSH_CONFIG[prod|Port]}"     "2222"            "prod Port"
zassert_eq "${SSH_CONFIG[staging|User]}"  "devops"          "staging User"
zassert_eq "${SSH_CONFIG[*|Port]}"        "22"              "Host * default Port"
zassert_eq "${SSH_CONFIG[bastion|ProxyCommand]}" "none"     "bastion ProxyCommand"
zassert_eq "${SSH_CONFIG[nosuch|Port]}"   ""                "unknown host has no entry"
zassert_contains "$(show_host prod)" "HostName        = prod.example.com" "show_host prints params"
ztest_run
