echo foo | string match -r "f(o+)"
echo bar | string replace -r "(b)(a)(r)" "\$3\$2\$1"
echo "2024-01-15" | string match -rg "(\\d+)-(\\d+)-(\\d+)"
string match -a "a*" apple avocado banana
