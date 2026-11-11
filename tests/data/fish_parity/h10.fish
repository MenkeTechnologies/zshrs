function a --wraps ls --description "x"; echo wrapped; end
a
functions -q a; and echo yes
type -t a
type -t echo
type -q nope_zzz; echo $status
