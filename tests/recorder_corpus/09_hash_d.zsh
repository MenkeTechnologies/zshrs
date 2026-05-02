# hash -d — named directories. Plain hash NAME=PATH is command-hash
# (runtime cache, not config) and should NOT produce a record.
hash -d zpwr=/tmp/zpwr
hash -d proj=/tmp/proj
hash -d notes=/tmp/notes
hash some_cmd=/usr/bin/true
