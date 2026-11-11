source @FN@/contains_seq.fish
contains_seq b c -- a b c d; echo $status
contains_seq b d -- a b c d; echo $status
contains_seq -- a b; echo $status
