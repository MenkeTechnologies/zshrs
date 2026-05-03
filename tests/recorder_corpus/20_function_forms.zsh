# Every function-declaration syntax zsh accepts. RECORDER.md surface
# `function` is supposed to cover all of these — and now does, after
# the lexer/parser fix that made `{` lex as Inbrace inside a funcdef
# header even when the preceding name String reset incmdpos.
#
# 1. POSIX paren form
paren_form() { echo p; }
# 2. zsh keyword form (the previously-broken case)
function kw_form { echo k; }
# 3. Mixed: keyword + paren
function mixed_form() { echo mx; }
# 4. Empty body, paren form
empty_paren() {}
# 5. Empty body, keyword form
function empty_kw {}
# 6. Multi-name keyword form (one declaration installs N names)
function multi_a multi_b multi_c { echo many; }
# 7. Keyword form with -T trace flag
function -T traced_form { echo tr; }
# 8. Keyword form with -U flag
function -U uniq_form { echo u; }
# 9. autoload (lazy-load registration)
autoload lazy_one
autoload -U lazy_two lazy_three
# Total events expected:
#   paren=1 + kw=1 + mixed=1 + empty_paren=1 + empty_kw=1
#   + multi(3) + traced=1 + uniq=1 + autoload(3) = 13
