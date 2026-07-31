# Every function-declaration syntax zsh accepts. RECORDER.md surface
# `function` is supposed to cover all of these — and now does, after
# the lexer/parser fix that made `{` lex as Inbrace inside a funcdef
# header even when the preceding name String reset incmdpos.
#
# Every count below was verified against zsh 5.9 by running the line on
# its own and listing ${(ok)functions}.
#
# 1. POSIX paren form
paren_form() { echo p; }
# 2. zsh keyword form (the previously-broken case)
function kw_form { echo k; }
# 3. Mixed: keyword + paren
function mixed_form() { echo mx; }
# 4. Empty body, paren form
empty_paren() {}
# 5. Empty body, keyword form. The brace pair MUST be spaced: `{}` with
#    no space lexes as a single word, so `function empty_kw {}` reads as
#    a two-NAME declaration (`empty_kw`, `{}`) whose body is still
#    pending — zsh then swallows following lines looking for it and
#    defines nothing here.
function empty_kw { }
# 6. Multi-name keyword form (one declaration installs N names)
function multi_a multi_b multi_c { echo many; }
# 7. Keyword form with -T trace flag — `-T` is consumed as a flag
function -T traced_form { echo tr; }
# 8. `-U` is NOT a funcdef flag (unlike -T): zsh takes it as an ordinary
#    NAME, so this declaration installs BOTH `-U` and `uniq_form`.
function -U uniq_form { echo u; }
# 9. autoload (lazy-load registration)
autoload lazy_one
autoload -U lazy_two lazy_three
# Total events expected:
#   paren=1 + kw=1 + mixed=1 + empty_paren=1 + empty_kw=1
#   + multi(3) + traced=1 + uniq(2) + autoload(3) = 14
