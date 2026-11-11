set -x FOO bar
env | grep "^FOO="
set -gx LIST a b c
env | grep "^LIST="
set -e FOO
env | grep -c "^FOO="
