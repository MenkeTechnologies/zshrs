echo a \
  b \
  c \
  d
x=$( echo "long" \
       "string" \
       "split" )
[[ -f /tmp/x \
   && -r /tmp/x ]]
for i in 1 2 3 \
         4 5 6; do
  echo $i
done
