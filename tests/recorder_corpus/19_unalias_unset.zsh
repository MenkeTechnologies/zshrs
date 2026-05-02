# RECORDER.md "Open question 4": removal events. Recorder captures
# `unalias` and `unset` so `zwhere -l NAME` lineage shows the full
# define→unset→redefine chain. Each removal is a distinct kind so
# query-side can filter (e.g. `zwhere -k unalias` to find every
# alias-removal site).
alias x=cmd1
alias y=cmd2
unalias x
unalias y
TOUNSET1=1
TOUNSET2=2
unset TOUNSET1
unset TOUNSET2
