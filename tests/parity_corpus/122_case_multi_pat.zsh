# Multiple patterns in case
case $x in
  a|b|c) echo matched ;;
  d|e) echo matched ;;
  *) echo other ;;
esac
