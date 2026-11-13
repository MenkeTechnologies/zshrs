printf '#!/bin/sh\nprintf "function hi\\n    echo edited\\nend\\n" > "$1"\n' >ed.sh
chmod +x ed.sh
printf 'function hi\n  echo orig\nend\n' >def.fish
source def.fish
hi
funced -e ./ed.sh hi </dev/null >out 2>&1
string replace (pwd) . <out
hi
cat def.fish
funced -e ./ed.sh hi </dev/null >out 2>&1
string replace (pwd) . <out
funced -e ./nonexistent hi </dev/null >out 2>&1
string replace (pwd) . <out
funced
echo st=$status
funced a b
echo st=$status
