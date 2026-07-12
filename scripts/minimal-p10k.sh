#!/usr/bin/env bash
# minimal-p10k.sh — launch zshrs with ONLY powerlevel10k + your .p10k.zsh config.
# No zpwr, no zinit, no turbo, no other plugins. Used to reproduce/verify the
# p10k prompt in isolation.
#
# Usage:
#   scripts/minimal-p10k.sh            # interactive: opens a zshrs shell, look at the prompt
#   scripts/minimal-p10k.sh --dump     # non-interactive: dumps $PROMPT + segment count and exits
#   scripts/minimal-p10k.sh --zsh      # same but launch real zsh (baseline comparison)
#   BIN=/path/to/zshrs scripts/minimal-p10k.sh   # override the binary
set -u

# ---- locate the shell binary ----------------------------------------------
repo_root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
BIN="${BIN:-$repo_root/target/debug/zshrs}"
USE_ZSH=0
DUMP=0
for a in "$@"; do
  case "$a" in
    --zsh)  USE_ZSH=1 ;;
    --dump) DUMP=1 ;;
    *) printf 'unknown arg: %s\n' "$a" >&2; exit 2 ;;
  esac
done
if [ "$USE_ZSH" = 1 ]; then
  BIN="$(command -v zsh || echo /bin/zsh)"
  SHELL_ARGS=(-i)
else
  SHELL_ARGS=(--zsh -i)   # zshrs runs the p10k config in zsh-compat mode
fi
[ -x "$BIN" ] || { printf 'shell binary not found/executable: %s\n' "$BIN" >&2; exit 1; }

# ---- locate powerlevel10k theme + your config -----------------------------
THEME=""
for t in \
  "$HOME/.zinit/plugins/romkatv---powerlevel10k/powerlevel10k.zsh-theme" \
  "$HOME/powerlevel10k/powerlevel10k.zsh-theme" \
  "$HOME/.oh-my-zsh/custom/themes/powerlevel10k/powerlevel10k.zsh-theme"; do
  [ -r "$t" ] && THEME="$t" && break
done
[ -n "$THEME" ] || { printf 'powerlevel10k.zsh-theme not found\n' >&2; exit 1; }

CFG=""
for c in "$HOME/.zpwr/env/.p10k.zsh" "$HOME/.p10k.zsh"; do
  [ -r "$c" ] && CFG="$c" && break
done
[ -n "$CFG" ] || { printf 'no .p10k.zsh config found\n' >&2; exit 1; }

# ---- build a throwaway ZDOTDIR with a minimal .zshrc ----------------------
ZDIR="$(mktemp -d "${TMPDIR:-/tmp}/minp10k.XXXXXX")"
trap 'rm -rf "$ZDIR"' EXIT

{
  printf 'source %q\n' "$THEME"
  printf '[[ -r %q ]] && source %q\n' "$CFG" "$CFG"
  if [ "$DUMP" = 1 ]; then
    # after the first prompt is set up, print the prompt state and exit
    printf 'autoload -Uz add-zsh-hook\n'
    printf '_dump_and_exit() {\n'
    # split PROMPT on each "P9K_CONTENT::=" marker; (#pieces - 1) = segments built
    printf '  local -a _pieces=("${(@ps:P9K_CONTENT\\x3a\\x3a=:)PROMPT}")\n'
    printf '  local -i _segs=$(( $#_pieces - 1 ))\n'
    printf '  (( _segs < 0 )) && _segs=0\n'
    printf '  print -r -- "SEGMENTS_BUILT=$_segs  PROMPT_LEN=${#PROMPT}"\n'
    printf '  if (( _segs > 0 )); then print -r -- "RESULT: OK (full prompt)"; else print -r -- "RESULT: EMPTY (0 segments — the p10k bug)"; fi\n'
    printf '  exit 0\n'
    printf '}\n'
    printf 'add-zsh-hook precmd _dump_and_exit\n'
  fi
} > "$ZDIR/.zshrc"

printf '=== minimal p10k ===\n' >&2
printf 'shell : %s %s\n' "$BIN" "${SHELL_ARGS[*]}" >&2
printf 'theme : %s\n' "$THEME" >&2
printf 'config: %s\n' "$CFG" >&2
printf 'zdotdir (temp): %s\n\n' "$ZDIR" >&2

ZDOTDIR="$ZDIR" HOME="$HOME" exec "$BIN" "${SHELL_ARGS[@]}"
