#!/usr/bin/env zshrs
# Periodic functions — $PERIOD + periodic() callback.
# Ports Src/builtin.c periodic + Src/init.c periodic_sched_cmd.

echo "── periodic() function setup ──"
typeset -gi PERIOD=2
typeset -gi tick_count=0

periodic() {
    (( tick_count++ ))
    echo "  [periodic tick #$tick_count]  PERIOD=$PERIOD"
}

echo "  PERIOD set to $PERIOD seconds"
echo "  periodic() callback registered"

echo
echo "── manual invocation ──"
periodic
periodic
periodic
echo "  ticks fired so far: $tick_count"

echo
echo "── $PERIOD parameter mechanics ──"
echo "  current PERIOD: $PERIOD"
PERIOD=5
echo "  changed to:     $PERIOD"
PERIOD=0
echo "  disabled (0):   $PERIOD"

echo
echo "── add-zsh-hook for periodic ──"
autoload -Uz add-zsh-hook 2>/dev/null

monitor1() { echo "  monitor1 fired @ tick $tick_count"; }
monitor2() { echo "  monitor2 fired @ tick $tick_count"; }

if (( ${+functions[add-zsh-hook]} )); then
    add-zsh-hook periodic monitor1 2>/dev/null
    add-zsh-hook periodic monitor2 2>/dev/null
    echo "  registered monitor1, monitor2"
fi

echo
echo "── show $periodic_functions array ──"
echo "  registered: ${periodic_functions[@]}"

echo
echo "── chpwd & related hooks ──"
# chpwd_functions, precmd_functions, preexec_functions — all assoc arrays.
typeset -a chpwd_functions
chpwd_functions=(chpwd_log chpwd_stat)

chpwd_log() { echo "  chpwd: cd to $PWD"; }
chpwd_stat() { echo "  chpwd: $(ls $PWD | wc -l) files visible"; }

echo "  chpwd_functions: ${chpwd_functions[@]}"

# Simulate cd by manually firing.
echo
echo "── simulated cd to /tmp ──"
for fn in "${chpwd_functions[@]}"; do
    if (( ${+functions[$fn]} )); then
        $fn
    fi
done

echo
echo "── precmd_functions sequence ──"
typeset -a precmd_functions
precmd_functions=(precmd_jobs precmd_stat precmd_log)

precmd_jobs() { echo "  precmd: jobs=0"; }
precmd_stat() { echo "  precmd: hist=42 lines"; }
precmd_log() { echo "  precmd: ts=$EPOCHSECONDS"; }

zmodload zsh/datetime 2>/dev/null

echo "  precmd hooks: ${precmd_functions[@]}"
echo "  firing all:"
for fn in "${precmd_functions[@]}"; do
    if (( ${+functions[$fn]} )); then
        $fn
    fi
done

echo
echo "── preexec_functions chain ──"
typeset -a preexec_functions
preexec_functions=(preexec_audit)

preexec_audit() {
    local cmd=$1
    echo "  preexec: about to run [$cmd]"
}

if (( ${+functions[preexec_audit]} )); then
    preexec_audit "ls -la /tmp"
fi

echo
echo "── unregister ──"
unfunction periodic monitor1 monitor2 chpwd_log chpwd_stat 2>/dev/null
unfunction precmd_jobs precmd_stat precmd_log preexec_audit 2>/dev/null
echo "  all hook functions unfunc'd"
echo "  total ticks fired this run: $tick_count"
