# Every assignment / reassignment shape on a non-local shell variable.
# This is the recorder's chokepoint contract: any mutation to the
# global shell state via assignment must surface as an event.
#
# Scalar
PROJECT=zshrs
# Scalar concat
PROJECT+=_v2
# Indexed array set
arr=(a b c)
# Indexed array append
arr+=(d e)
# Indexed array element set
arr[1]=replaced
# Assoc declaration + bulk init + element set + element append
typeset -A h
h=(k1 v1 k2 v2)
h[k3]=v3
h[k3]+=tail
# Path-family arrays — set form
path=(/p1 /p2)
fpath=(/fp1)
manpath=(/mp1)
module_path=(/mod1)
cdpath=(/cd1)
# Path-family arrays — append form
path+=(/p3)
fpath+=(/fp2)
module_path+=(/mod2)
cdpath+=(/cd2)
# Scalar PATH-family — set + concat
PATH=/old
FPATH=/oldfp
PATH+=:/new
FPATH+=:/newfp
