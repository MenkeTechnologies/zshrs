# Multiple if/elif/else
if [[ -f a ]]; then
  echo a
elif [[ -f b ]]; then
  echo b
elif [[ -f c ]]; then
  echo c
else
  echo d
fi
