# --init-extra probe for b122d9cbe1 — `zle-line-pre-redraw` never ran.
#
# *** DIVERGES TODAY. *** Measured 2026-08-30 against target/debug/zshrs
# 0.12.49 @10:38 vs zsh 5.9.2, 46.8 s, fingerprint 0602148f26:
#
#     zsh   : @CT@ ls /usr [PRD]
#     zshrs : @CT@ ls /usr
#
# Replay:
#   scripts/comptab_parity.py \
#     --init-extra scripts/parity_corpus_fuzz/reg_zle_pre_redraw_hook_init.zsh \
#     --case 'ls /usr' --keys tab
#
# CONTROL, which rules out `POSTDISPLAY` being the thing that is missing: the
# identical widget bound to `zle-line-init` instead renders ` [LI]` on BOTH
# shells (PASS, 23.3 s, both grids `@CT@ ls /usr [LI]`) —
# reg_zle_line_init_control_init.zsh. So POSTDISPLAY works and the hook is
# what does not fire.
#
# Why it matters beyond this cell: `redrawhook` (zle_main.c:1066) is where
# every zsh syntax highlighter repaints $region_highlight. b122d9cbe1 reports
# porting it synchronously, C body and all, after finding it had been queued
# onto PENDING_HOOKS for a host drain no shell code performs. This cell says
# the hook still does not reach a widget bound with `zle -N`.
_zpf_reg_prd() { POSTDISPLAY=' [PRD]' }
zle -N zle-line-pre-redraw _zpf_reg_prd
