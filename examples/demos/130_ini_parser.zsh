#!/usr/bin/env zshrs
# INI file parser in pure zsh — sections + key=value.

typeset -A INI_DATA

parse_ini() {
    local line section="" key val
    while IFS= read -r line; do
        # Strip leading/trailing whitespace.
        line=${line##[[:space:]]##}
        line=${line%%[[:space:]]##}
        # Skip empty lines and comments.
        [[ -z $line || $line == \#* || $line == \;* ]] && continue
        # Section header [name].
        if [[ $line == \[*\] ]]; then
            section=${line:1:-1}
            INI_DATA[__section__${section}]=1
            continue
        fi
        # key=value.
        if [[ $line == *=* ]]; then
            key=${line%%=*}
            val=${line#*=}
            # Strip ws around key.
            key=${key%%[[:space:]]##}
            val=${val##[[:space:]]##}
            # Strip surrounding quotes.
            if [[ $val == \"*\" ]]; then val=${val:1:-1}; fi
            INI_DATA["${section}.${key}"]=$val
        fi
    done
}

show_ini() {
    local sections=()
    for k in ${(k)INI_DATA}; do
        [[ $k == __section__* ]] && sections+=( ${k##__section__} )
    done
    for sec in ${(o)sections}; do
        echo "[${sec}]"
        for k in ${(o)${(k)INI_DATA}}; do
            if [[ $k == ${sec}.* ]]; then
                local name=${k#${sec}.}
                echo "  ${name} = ${INI_DATA[$k]}"
            fi
        done
        echo
    done
}

tmpini=/tmp/zshrs_ini_$$
cat > "$tmpini" <<EOF
# global config
[database]
host = localhost
port = 5432
user = admin
password = "s3cret"

[server]
bind_addr = 0.0.0.0
port = 8080
workers = 4

[logging]
; level can be debug/info/warn/error
level = info
file  = /var/log/app.log
EOF

echo "── parse ──"
while IFS= read -r line; do
    line=${line##[[:space:]]##}
    line=${line%%[[:space:]]##}
    [[ -z $line || $line == \#* || $line == \;* ]] && continue
    if [[ $line == \[*\] ]]; then
        section=${line:1:-1}
        INI_DATA[__section__${section}]=1
        continue
    fi
    if [[ $line == *=* ]]; then
        key=${line%%=*}
        val=${line#*=}
        key=${key%%[[:space:]]##}
        val=${val##[[:space:]]##}
        if [[ $val == \"*\" ]]; then val=${val:1:-1}; fi
        INI_DATA["${section}.${key}"]=$val
    fi
done < "$tmpini"
echo "parsed ${#INI_DATA[@]} entries"

echo "── pretty-print ──"
show_ini

echo "── direct lookup ──"
echo "DB host: ${INI_DATA[database.host]}"
echo "DB port: ${INI_DATA[database.port]}"
echo "DB password (was quoted): ${INI_DATA[database.password]}"
echo "Server workers: ${INI_DATA[server.workers]}"
echo "Log file: ${INI_DATA[logging.file]}"

rm -f "$tmpini"
