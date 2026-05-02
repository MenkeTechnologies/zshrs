arr=(a b c d e)
echo ${arr[1]}
echo ${arr[-1]}
echo ${arr[2,4]}
echo ${arr[@]}
echo ${#arr[@]}
echo ${arr[1,3]}
typeset -A hash
hash[key]=value
echo ${hash[key]}
echo ${(k)hash}
echo ${(v)hash}
echo ${(kv)hash}
arr+=(f g)
echo $arr[(r)c]
