select choice in apple banana cherry; do
  echo $choice
  break
done

coproc cat
print -p hello
read -p line
echo $line

time echo slow
time {
  for i in 1 2 3; do
    sleep 0.1
  done
}
