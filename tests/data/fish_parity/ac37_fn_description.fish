function zf -d 'my d'; end
functions -Dv zf | tail -n +2
functions -Dv nosuch
functions -d new zf
functions zf | tail -n +2
functions -d 'two words' zf
functions zf | tail -n +2
function h -S; end
functions -Dv h | tail -n +2
functions -e zf
functions -Dv zf
set -g gg 1
set -ql gg; echo $status
function f; set -ql gg; echo in $status; set -l q 1; set -ql q; echo $status; end; f
