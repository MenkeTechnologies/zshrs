echo abc | not string match -q "*z*"; echo $status
echo abc | not string match -q "*a*"; echo $status
echo abc | string match -q "*a*" | not cat; echo $status
