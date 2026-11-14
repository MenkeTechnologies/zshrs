status is-block; echo $status
begin; status is-block; echo $status; end
function f; status is-block; echo $status; end; f
if true; status is-block; echo $status; end
for i in 1; status is-block; echo $status; end
status job-control full; echo $status
status features | string length | head -2
status test-feature qmark-noglob; echo $status
status test-feature test-require-arg; echo $status
status test-feature nosuch; echo $status
status nosuch 2>/dev/null; echo $status
