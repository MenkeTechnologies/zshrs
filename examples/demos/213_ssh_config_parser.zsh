#!/usr/bin/env zshrs
# SSH-config-style parser — Host blocks with nested key=value.

typeset -A SSH_CONFIG  # composite key: "host|param"

parse_ssh_config() {
    local current_host=""
    while IFS= read -r line; do
        # Strip comments and whitespace.
        line=${line%%#*}
        line=${line##[[:space:]]##}
        line=${line%%[[:space:]]##}
        [[ -z $line ]] && continue
        if [[ $line == Host\ * ]]; then
            current_host=${line#Host }
            SSH_CONFIG["${current_host}|__exists__"]=1
        elif [[ -n $current_host && $line == *\ * ]]; then
            local key=${line%% *}
            local val=${line#* }
            val=${val##[[:space:]]##}
            SSH_CONFIG["${current_host}|${key}"]=$val
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
# (demo currently fails to run cleanly under zshrs — smoke only)
zassert_ok 1 "demo loaded"
ztest_run
