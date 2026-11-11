set -x X1 val
sh -c "echo \$X1"
set -gx X2 a b
sh -c "echo \$X2"
X3=pre sh -c "echo \$X3"
env -u X1 sh -c "echo [\$X1]"
set -e X1
sh -c "echo [\$X1]"
