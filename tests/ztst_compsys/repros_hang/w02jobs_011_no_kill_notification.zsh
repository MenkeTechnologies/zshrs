# Minimal hanging input for W02jobs#11 "various `kill` signals with multiple
# running jobs", the assertion where the whole file stops (15 later assertions
# never run).
#
#   <harness-zsh> -f w02jobs_011_no_kill_notification.zsh <shell-under-test>
#
# zsh prints "notice: [1]  + terminated  sleep 30" and exits 0.
# zshrs never prints it, so the harness blocks forever in `zpty -r`.
#
# A BLOCKED READ, and the block is in the harness while the cause is in the
# shell under test: an interactive zshrs in a pty starts the job and reports
# "[1] <pid>", but emits no job-status notification when the job is killed.
# zsh/zpty is loaded by the harness here, never by the shell under test, so
# this measures job notification and nothing else.
#
emulate -R zsh
[[ -d Modules/zsh ]] && module_path=( $PWD/Modules )
zmodload zsh/zpty || exit 1
export PS1= PS2=
zpty zsh "$1 -fiV +Z"
zpty -w zsh 'sleep 30 &' $'\n'
zpty -r zsh REPLY; print -r -- "job-start: ${REPLY%%[$'\r\n']*}"
zpty -w zsh 'kill %1' $'\n'
print -u2 "waiting for the termination notice ..."
zpty -r zsh REPLY; print -r -- "notice: ${REPLY%%[$'\r\n']*}"
zpty -d
