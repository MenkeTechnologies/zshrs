# names ending in PATH (and LANGUAGE) are path variables: split on `:` when set,
# joined with `:` in a string and in the environment; --path / --unpath override
set -x ZQ_PATH /a:/b; echo 1 (count $ZQ_PATH) $ZQ_PATH[2]; echo "1q $ZQ_PATH"; env | string match 'ZQ_PATH=*'
set ZQ2PATH /a:/b /c; echo 2 (count $ZQ2PATH) "$ZQ2PATH"
set -l ZQLPATH x:y; echo 3 (count $ZQLPATH) "$ZQLPATH"
set zqfoo /a:/b; echo 4 (count $zqfoo) "$zqfoo"
set -a ZQ_PATH x:y; echo 5 (count $ZQ_PATH) "$ZQ_PATH"
set -p ZQ_PATH p:q; echo 6 (count $ZQ_PATH) "$ZQ_PATH"
set ZQ_PATH[2] m:n; echo 7 (count $ZQ_PATH) "$ZQ_PATH" $ZQ_PATH[3]
set ZQ3PATH '' ''; echo 8 (count $ZQ3PATH) "[$ZQ3PATH]"
set ZQ3PATH a::b; echo 9 (count $ZQ3PATH) "[$ZQ3PATH]"
set ZQ3PATH ''; echo 10 (count $ZQ3PATH)
set LANGUAGE en:de; echo 11 (count $LANGUAGE) "$LANGUAGE"
set zqpath a:b; echo 12 (count $zqpath)
set -S ZQ2PATH
set --path zqp /a:/b /c; echo 13 (count $zqp) "$zqp"
set zqp $zqp; echo 14 (count $zqp) "$zqp"
set zqp a:b; echo 15 (count $zqp)
set --unpath ZQ4PATH /a:/b; echo 16 (count $ZQ4PATH) "$ZQ4PATH"
set ZQ4PATH /q:/r; echo 17 (count $ZQ4PATH)
set --path ZQ4PATH /q:/r; echo 18 (count $ZQ4PATH) "$ZQ4PATH"
set -x zqx a b; env | string match 'zqx=*'; set --path zqx $zqx; env | string match 'zqx=*'
set -e zqp; set zqp a:b; echo 19 (count $zqp)
function f; set -l ZQ5PATH a:b; echo f (count $ZQ5PATH) "$ZQ5PATH"; end; f
