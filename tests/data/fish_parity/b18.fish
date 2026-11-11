set -g count 0
function inc; set count (math $count + 1); end
inc; inc; inc
echo $count
