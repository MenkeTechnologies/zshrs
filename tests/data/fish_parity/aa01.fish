echo (count $PATH) | string match -qr "^[1-9]"; echo $status
set -l v PATH; echo (count $$v) | string match -qr "^[1-9]"; echo $status
set -l w HOME; echo $$w | string length -q; echo $status
set -gx FOO a b; set -l n FOO; echo (count $$n)
set -l path_like /x; set -l n2 path_like; echo $$n2
