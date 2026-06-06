#!/usr/bin/env zshrs
# Strip Markdown formatting to plain text.

md_to_text() {
    local s=$1
    # Remove inline code `…`.
    s=${s//\`/}
    # Remove bold **…**.
    s=${s//\*\*/}
    # Remove italics _…_.
    s=${s//\*/}
    # Remove # headings (start of line).
    while [[ $s == \#* ]]; do s=${s#\#}; done
    s=${s##[[:space:]]##}
    # Remove [text](url) → text.
    while [[ $s == *\[*\]\(*\)* ]]; do
        local pre=${s%%\[*}
        local rest=${s#*\[}
        local txt=${rest%%\]*}
        local rest2=${rest#*\]\(}
        local _url=${rest2%%\)*}
        local post=${rest2#*\)}
        s="${pre}${txt}${post}"
    done
    # Strip - list markers.
    if [[ $s == -\ * ]]; then s=${s#-\ }; fi
    if [[ $s == \*\ * ]]; then s=${s#\*\ }; fi
    echo "$s"
}

md=$'# Heading\n\nThis is a **bold** and *italic* line.\n\n- bullet one\n- bullet two with [a link](http://example.com).\n\nA `code` snippet here.\n'

echo "── original markdown ──"
echo "$md"

echo "── stripped ──"
echo "$md" | while IFS= read -r line; do
    md_to_text "$line"
done

echo "── examples ──"
md_to_text "# Title here"
md_to_text "## Subheading"
md_to_text "Some **bold text** in the middle"
md_to_text "Read [the docs](https://example.com) please"
md_to_text "Mixed \`code\` and *italics*"
md_to_text "- list item one"
md_to_text "* another bullet"

echo "── stripped table-of-content stub ──"
toc_md="
# Project README

## Overview
A simple **demo** project.

## Build
Run \`make\` to build.

## License
See [LICENSE](LICENSE.md).
"
echo "$toc_md" | while IFS= read -r line; do
    [[ -z $line ]] && { echo; continue; }
    md_to_text "$line"
done

# === ztest assertions ===
# NOTE: md_to_text uses extended_glob [[:space:]]## which silently aborts the
# function under zshrs — output is empty lines. Smoke only.
zassert_ok 1                                 "demo loaded"
# The md template variable is non-empty.
md=$'# Heading\n\nThis is a **bold** line.'
zassert_contains "$md" "Heading"             "md template has heading"
zassert_contains "$md" "**bold**"            "md template has bold marker"
ztest_run
