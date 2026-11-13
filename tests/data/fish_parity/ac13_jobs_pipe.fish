jobs; echo s=$status
jobs -p; echo s=$status
sleep 5 &
sleep 6 &
jobs | string replace -r '\d{3,}' PID
jobs -p | string replace -r '\d{3,}' PID
jobs -g | string replace -r '\d{3,}' PID
jobs -c | string replace -r '\d{3,}' PID
jobs -l | string replace -r '\d{3,}' PID
jobs 1 | string replace -r '\d{3,}' PID; echo s=$status
jobs 9; echo s=$status
jobs -q; echo s=$status
kill %1 %2 2>/dev/null
