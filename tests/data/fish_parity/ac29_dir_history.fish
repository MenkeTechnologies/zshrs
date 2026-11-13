cd /usr; cd /etc; cd /usr; cd /usr; cd /tmp
cd -; echo $PWD
prevd; echo $PWD
nextd; echo $PWD
nextd; echo $PWD
cd /; pushd /usr; pushd /etc; echo $dirstack
popd; echo $PWD $dirstack
popd; echo $PWD
echo (count $dirprev)
