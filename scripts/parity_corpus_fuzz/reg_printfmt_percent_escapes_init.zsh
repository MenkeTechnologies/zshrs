# --init-extra guard for 44a3b4841c item 2 + item 3.
#
# Replay:
#   scripts/comptab_parity.py \
#     --init-extra scripts/parity_corpus_fuzz/reg_printfmt_percent_escapes_init.zsh \
#     --case 'true ' --keys ctrl-d --compare-attrs --strict-stream
#
# printfmt's escape switch (zle_tricky.c:2438-2535) was unimplemented except
# for `%%`. Every explanation string below exercises one arm of it, including
# the one that gave the bug its name: an UNKNOWN escape must emit NOTHING, not
# leak its letter — `%Hhi%h` rendered as `Hhih`. The listing rows this draws
# are also the guard for item 3, the TCCLEAREOL / space-padding tail every
# listing row is terminated with (c:2576-2588, c:2539-2549).
#
# This cannot be a corpus entry: it needs a completer that does not exist on
# the host, which is exactly the gap --init-extra was added to close.
_zpf_reg_pf() {
  compadd -J gB -X '%Bbold%b'      -- b1 b2
  compadd -J gU -X '%Uunder%u'     -- u1 u2
  compadd -J gS -X '%Sstand%s'     -- s1 s2
  compadd -J gF -X '%F{red}fg%f'   -- f1 f2
  compadd -J gK -X '%K{blue}bg%k'  -- k1 k2
  compadd -J gH -X '%Hhi%h'        -- h1 h2
  compadd -J gZ -X '%{X%}zero'     -- z1 z2
  compadd -J gP -X '100%% pct'     -- p1 p2
}
compdef _zpf_reg_pf true
