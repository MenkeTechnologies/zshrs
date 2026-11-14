# status 1 when a subcommand changed / matched / produced nothing (builtins/string/*.rs)
string lower -- abc 'a b'; echo $status
string lower -- aBc; echo $status
string upper -q abc; echo $status
string escape; echo $status
string escape ''; echo $status
string unescape 'a\\'; echo $status
string unescape --style=url %zz %4; echo $status
string unescape --style=url a%20b; echo $status
string collect '' ''; echo $status
string collect -a; echo $status
string shorten -m 0 abcdef; echo $status
string shorten abcdef abc; echo $status
string shorten -m 1 -c xyz abcdef; echo $status
string pad -C -w 6 ab; echo $status
string pad -C -r -w 7 ab; echo $status
set -l e
string join , $e; echo $status
string length $e; echo $status
