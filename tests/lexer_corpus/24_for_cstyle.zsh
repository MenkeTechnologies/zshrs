for (( i = 0; i < 10; i++ )); do
  echo $i
done

for (( ; ; )); do
  break
done

for (( i = 1, j = 100; i <= 10; i++, j-- )); do
  echo $i $j
done
