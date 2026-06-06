#!/usr/bin/env zshrs
# Parse git-log-style output — author, date, subject.

parse_git_log() {
    typeset -A entry
    while IFS= read -r line; do
        if [[ $line == commit\ * ]]; then
            # New commit; flush previous.
            if [[ -n ${entry[hash]+x} ]]; then
                printf "  [%s] %s — %s | %s\n" \
                    "${entry[hash]:0:7}" \
                    "${entry[author]:-?}" \
                    "${entry[date]:-?}" \
                    "${entry[subject]:-?}"
            fi
            entry=()
            entry[hash]=${line#commit }
        elif [[ $line == Author:\ * ]]; then
            entry[author]=${line#Author: }
        elif [[ $line == Date:\ * ]]; then
            entry[date]=${line#Date: }
        elif [[ -z ${entry[subject]+x} && -n $line && $line != " "* ]]; then
            entry[subject]=${line##[[:space:]]##}
        fi
    done
    # Final.
    if [[ -n ${entry[hash]+x} ]]; then
        printf "  [%s] %s — %s | %s\n" \
            "${entry[hash]:0:7}" \
            "${entry[author]:-?}" \
            "${entry[date]:-?}" \
            "${entry[subject]:-?}"
    fi
}

setopt extended_glob

cat <<'EOF' | parse_git_log
commit 1234567890abcdef1234567890abcdef12345678
Author: Alice <alice@example.com>
Date:   Wed May 29 14:00:00 2026 +0000

    feat: add new feature

    Longer description here.

commit abcdef1234567890abcdef1234567890abcdef12
Author: Bob <bob@example.com>
Date:   Tue May 28 10:30:00 2026 +0000

    fix: resolve bug in parser

commit deadbeefdeadbeefdeadbeefdeadbeefdeadbeef
Author: Carol <carol@example.com>
Date:   Mon May 27 09:15:00 2026 +0000

    docs: update README
EOF

echo
echo "── short stat extraction ──"
extract_stats() {
    while IFS= read -r line; do
        if [[ $line =~ "^([0-9]+)[[:space:]]+([0-9]+)[[:space:]]+(.+)$" ]]; then
            local added=${match[1]} removed=${match[2]} file=${match[3]}
            printf "  +%-4d -%-4d  %s\n" $added $removed "$file"
        fi
    done
}

cat <<'EOF' | extract_stats
5       2       src/main.rs
12      0       src/lib.rs
0       8       src/old.rs
3       3       README.md
EOF

echo
echo "── extract commit subjects only ──"
extract_subjects() {
    local in_msg=0
    while IFS= read -r line; do
        if [[ $line == commit\ * ]]; then
            in_msg=0
        elif [[ $line == "    "* && $in_msg == 0 ]]; then
            echo "  ${line##[[:space:]]##}"
            in_msg=1
        fi
    done
}

cat <<'EOF' | extract_subjects
commit aaaa
Author: A <a@x>
Date:   Today

    first commit

commit bbbb
Author: B <b@x>
Date:   Today

    second commit
EOF

# === ztest assertions ===
subs=$(cat <<'EOF' | extract_subjects
commit aaaa
Author: A <a@x>
Date:   Today

    first commit

commit bbbb
Author: B <b@x>
Date:   Today

    second commit
EOF
)
zassert_contains "$subs" "first commit"   "subject 1 extracted"
zassert_contains "$subs" "second commit"  "subject 2 extracted"

stats=$(cat <<'EOF' | extract_stats
5       2       src/main.rs
12      0       src/lib.rs
0       8       src/old.rs
3       3       README.md
EOF
)
zassert_contains "$stats" "src/main.rs"   "stats: main.rs row"
zassert_contains "$stats" "+12"           "stats: lib.rs additions"
zassert_contains "$stats" "-8"            "stats: old.rs deletions"
zassert_contains "$stats" "README.md"     "stats: README.md row"
ztest_run
