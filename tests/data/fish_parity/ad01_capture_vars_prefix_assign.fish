# string match -r sets a variable per named group: from the first matching
# argument, every match of it with --all, nothing when no argument matched
string match -rq -- '(?<name>.+)=(?<value>.*)' -- --help=
echo "[$name][$value]"
string match -ra '(?<d>\d)(?<l>[a-z])?' '1a2 3c'
echo "[$d][$l]" (count $l)
set -l kept old
string match -rq '(?<kept>x)' zzz
echo (count $kept)
# a group that took no part prints nothing; -n gives each group's index
string match -r 'a(b)?' ac
string match -rn 'a(b)' xab
string match -rnv a xb
# -e in glob mode prints the whole argument
string match -e b abc
string match -e 'a*' abc xabc

# NAME=value cmd: the value is a list and the command's own words see it
mkdir -p ad01_dir
touch ad01_dir/one ad01_dir/two
files=ad01_dir/* string join , $files
X=a sh -c 'echo $X'
echo "after:$X"
function ad01_show
    echo $V
end
V=hi ad01_show
rm -r ad01_dir

# a path variable is listed by its elements
set -gx PATH /usr/bin /bin
set -x | string match 'PATH *'
