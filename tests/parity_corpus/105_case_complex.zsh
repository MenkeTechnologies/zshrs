# Case with glob and pipe
case $x in
  (*.zsh|*.sh) echo shell ;;
  (*.rs|*.c) echo source ;;
esac
