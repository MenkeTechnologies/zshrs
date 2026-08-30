# Control for reg_zle_pre_redraw_hook_init.zsh — the SAME widget body, bound to
# `zle-line-init` instead of `zle-line-pre-redraw`.
#
# Replay:
#   scripts/comptab_parity.py \
#     --init-extra scripts/parity_corpus_fuzz/reg_zle_line_init_control_init.zsh \
#     --case 'ls /usr' --keys tab -v
#
# Measured 2026-08-30, zshrs 0.12.49 @10:38 vs zsh 5.9.2: PASS in 23.3 s, and
# `-v` shows the grid is `@CT@ ls /usr [LI]` — i.e. BOTH shells render it. That
# is what makes the pre-redraw cell a hook finding rather than a POSTDISPLAY
# finding; without this control the two explanations are indistinguishable.
_zpf_reg_li() { POSTDISPLAY=' [LI]' }
zle -N zle-line-init _zpf_reg_li
