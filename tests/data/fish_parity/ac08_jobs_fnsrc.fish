jobs; echo s=$status
sleep 5 &
sleep 6 &
jobs | string replace -r '\d{3,}' PID
jobs -p | string replace -r '\d{3,}' PID
jobs -c
jobs -l | string replace -r '\d{3,}' PID
jobs -q; echo s=$status
kill %1 %2 2>/dev/null
function t; echo x; end
functions t
function u; argparse 'v/verbose+' -- $argv; echo after; end; u 2>/dev/null; echo s=$status
