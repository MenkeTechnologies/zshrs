case $x in
  a) echo a ;;
  b) echo b ;;
esac
select x; do
  echo $x
  break
done
