# Minimal hanging input for D04parameter#135 "zsh_eval_context resizing",
# the assertion where the whole file stops (118 later assertions never run).
#
#   zsh   -f d04parameter_135_nested_anon_fn.zsh   -> prints 49 immediately
#   zshrs -f d04parameter_135_nested_anon_fn.zsh   -> does not finish
#
# NOT a deadlock and NOT an infinite loop: parsing nested anonymous functions
# costs zshrs roughly 3x per added nesting level, so upstream's `repeat 48`
# (49 levels, a 253-byte program at depth 36) runs off the end of the 150 s
# per-file timeout.  Measured against zshrs 0.12.49 @29ee728e:
#
#   depth   zsh      zshrs -c    zshrs -n (parse only)
#   24      0.008 s  0.396 s     0.150 s
#   28      0.008 s  1.164 s     0.415 s
#   32      0.017 s  3.509 s     1.433 s
#   36      0.008 s  13.182 s    3.722 s
#
# -n (noexec) blows up too, so the cost is in the parse/compile path, not in
# execution.  Braces, subshells and nested `eval` at the same depth are all
# flat in both shells; only `() { ... }` is superlinear.

emulate -R zsh
integer depth=${1:-49}
local cmd=':'
repeat depth-1 cmd="() { $cmd }"
eval $cmd
print -r -- "completed at depth $depth"
