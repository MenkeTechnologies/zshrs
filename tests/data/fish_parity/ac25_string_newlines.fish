function show; printf '%-26s' $argv[1]; od -An -c | string trim | string join ' ' | string collect -N; echo ']'; end
function p1; printf a; end
function p2; printf 'a\nb'; end
function p3; printf 'a\n'; end
function p0; printf ''; end
function pnl; printf '\n'; end
for spec in "upper" "lower" "trim" "escape" "unescape" "collect" "replace a x" "replace -r a x" "match a" "match -r a" "match -v z" "sub -s 1" "pad -w 3" "join ," "join0" "repeat -n2" "length" "shorten -m 5" "split ," "split0" "match -e a" "match -n a"
    for p in p0 pnl p1 p2 p3
        $p | string (string split ' ' -- $spec) | show "$spec < $p"
    end
end
printf 'a\n' | string collect -N > c1
printf 'a\n\n' | string collect -N > c2
printf 'a\n\n' | string collect > c3
printf 'a' | string collect -N > c4
printf 'a' | string collect > c5
printf 'a\nb\n' | string collect > c6
printf '' | string collect -N > c7
printf 'a\nb\n' | string collect -N > c8
printf '\n' | string collect > c9
od -c c1 | head -3
od -c c2 | head -3
od -c c3 | head -3
od -c c4 | head -3
od -c c5 | head -3
od -c c6 | head -3
od -c c7 | head -3
od -c c8 | head -3
od -c c9 | head -3
