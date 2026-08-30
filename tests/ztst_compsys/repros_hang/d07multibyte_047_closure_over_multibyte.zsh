# Minimal hanging input for D07multibyte#47 "Raw bytes don't match multibyte
# characters part 2", the assertion where the whole file stops (9 later
# assertions never run).
#
#   zsh   -f d07multibyte_047_closure_over_multibyte.zsh  -> rc=0, exits
#   zshrs -f d07multibyte_047_closure_over_multibyte.zsh  -> spins at 100% CPU
#
# An infinite loop in the pattern matcher, not a blocked read: 4.95 s of 5.00 s
# wall was user time.  The trigger is the EXTENDED_GLOB closure operator `#`
# applied to a multibyte literal.  Locale-independent (C and en_US.UTF-8 both
# hang) and specific to the bare literal -- the same character wrapped in a
# group, `(é)#`, returns immediately.  ASCII `a#` is fine.
#
# Upstream's line is `[[ éé != é#$'\xa9' ]]`; the raw byte is not needed.

emulate -R zsh
setopt extendedglob
print -r -- "ascii closure:      $( [[ aa = a#   ]] && print yes || print no )"
print -r -- "grouped multibyte:  $( [[ éé = (é)# ]] && print yes || print no )"
print -r -- "bare multibyte closure follows -- zshrs does not return from this:"
[[ éé = é# ]]
print -r -- "returned rc=$?"
