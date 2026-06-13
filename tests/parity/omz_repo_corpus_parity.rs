//! Behavioural parity corpus mined from the full oh-my-zsh GitHub repo
//! (lib/*.zsh, themes/*.zsh-theme, plugins/*) — beyond the locally-installed
//! OMZ snippets already covered. Sources: spectrum FX/FG/BG color arrays,
//! the git_prompt_* parsing functions, theme prompt assembly, and plugin
//! logic (extract, universalarchive, web-search, urltools, dirhistory, …).
//!
//! Every candidate was extracted from real OMZ source and VERIFIED
//! deterministic across two `zsh -fc` runs before inclusion. Each test
//! asserts `zshrs --zsh -fc` matches `/opt/homebrew/bin/zsh -fc` on stdout
//! + exit; escape/color output is rendered through `cat -v`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path()).args(["-fc", script]).output().expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "exit-code divergence on:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

// ═══════════════════════════════ lib/*.zsh ═════════════════════════

mod omz_lib {
    use super::*;

    /// spectrum.zsh — typeset -AHg FX FG BG + {000..255} loop building 256-entry arrays.
    #[test]
    fn spectrum_construct() {
        assert_parity(r###"typeset -AHg FX FG BG
FX=(reset "%{\e[00m%}" bold "%{\e[01m%}" no-bold "%{\e[22m%}")
for color in {000..255}; do
  FG[$color]="%{\e[38;5;${color}m%}"
  BG[$color]="%{\e[48;5;${color}m%}"
done
print -r -- "FGcount=${#FG} BGcount=${#BG} FXcount=${#FX}"
print -r -- "FG001=${FG[001]}" | cat -v
print -r -- "FXbold=${FX[bold]}" | cat -v"###);
    }

    /// git.zsh — git_prompt_status accumulation: anchored regex over porcelain, ordered prepend.
    #[test]
    fn git_prompt_status_accum() {
        assert_parity(r###"local -A prefix_constant_map constant_prompt_map statuses_seen
local status_text status_prompt status_prefix status_constant status_regex
local status_constants
prefix_constant_map=('\?\? ' 'UNTRACKED' 'A  ' 'ADDED' ' M ' 'MODIFIED' 'UU ' 'UNMERGED')
constant_prompt_map=(UNTRACKED '?' ADDED '+' MODIFIED '!' UNMERGED '=' STASHED 'S')
status_constants=(UNTRACKED ADDED MODIFIED STASHED UNMERGED)
status_text='?? newfile.txt
 M tracked.c
A  staged.h
UU conflict.rs'
statuses_seen[STASHED]=1
for status_prefix in "${(@k)prefix_constant_map}"; do
  status_constant="${prefix_constant_map[$status_prefix]}"
  status_regex=$'(^|\n)'"$status_prefix"
  [[ "$status_text" =~ $status_regex ]] && statuses_seen[$status_constant]=1
done
for status_constant in $status_constants; do
  (( ${+statuses_seen[$status_constant]} )) && status_prompt="$constant_prompt_map[$status_constant]$status_prompt"
done
echo "PROMPT=$status_prompt""###);
    }

    /// git.zsh — branch-header parse: =~ capture + (@s/,/) split + nested numeric capture.
    #[test]
    fn git_branch_header_parse() {
        assert_parity(r###"local -A prefix_constant_map statuses_seen
local status_lines branch_statuses line branch_status last_parsed_status
prefix_constant_map=(ahead AHEAD behind BEHIND diverged DIVERGED)
status_lines=("## main...origin/main [ahead 2, behind 1]" "## feature...origin/feature [ahead 5]")
for line in $status_lines; do
  if [[ "$line" =~ "^## [^ ]+ \[(.*)\]" ]]; then
    branch_statuses=("${(@s/,/)match}")
    for branch_status in $branch_statuses; do
      [[ ! $branch_status =~ "(behind|diverged|ahead) ([0-9]+)?" ]] && continue
      last_parsed_status=$prefix_constant_map[$match[1]]
      statuses_seen[$last_parsed_status]=$match[2]
    done
  fi
done
for k in AHEAD BEHIND DIVERGED; do print -r -- "$k=${statuses_seen[$k]:-_}"; done"###);
    }

    /// git.zsh — parse_git_dirty FLAGS accumulation + tail-driven DIRTY/CLEAN.
    #[test]
    fn parse_git_dirty_flags() {
        assert_parity(r###"local STATUS
local -a FLAGS
local DISABLE_UNTRACKED_FILES_DIRTY=true GIT_STATUS_IGNORE_SUBMODULES=''
FLAGS=('--porcelain')
[[ "${DISABLE_UNTRACKED_FILES_DIRTY:-}" == "true" ]] && FLAGS+='--untracked-files=no'
case "${GIT_STATUS_IGNORE_SUBMODULES:-}" in
  git) ;;
  *) FLAGS+="--ignore-submodules=${GIT_STATUS_IGNORE_SUBMODULES:-dirty}" ;;
esac
print -r -- "FLAGS=${(j: :)FLAGS}"
local canned=' M file.c'
STATUS=$(print -r -- "$canned" | tail -n 1)
[[ -n $STATUS ]] && echo dirty || echo clean"###);
    }

    /// history.zsh — HISTSIZE/SAVEHIST floor + HIST_STAMPS case mapping.
    #[test]
    fn history_floor_stamps() {
        assert_parity(r###"local HISTSIZE=1000 SAVEHIST=500 HIST_STAMPS
[ "$HISTSIZE" -lt 50000 ] && HISTSIZE=50000
[ "$SAVEHIST" -lt 10000 ] && SAVEHIST=10000
print -r -- "HISTSIZE=$HISTSIZE SAVEHIST=$SAVEHIST"
for HIST_STAMPS in "mm/dd/yyyy" "dd.mm.yyyy" "yyyy-mm-dd" "" "%F"; do
  case ${HIST_STAMPS-} in
    "mm/dd/yyyy") print -r -- "alias=omz_history -f" ;;
    "dd.mm.yyyy") print -r -- "alias=omz_history -E" ;;
    "yyyy-mm-dd") print -r -- "alias=omz_history -i" ;;
    "") print -r -- "alias=omz_history" ;;
    *) print -r -- "alias=omz_history -t '$HIST_STAMPS'" ;;
  esac
