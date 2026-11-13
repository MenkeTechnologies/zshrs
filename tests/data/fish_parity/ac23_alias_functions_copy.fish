alias ll 'ls -l'; alias ll | string match -q '*ls -l*'; echo $status
alias | string match -q 'alias ll*'; echo $status
functions -e ll; functions -q ll; echo $status
function a1; end; function a2; end; 
functions --copy a1 a3; echo $status; functions -q a3; echo $status
functions -c a1 a4; functions -q a4; echo $status
functions -n | string match -q a1; echo $status
function one_line; echo hi; end; functions one_line
function empty; end; functions empty
alias ll 'ls -l'
functions -e ll; echo e=$status
functions -q ll; echo q=$status
alias l3 'ls -l'
functions -q l3; echo q3=$status
functions -e l3
functions -q l3; echo q3b=$status
type -q ll; echo t=$status
function a1; echo hi; end
functions --copy a1 a3; echo c=$status; a3
functions -c a1 a4; a4
functions a1
function e1; end; functions e1
function e2
end; functions e2
function e3; echo; end; functions e3
function e4 --description x; end; functions e4
function e5
  # c
end; functions e5
