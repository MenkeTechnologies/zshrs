case $x in
  ([a-z]*) echo alpha ;;
  ([0-9]*) echo numeric ;;
  (?) echo single ;;
  (*) echo multi ;;
esac