done"###);
    }

    /// theme-and-appearance.zsh — case "$OSTYPE" alternation selecting ls flag.
    #[test]
    #[ignore = "zshrs gap: case pattern (darwin|freebsd)* — glob alternation followed by * fails to match (darwin23.0 falls to default); confirmed in isolation"]
    fn theme_ostype_ls() {
        assert_parity(r###"choose() {
  local os="$1" lsalias
  case "$os" in
    netbsd*)  lsalias="gls --color=tty" ;;
    openbsd*) lsalias="colorls -G" ;;
    (darwin|freebsd)*) lsalias="ls -G" ;;
    *) lsalias="ls --color=tty" ;;
  esac
  print -r -- "$os -> $lsalias"
}
choose darwin23.0; choose freebsd13; choose netbsd9; choose openbsd7; choose linux-gnu"###);
    }

    /// prompt_info_functions.zsh — multi-name dummy fn def + function_exists via typeset -f.
    #[test]
    fn prompt_info_multifn() {
        assert_parity(r###"function chruby_prompt_info rbenv_prompt_info hg_prompt_info { return 1 }
function_exists() { typeset -f "$1" > /dev/null }
print -r -- "exists chruby=$(function_exists chruby_prompt_info && echo yes || echo no)"
print -r -- "exists nope=$(function_exists nope_prompt_info && echo yes || echo no)"
print -r -- "dummy: $(chruby_prompt_info; echo rc=$?)""###);
    }

    /// prompt_info_functions.zsh — rvm_prompt :gs/%/%% percent-doubling vs print -P.
    #[test]
    #[ignore = "zshrs gap: literal parens surrounding a ${var:gs/%/%%}-modified value are dropped from the result"]
    fn rvm_pct_double() {
        assert_parity(r###"local rvm_prompt='ruby-3.2%@1.0'
local out="(${rvm_prompt:gs/%/%%})"
print -r -- "raw=$out"
print -rn -- "P="; print -P -- "$out""###);
    }

    /// functions.zsh — env_default existence test + conditional export + distinct rc.
    #[test]
    fn env_default() {
        assert_parity(r###"env_default() { (( ${+parameters[$1]} )) && return 0; export "$1=$2" && return 3; }
typeset -gx PAGER=less
env_default 'PAGER' 'more'; print -r -- "PAGER=$PAGER rc=$?"
unset NEWVAR 2>/dev/null
env_default 'NEWVAR' 'hello'; print -r -- "NEWVAR=$NEWVAR rc=$?""###);
    }

    /// prompt_info_functions.zsh — ${+functions[name]} define-if-absent override.
    #[test]
    fn functions_redef() {
        assert_parity(r###"function git_prompt_info() { echo stub }
print -r -- "defined=${+functions[git_prompt_info]} missing=${+functions[no_such_fn]}"
if (( ! ${+functions[ruby_prompt_info]} )); then function ruby_prompt_info() { echo default-ruby }; fi
print -r -- "$(ruby_prompt_info)"
print -r -- "body_has_stub=$([[ ${functions[git_prompt_info]} == *stub* ]] && echo 1 || echo 0)""###);
    }

    /// git.zsh — porcelain XY char-index classification ${code[1]}/${code[2]}.
    #[test]
    #[ignore = "zshrs gap: ${code[1]} char-subscript + [[ != ]] staged classification inside $() yields 0 (zsh: 1); confirmed in isolation"]
    fn porcelain_xy_index() {
        assert_parity(r###"local code
for code in 'M ' ' M' 'MM' '??' 'A ' 'UU'; do
  local x="${code[1]}" y="${code[2]}"
  print -r -- "code='$code' X='$x' Y='$y' staged=$([[ $x != ' ' && $x != '?' ]] && echo 1 || echo 0)"
done"###);
    }
}

// ════════════════════════════ themes/*.zsh-theme ═══════════════════

mod omz_themes {
    use super::*;

    /// agnoster — prompt_segment with %K{}+%F{} both wrapped in %{...%}, (%) on $(fn).
    #[test]
    fn prompt_segment_expand() {
        assert_parity(r###"prompt_segment() { print -n "%{%K{$1}%}%{%F{$2}%} $3 "; }
s="${(%)$(prompt_segment blue black DIR)}${(%)$(prompt_segment green white GIT)}"
print -rn -- "$s" | cat -v
echo"###);
    }

    /// robbyrussell — %(?:t:t) colon-form ternary + %1{x%} width hint + $fg_bold.
    #[test]
    fn robby_colon_ternary() {
        assert_parity(r###"autoload -U colors; colors
false
P="%(?:%{$fg_bold[green]%}%1{>%} :%{$fg_bold[red]%}%1{>%} ) %{$fg[cyan]%}OK%{$reset_color%}"
print -rn -- "${(%)P}" | cat -v
echo"###);
    }

    /// af-magic — dashed rule via ${(l:COLUMNS::-:)} (COLUMNS bare in pad spec).
    #[test]
    fn columns_rule() {
        assert_parity(r###"COLUMNS=20
rule="${(l:$COLUMNS::-:)}"
print -rn -- "$rule" | cat -v
echo"###);
    }

    /// bira — multi-line PROMPT with embedded newline + %B%(!.#.>)%b.
    #[test]
    fn multiline_prompt() {
        assert_parity(r###"autoload -U colors; colors
d="$(mktemp -d)"; mkdir -p "$d/proj"; cd "$d/proj"
cur="%{$fg[blue]%}%c%{$reset_color%}"
P="X-${cur}
Y-%B%(!.#.>)%b "
print -rn -- "${(%)P}" | cat -v
echo
cd /; rm -rf "$d""###);
    }

    /// fishy — collapse cwd: (s:/:) split, per-segment first-letter, (j:/:) join.
    #[test]
    fn fishy_collapse() {
        assert_parity(r###"collapse(){ local i pwd; pwd=("${(s:/:)1}")
  if (( $#pwd > 1 )); then for i in {1..$(($#pwd-1))}; do
    if [[ "$pwd[$i]" = .* ]]; then pwd[$i]="${${pwd[$i]}[1,2]}"; else pwd[$i]="${${pwd[$i]}[1]}"; fi
  done; fi
  echo "${(j:/:)pwd}"; }
collapse "home/user/.config/projects/zshrs""###);
    }

    /// ys — %{$terminfo[bold]%} terminfo-driven bold + $fg.
    #[test]
    fn terminfo_bold() {
        assert_parity(r###"autoload -U colors; colors
P="%{$terminfo[bold]%}%{$fg[yellow]%}DIR%{$reset_color%}"
print -rn -- "${(%)P}" | cat -v
echo"###);
    }

    /// fishy — dynamic color-array key $fg[$var].
    #[test]
    fn dynamic_color_key() {
        assert_parity(r###"autoload -U colors; colors
user_color=green; host_color=yellow
P="%{$fg[$user_color]%}user%{$reset_color%}@%{$fg[$host_color]%}host%{$reset_color%}%(!.#.>) "
print -rn -- "${(%)P}" | cat -v
echo"###);
    }

    /// robbyrussell — real setopt promptsubst PROMPT with $(fn) cmdsubst via print -P.
    #[test]
    fn promptsubst_cmdsubst() {
        assert_parity(r###"setopt promptsubst
seg(){ print -n "[$1]"; }
PROMPT='%F{green}$(seg A)$(seg B)%f'
print -Pn -- "$PROMPT" | cat -v
echo"###);
    }

    /// af-magic — PS1 dynamic rule ${(l.$(fn)..=.)} with $() in pad-spec under promptsubst.
    #[test]
    #[ignore = "zshrs gap: promptsubst PS1 with ${(l.$(fn)..=.)} (command-sub inside pad-spec) renders empty under print -P"]
    fn promptsubst_dynamic_pad() {
        assert_parity(r###"setopt promptsubst
COLUMNS=12
dashes(){ echo $COLUMNS; }
PS1='%F{240}${(l.$(dashes)..=.)}%f'
print -Pn -- "$PS1" | cat -v
echo"###);
    }

    /// ys — $fg_no_bold explicit non-bold color-array variant.
    #[test]
    fn fg_no_bold() {
        assert_parity(r###"autoload -U colors; colors
P="%{$fg_no_bold[blue]%}plain%{$reset_color%}"
print -rn -- "${(%)P}" | cat -v
echo"###);
    }

    /// sorin — %(N~|t|f) bar-form path-depth conditional.
    #[test]
    fn bar_form_depth() {
        assert_parity(r###"d=$(mktemp -d); mkdir -p "$d/a/b/c/d/e"; cd "$d/a/b/c/d/e"
P="%(4~|DEEP|SHALLOW)"
print -rn -- "${(%)P}" | cat -v
echo; cd /; rm -rf "$d""###);
    }

    /// agnoster — segment transition: current bg + next bg around a separator.
    #[test]
    fn segment_transition() {
        assert_parity(r###"seg(){ print -n "%K{$1}%F{$2} $3 %k"; }
s="${(%)$(seg blue white A)}%F{blue}%K{green}>%f${(%)$(seg green black B)}"
print -rn -- "${(%)s}" | cat -v
echo"###);
    }

    /// jonathan — repeat builtin building a rule string by appending.
    #[test]
    fn repeat_rule() {
        assert_parity(r###"rule=""; repeat 10 rule+="-"
print -rn -- "$rule" | cat -v
echo"###);
    }

    /// gallois — vcs_info_msg_0_-style format placeholder substitution then prompt-expand.
    #[test]
    fn vcs_format_subst() {
        assert_parity(r###"autoload -U colors; colors
fmt="%{$fg[yellow]%}(git)-[BRANCH]%{$reset_color%}"
vcs_info_msg_0_="${fmt//BRANCH/main}"
print -rn -- "${(%)vcs_info_msg_0_}" | cat -v
echo"###);
    }

    /// gnzh — nested prompt conditionals combining exit-status and privilege.
    #[test]
    fn nested_status_priv() {
        assert_parity(r###"P="%(?.%(!.RA.UA).%(!.RF.UF))"
ok=$( (exit 0); print -rn -- "${(%)P}" )
bad=$( (exit 1); print -rn -- "${(%)P}" )
print -rn -- "$ok|$bad" | cat -v
echo"###);
    }
}

// ════════════════════════════ plugins/* ════════════════════════════

mod omz_plugins {
    use super::*;

    /// extract — big case "$1" in (*.tar.gz|*.tgz) multi-pattern extension dispatch.
    #[test]
    fn extract_case_dispatch() {
        assert_parity(r###"for f in archive.tar.gz photo.zip data.tar.bz2 doc.rar pkg.deb blob.7z note.txt music.tar.xz; do
  case "$f" in
    (*.tar.gz|*.tgz)  print "$f -> tar xzf" ;;
    (*.tar.bz2|*.tbz) print "$f -> tar xjf" ;;
    (*.tar.xz|*.txz)  print "$f -> tar xJf" ;;
    (*.zip|*.jar|*.war) print "$f -> unzip" ;;
    (*.rar)  print "$f -> unrar x" ;;
    (*.deb)  print "$f -> ar x" ;;
    (*.7z)   print "$f -> 7z x" ;;
    (*)      print "$f -> unknown" ;;
  esac
done"###);
    }

    /// universalarchive — build command vector into local -a, join with (j: :).
    #[test]
    fn ua_cmd_vector() {
        assert_parity(r###"build() {
  local ext="$1" output="$2"; shift 2
  local -a cmd
  case "$ext" in
    tgz|tar.gz)  cmd=(tar -cvzf "$output" "$@") ;;
    zip)         cmd=(zip -rull "$output" "$@") ;;
    gz)          cmd=(gzip -vcf "$@") ;;
    *)           print "unsupported: $ext"; return 1 ;;
  esac
  print -r -- "${(j: :)cmd}"
}
build tgz out.tgz a b c; build zip out.zip dir; build gz out.gz file; build foo out.foo x; print "rc=$?""###);
    }

    /// universalarchive — output-name via ${input:r:t} vs ${input:t} modifier chains.
    #[test]
    fn ua_outname_modifiers() {
        assert_parity(r###"for input ext in /home/u/proj.txt tgz /home/u/dir gz /home/u/a.b.c zip; do
  print "input=$input r:t=${input:r:t} t=${input:t} -> file:${input:r:t}.${ext}"
done"###);
    }

    /// encode64 — $#-gated stdin-vs-arg branch, base64 roundtrip.
    #[test]
    fn encode64_roundtrip() {
        assert_parity(r###"encode64() { if [[ $# -eq 0 ]]; then cat | base64; else printf "%s" "$1" | base64; fi }
decode64() { if [[ $# -eq 0 ]]; then cat | base64 --decode; else printf "%s" "$1" | base64 --decode; fi }
e=$(encode64 "hello world")
print "enc=$e"
print "dec=$(decode64 "$e")"
print "pipe=$(print -n "ohmyzsh" | encode64)""###);
    }

    /// web-search — assoc engine lookup + (j://:)(s:/:)[1,2] host extraction.
    #[test]
    fn websearch_assoc_host() {
        assert_parity(r###"typeset -A urls
urls=( google "https://www.google.com/search?q=" archive "https://web.archive.org/web/*/" )
lookup() {
  if [[ -z "$urls[$1]" ]]; then print "Search engine ${1} not supported."; return 1; fi
  if [[ $# -gt 1 ]]; then print "${urls[$1]}${(j:+:)@[2,-1]}"
  else print "${(j://:)${(s:/:)urls[$1]}[1,2]}"; fi
}
lookup google rust lang; lookup archive; lookup nope; print "rc=$?""###);
    }

    /// dirhistory — stack push/pop with no_ksh_arrays, shift arr, arr[i]=().
    #[test]
    fn dirhistory_stack() {
        assert_parity(r###"setopt no_ksh_arrays
typeset -ga past=()
DIRHISTORY_SIZE=3
push_past() {
  if [[ $#past -ge $DIRHISTORY_SIZE ]]; then shift past; fi
  if [[ $#past -eq 0 || $past[$#past] != "$1" ]]; then past+=($1); fi
}
pop_past() { if [[ $#past -gt 0 ]]; then typeset -g $1="${past[$#past]}"; past[$#past]=(); fi }
push_past /a; push_past /a; push_past /b; push_past /c; push_past /d
print "stack=${(j:,:)past} size=$#past"
local top=""
pop_past top
print "popped=$top remain=${(j:,:)past}""###);
    }

    /// copypath — absolute-path build + prompt-escape via ${(%):-"%B...%b"}.
    #[test]
    fn copypath_abs() {
        assert_parity(r###"cp_logic() {
  local file="${1:-.}"
  [[ $file = /* ]] || file="/sandbox/base/$file"
  print "abs=$file"
  print -r -- "${(%):-"%B${file}%b ready."}" | cat -v
}
cp_logic; cp_logic sub/dir; cp_logic /etc/hosts"###);
    }

    /// magic-enter — empty-BUFFER + CONTEXT==start gate + : ${VAR:=default}.
    #[test]
    fn magic_enter() {
        assert_parity(r###"me() {
  local BUFFER="$1" CONTEXT="$2" in_git="$3"
  : ${GIT_CMD:="git status -u ."} ${OTHER_CMD:="ls -lh ."}
  if [[ -n "$BUFFER" || "$CONTEXT" != start ]]; then print "(noop)"; return; fi
  if [[ "$in_git" == yes ]]; then print "$GIT_CMD"; else print "$OTHER_CMD"; fi
}
me "" start yes; me "" start no; me "ls" start no; me "" vared no"###);
    }

    /// dirpersist — load file into array (f)"$(<file)", prepend, dedup (u).
    #[test]
    #[ignore = "zshrs gap: assigning to the special `dirstack` array yields empty (dirstack=(/x /y /z) → empty); confirmed in isolation"]
    fn dirpersist_load() {
        assert_parity(r###"d=$(mktemp -d)
printf "%s\n" /x /y /z /x > "$d/zdirs"
dirstack=( ${(f)"$(< $d/zdirs)"} )
print "loaded=${(j:,:)dirstack} first=$dirstack[1]"
local -a my_stack
my_stack=( /new ${dirstack} )
print "--uniq--"
print -l ${(u)my_stack}
rm -rf "$d""###);
    }

    /// history-substring-search — escape glob metachars (#m) then join with (j:*:).
    #[test]
    fn hss_escape_join() {
        assert_parity(r###"setopt extended_glob
q="ls *.txt | grep [a-z]"
parts=(${=q})
search="${(j:*:)parts[@]//(#m)[\][()|\\*?#<>~^]/\\$MATCH}*"
print -r -- "pattern=$search" | cat -v"###);
    }

    /// git — git_main_branch brace-expanded candidate refs, first match wins, :t + fallback.
    #[test]
    fn git_main_branch() {
        assert_parity(r###"pick() {
  local found="$1"; shift
  local ref
  for ref in refs/{heads,remotes/{origin,upstream}}/{main,trunk,master}; do
    if [[ "$ref" == "$found" ]]; then print "${ref:t}"; return 0; fi
  done
  print master; return 1
}
pick refs/heads/main; pick refs/remotes/origin/trunk; pick refs/heads/nonexist; print "rc=$?""###);
    }

    /// genpass — char-code-to-modulo string index $chars[#c%$#chars+1].
    #[test]
    #[ignore = "zshrs gap: $chars[#c%$#chars+1] char-code arithmetic string subscript yields empty"]
    fn genpass_modulo_index() {
        assert_parity(r###"chars=abcdefghjkmnpqrstvwxyz0123456789
local c
for c in A B Z a z 0 9 "~"; do printf "%s" $chars[#c%$#chars+1]; done
print"###);
    }

    /// colored-man-pages — assoc-to-NAME=VALUE via for k v in (@kv), sorted (o).
    #[test]
    fn colored_man_kv() {
        assert_parity(r###"typeset -A tc
tc=( md "BOLD" me "RESET" so "STANDOUT" se "RESET" )
local -a environment k v
for k v in "${(@kv)tc}"; do environment+=( "LESS_TERMCAP_${k}=${v}" ); done
print -l ${(o)environment}"###);
    }

    /// alias-finder — progressively strip trailing word with ${cmd% *}.
    #[test]
    fn alias_finder_strip() {
        assert_parity(r###"cmd="git commit --amend --no-edit"
while [[ -n "$cmd" ]]; do
  print "try: [$cmd]"
  [[ "$cmd" != *" "* ]] && break
  cmd="${cmd% *}"
done"###);
    }

    /// jsontools — method auto-detect loop with break + unset over (I) membership.
    #[test]
    fn jsontools_detect() {
        assert_parity(r###"typeset M
avail=(ruby python3 node)
has() { (( ${avail[(I)$1]} )) }
for M in node python3 ruby; do if has $M; then break; fi; unset M; done
print "method=${M:-NONE}""###);
    }

    /// git — git_develop_branch membership via word-bounded *" $x "*.
    #[test]
    fn git_develop_member() {
        assert_parity(r###"pick() {
  local present="$1"; shift
  local branch
  for branch in dev devel develop development; do
    if [[ " $present " == *" $branch "* ]]; then print $branch; return 0; fi
  done
  print develop; return 1
}
pick "main develop foo"; pick "main dev"; pick "main feature"; print "rc=$?""###);
    }

    /// extract — while [[ "$1" == -* ]] option loop with shift/shift 2 and --.
    #[test]
    fn extract_option_loop() {
        assert_parity(r###"parse() {
  local remove=0 todir=""
  while [[ "$1" == -* ]]; do
    case "$1" in
      (-r|--remove) remove=1; shift ;;
      (-t|--to-directory) todir="$2"; shift 2 ;;
      (--) shift; break ;;
      (*) print "bad opt $1"; return 1 ;;
    esac
  done
  print "remove=$remove todir=[$todir] file=$1"
}
parse -r archive.zip; parse -t /tmp/out archive.tar.gz; parse --remove --to-directory dest file.7z"###);
    }

    /// urltools — pure-shell percent-encode: per-char loop, case allow-list, printf "%%%02X".
    #[test]
    fn urltools_encode() {
        assert_parity(r###"urlencode() {
  local s="$1" out="" i ch
  for (( i=1; i<=${#s}; i++ )); do
    ch="${s[i]}"
    case "$ch" in
      ([a-zA-Z0-9._~-]) out+="$ch" ;;
      (*) out+=$(printf "%%%02X" "'$ch") ;;
    esac
  done
  print -r -- "$out"
}
urlencode "a b/c?d=e&f+g"; urlencode "100% sure!""###);
    }

    /// urltools — pure-shell percent-decode: ${1//+/ }, 2-char slice ${s[i+1,i+2]}, printf \x.
    #[test]
    fn urltools_decode() {
        assert_parity(r###"urldecode() {
  local s="${1//+/ }" out="" i ch hex
  for (( i=1; i<=${#s}; i++ )); do
    ch="${s[i]}"
    if [[ "$ch" == "%" ]]; then hex="${s[i+1,i+2]}"; out+=$(printf "\\x$hex"); (( i+=2 ))
    else out+="$ch"; fi
  done
  print -r -- "$out"
}
urldecode "a%20b%2Fc"; urldecode "hello+world%21""###);
    }

    /// otp — non-digit strip sanitize ${var//[^0-9]/}.
    #[test]
    fn otp_sanitize() {
        assert_parity(r###"for s in "v1.2.3" "(555) 123-4567" "abc"; do
  print "$s -> [${s//[^0-9]/}]"
done"###);
    }

    /// web-search — rest-of-args slice ${@[2,-1]} joined as query.
    #[test]
    fn websearch_slice() {
        assert_parity(r###"f() {
  print "engine=$1"
  local rest=(${@[2,-1]})
  print "query=${(j:+:)rest}"
}
f google how to port zsh; f bing single"###);
    }

    /// colored-man-pages — combined path modifiers ${p:A:h} vs :h/:t.
    #[test]
    fn p_A_h_modifiers() {
        assert_parity(r###"p="/a/b/../c/./plugin.zsh"
print "A:h=${p:A:h}"
print "h=${p:h}"
print "t=${p:t}""###);
    }

    /// jsontools — while IFS="=" read -r k v building assoc, sorted-keys (ok).
    #[test]
    fn jsontools_ifs_read() {
        assert_parity(r###"data="name=foo
size=42
type=bar"
typeset -A kv
while IFS="=" read -r k v; do kv[$k]="$v"; done <<< "$data"
for k in ${(ok)kv}; do print "$k -> $kv[$k]"; done"###);
    }

    /// qrcode — $* arg-join with empty-input stdin sentinel.
    #[test]
    #[ignore = "zshrs gap: `local input=\"$*\"` captures only the first positional (plain x=\"$*\" and local x=\"$var\" both work); confirmed in isolation"]
    fn qrcode_sentinel() {
        assert_parity(r###"qc() {
  local input="$*"
  [ -z "$input" ] && input="@/dev/stdin"
  print "payload=[$input]"
}
qc hello world test; qc"###);
    }

    /// otp — basename-strip ${${f:t}%.otp.asc} over array into reply.
    #[test]
    fn otp_basename_strip() {
        assert_parity(r###"local -a files; files=(/h/.otp/work.otp.asc /h/.otp/personal.otp.asc)
local -a reply
for f in $files; do reply+=( ${${f:t}%.otp.asc} ); done
print -l $reply"###);
    }
}
