# string replace -r -a follows pcre2_substitute's global loop: an empty match at the end
# of the subject counts, an empty match right after a non-empty one counts, and after an
# empty match the next search is anchored there and must not be empty.
string replace -ra '' L one
string replace -ra '' 0 'a b' c
string replace -ra 'x*' - xa
string replace -ra 'x*' - axxb
string replace -ra '\b' '|' 'ab cd'
string replace -ra '' . héllo
string replace -ra '(?=b)|b' X abc
string replace -ra 'a|' X baac
string replace -ra '^' X abc
string replace -ra 'a\K' X aab
string replace -r '' L ''
string replace -r '$' L ab
string replace -fra '' X a b
echo $status
string replace -m1 -ra '' X a b
echo $status
string replace -qra '' X a
echo $status
