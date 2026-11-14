# string reads stdin only when its own stdin is redirected (exec.rs stdin_is_directly_redirected)
printf 'q\n' > in.txt
function f; string upper; echo $status; end
echo abc | f
echo abc | string upper; echo $status
echo a | begin; string upper; echo $status; end
begin; string upper; echo $status; end < in.txt
string upper < in.txt; echo $status
string upper z < in.txt; echo $status
echo a | string split , x; echo $status
echo x | string repeat 2; echo $status
echo x | string repeat -n 2 y; echo $status
printf 'a\nb\n' | string join ,; echo $status
true | string collect -a; echo $status
string escape; echo $status
command rm -f in.txt
