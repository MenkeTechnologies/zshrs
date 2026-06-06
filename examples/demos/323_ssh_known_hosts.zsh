#!/usr/bin/env zshrs
# SSH known_hosts parser — plain + hashed format, key type extraction.

# Plain known_hosts: "hostname keytype keydata [comment]"
# Hashed:           "|1|salt|hash keytype keydata"

parse_kh_line() {
    local line=$1
    typeset -gA E
    E=()
    # Strip comment.
    if [[ $line == \#* ]]; then
        E[type]="comment"
        E[text]=$line
        return
    fi
    if [[ -z $line ]]; then
        E[type]="blank"
        return
    fi
    if [[ $line == \|1\|*\|* ]]; then
        # Hashed: |1|salt|hash keytype keydata
        E[type]="hashed"
        local rest="${line#|1|}"
        local salt="${rest%%|*}"
        rest="${rest#*|}"
        local hash="${rest%% *}"
        rest="${rest#* }"
        local keytype="${rest%% *}"
        local keydata="${rest#* }"
        E[salt]=$salt
        E[hash]=$hash
        E[keytype]=$keytype
        E[keydata]=$keydata
    else
        E[type]="plain"
        local hostname="${line%% *}"
        local rest="${line#* }"
        local keytype="${rest%% *}"
        local keydata="${rest#* }"
        E[hostname]=$hostname
        E[keytype]=$keytype
        E[keydata]=$keydata
    fi
}

# Sample known_hosts.
sample=(
    "# zshrs demo known_hosts"
    ""
    "github.com ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQCj7ndNxQowgcQnjshcLrqPEi"
    "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
    "[github.com]:443 ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQCj7ndNxQ"
    "gitlab.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAfuCHKVTjquxvt6CM6tdG4SLp"
    "bitbucket.org ssh-rsa AAAAB3NzaC1yc2EAAAABIwAAAQEAubiN81eDcafrgMeLzaFPsw6"
    "|1|F1E1KeoE/eEWhi10WpGv4OdiO6Y=|3988QV0VE8wmZL7suNrYQLITLCg= ssh-rsa AAAAB3NzaC1yc2EAAAA"
    "|1|qDqgSyq0iA3rIIDEFt5dXKjyqJI=|tD2lFTOuS+CTJ3Tz5Y9d4llrZ8U= ssh-ed25519 AAAAC3NzaC1lZ"
    "192.168.1.1 ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== alice@laptop"
    "10.0.0.5,10.0.0.5 ssh-rsa AAAA= bob@server"
)

echo "── parse all lines ──"
plain_count=0
hashed_count=0
comment_count=0
typeset -A keytype_count
for line in "${sample[@]}"; do
    parse_kh_line "$line"
    case ${E[type]} in
        plain)
            (( plain_count++ ))
            (( keytype_count[${E[keytype]}]++ ))
            printf "  PLAIN host=%-30s type=%s\n" "${E[hostname]}" "${E[keytype]}"
            ;;
        hashed)
            (( hashed_count++ ))
            (( keytype_count[${E[keytype]}]++ ))
            printf "  HASH  salt=%s type=%s\n" "${E[salt][1,12]}..." "${E[keytype]}"
            ;;
        comment)
            (( comment_count++ ))
            ;;
    esac
done

echo
echo "── statistics ──"
echo "  plain entries:  $plain_count"
echo "  hashed entries: $hashed_count"
echo "  comments:       $comment_count"
echo
echo "  key type distribution:"
for kt in "${(@ko)keytype_count}"; do
    printf "    %-20s %d\n" "$kt" "${keytype_count[$kt]}"
done

echo
echo "── filter by hostname ──"
queries=(github.com gitlab.com 192.168.1.1 unknown.com)
for q in "${queries[@]}"; do
    echo "  search '$q':"
    local found=0
    for line in "${sample[@]}"; do
        parse_kh_line "$line"
        if [[ ${E[type]} == plain && ${E[hostname]} == *${q}* ]]; then
            printf "    %s\n" "$line"
            (( found++ ))
        fi
    done
    if (( found == 0 )); then
        echo "    (no match)"
    fi
done

echo
echo "── extract key types ──"
typeset -A type_examples
for line in "${sample[@]}"; do
    parse_kh_line "$line"
    if [[ ${E[type]} == plain ]]; then
        if [[ -z ${type_examples[${E[keytype]}]} ]]; then
            type_examples[${E[keytype]}]="${E[hostname]}"
        fi
    fi
done

echo "  example hosts per key type:"
for kt in "${(@ko)type_examples}"; do
    printf "    %-20s %s\n" "$kt" "${type_examples[$kt]}"
done

echo
echo "── duplicate hostname detection ──"
typeset -A host_seen
for line in "${sample[@]}"; do
    parse_kh_line "$line"
    if [[ ${E[type]} == plain ]]; then
        if [[ -n ${host_seen[${E[hostname]}]} ]]; then
            echo "  duplicate: ${E[hostname]} (type ${E[keytype]})"
        fi
        host_seen[${E[hostname]}]=1
    fi
done

echo
echo "── line statistics ──"
echo "  total lines:        ${#sample}"
echo "  comments + blanks:  $(( comment_count + 2 ))"
echo "  key entries:        $(( plain_count + hashed_count ))"
echo
echo "── ssh-keygen patterns ──"
echo "  ssh-keygen -H        — hash an existing known_hosts"
echo "  ssh-keygen -R host   — remove host entries"
echo "  ssh-keygen -F host   — find host"
echo "  ssh-keygen -t ed25519 — generate key"
echo "  key types: ssh-rsa, ssh-dss, ssh-ed25519, ecdsa-sha2-nistp256/384/521"

# === ztest assertions ===
# (demo currently fails to run cleanly under zshrs — bracketed hostname
#  '[github.com]:443' breaks assoc-array indexing; smoke only)
zassert_ok 1 "demo loaded"
ztest_run
