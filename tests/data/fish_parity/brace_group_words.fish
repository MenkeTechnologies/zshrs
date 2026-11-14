# A brace group holds blanks and separators inside its word, as fish's tokenizer reads
# it. A group with no `,` and no variable directly in it is literal, spaces and all;
# in a live group the spaces at each end of an alternative are trimmed and those inside
# one kept; `\,` is a literal comma; a substitution's output is one alternative per line.
function show
    printf '[%s]' $argv
    echo
end
show {a, b} { a , b }x {a,  b,c } {a ,b}
show {a b,c}d {a b} {} a{}b {a} x{a}y {  } { }x
show {a\,b} {a\,b,c} {a,b\,c} { a\ ,b}
show {{a,b}} {{ a,b}} {{ a,b},c} x{ a,b} {a,{b}}
show {,a} {a,} {,} x{,}y
show {a;b,c} {a|b,c} {a>b,c} {a&b,c} {a#b,c} {a;b}
show {a,'b c'} {a, "b c" } {a,"b,c"} {'a,b'}
show {(echo 1 2),z} {(echo p; echo q),z} {a, (echo x)} {(echo a,b)} {(echo x),}
show a[x y] a[x;y] a[x|y] x{a,b}[1 2] a[x {y,z}] {a,[b c]}
set x a b
show {$x} {$x,z}{1,2} {a"$x"} {"$x",z} {$x[2]}
{ echo block}
{echo block2}
