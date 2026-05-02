# RECORDER.md surface row: `function` includes the `autoload NAME`
# form. Each autoload registration emits a function event with
# value="autoload" so query-side can distinguish autoload registrations
# from inline `name() {}` definitions.
autoload _git
autoload -U add-zsh-hook
autoload -Uz compinit
autoload first second third
