# trap 'cmd' SIG1 SIG2 — N records, one per signal, all sharing handler.
trap 'echo bye' EXIT
trap 'echo int' INT TERM
trap 'echo usr' USR1
