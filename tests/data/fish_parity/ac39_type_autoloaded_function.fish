type -t ls
type ls | head -2 | string replace -r '^(# Defined in embedded:).*' '$1'
type -q ls; echo $status
type -t nosuchcmd_x; echo $status
