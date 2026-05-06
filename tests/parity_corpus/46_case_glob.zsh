case $filename in
  *.zsh|*.sh) echo "shell script" ;;
  *.rs) echo "rust source" ;;
  *) echo "unknown" ;;
esac
