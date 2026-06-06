#!/usr/bin/env zshrs
# Shebang detector — read first line, classify by interpreter.

classify_shebang() {
    local f=$1 first
    if [[ ! -r $f ]]; then echo "unreadable"; return; fi
    IFS= read -r first < $f
    if [[ $first != \#!* ]]; then echo "none"; return; fi
    # Strip #! and leading whitespace.
    local rest="${first#\#!}"
    rest="${rest## }"
    # Handle /usr/bin/env LANG style.
    if [[ $rest == */env* ]]; then
        # Strip path/env + spaces.
        local after=${rest#*env }
        local lang=${after%% *}
        echo "env:$lang"
        return
    fi
    # Plain /path/to/interp [args].
    local interp=${rest%% *}
    local base=${interp##*/}
    echo "direct:$base"
}

tmpdir=$(mktemp -d)
mkdir -p $tmpdir

# Build test fixtures.
cat > $tmpdir/zshrs_script.zsh <<'EOF'
#!/usr/bin/env zshrs
echo "zshrs"
EOF

cat > $tmpdir/zsh_direct.zsh <<'EOF'
#!/bin/zsh
echo "zsh"
EOF

cat > $tmpdir/bash_script.sh <<'EOF'
#!/bin/bash
echo "bash"
EOF

cat > $tmpdir/python.py <<'EOF'
#!/usr/bin/env python3
print("py")
EOF

cat > $tmpdir/perl.pl <<'EOF'
#!/usr/bin/perl
print "perl\n";
EOF

cat > $tmpdir/node.js <<'EOF'
#!/usr/bin/env node
console.log("js")
EOF

cat > $tmpdir/ruby.rb <<'EOF'
#!/usr/bin/env ruby
puts "ruby"
EOF

cat > $tmpdir/no_shebang.txt <<'EOF'
plain text file
no shebang here
EOF

cat > $tmpdir/awk.awk <<'EOF'
#!/usr/bin/awk -f
{ print }
EOF

cat > $tmpdir/sed.sed <<'EOF'
#!/usr/bin/sed -f
s/old/new/g
EOF

cat > $tmpdir/sh.sh <<'EOF'
#!/bin/sh
echo "sh"
EOF

cat > $tmpdir/stryke.stk <<'EOF'
#!/usr/bin/env stryke
say "stryke!"
EOF

echo "── classify all ──"
files=($tmpdir/*)
for f in "${files[@]}"; do
    printf "  %-30s → %s\n" "${f##*/}" "$(classify_shebang $f)"
done

echo
echo "── stats by category ──"
typeset -A categories
for f in "${files[@]}"; do
    cat=$(classify_shebang $f)
    (( categories[$cat]++ ))
done
for k in "${(@k)categories}"; do
    printf "  %-20s × %d\n" "$k" "${categories[$k]}"
done

echo
echo "── filter scripts only (have shebang) ──"
for f in "${files[@]}"; do
    cat=$(classify_shebang $f)
    if [[ $cat != none && $cat != unreadable ]]; then
        printf "  %s\n" "${f##*/}"
    fi
done

command rm -rf $tmpdir

# === ztest assertions ===
tmpdir2=$(mktemp -d)
cat > $tmpdir2/a <<'EOF'
#!/usr/bin/env zshrs
EOF
cat > $tmpdir2/b <<'EOF'
#!/bin/bash
EOF
cat > $tmpdir2/c <<'EOF'
#!/usr/bin/perl
EOF
cat > $tmpdir2/d <<'EOF'
plain text
EOF
cat > $tmpdir2/e <<'EOF'
#!/usr/bin/awk -f
EOF
zassert_eq "$(classify_shebang $tmpdir2/a)" "env:zshrs"   "env:zshrs"
zassert_eq "$(classify_shebang $tmpdir2/b)" "direct:bash" "direct:bash"
zassert_eq "$(classify_shebang $tmpdir2/c)" "direct:perl" "direct:perl"
zassert_eq "$(classify_shebang $tmpdir2/d)" "none"        "no shebang"
zassert_eq "$(classify_shebang $tmpdir2/e)" "direct:awk"  "direct:awk -f arg dropped"
zassert_eq "$(classify_shebang $tmpdir2/nonexistent)" "unreadable" "missing file"
command rm -rf $tmpdir2
ztest_run
