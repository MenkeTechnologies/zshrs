functions -Dv isatty
functions -Dv prevd
functions -D umask
type isatty
type prevd | head -3
isatty stdout; echo $status
isatty 99; echo $status
umask 022; umask
functions -d isatty 2>/dev/null; echo $status
