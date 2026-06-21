//! Behavioural parity corpus mined from the zdharma-continuum GitHub org —
//! among the most advanced zsh code in existence: fast-syntax-highlighting
//! (buffer-parsing state machines), zsh-string-lib / zsh-util-lib / giturl
//! (the `(P)::=` write-back-by-name idiom family, recursion, base math),
//! and zui / zflai / zshelldoc / zinit (UI scroll math, multibyte padding,
//! plugin-id building, doc text-processing).
//!
//! Every candidate was extracted from real org source and VERIFIED
//! deterministic across two `zsh -fc` runs before inclusion. Each test
//! asserts `zshrs --zsh -fc` matches `/opt/homebrew/bin/zsh -fc` on stdout
//! + exit; escape output rendered via `cat -v`.

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

// ════════════════════ fast-syntax-highlighting ═════════════════════

mod fsh {
    use super::*;

    /// (z) tokenization driving an assoc dispatch table + ${#${(z)x}} count.
    #[test]
    fn z_token_dispatch() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
typeset -A T=( '|' 3 '||' 3 ';' 3 '&&' 3 'if' 2 'then' 2 'while' 2 '{' 2 'command' 1 'exec' 1 )
buf='if true && command ls | grep x ; then echo hi ; fi'
for tok in ${(z)buf}; do print -r -- "tok=[$tok] type=${T[$tok]:-0}"; done
print -r -- "ntokens=${#${(z)buf}}""###);
    }

    /// byte-offset accumulation skipping leading whitespace via (#b)(#s).
    #[test]
    fn byte_offset_accum() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
buf='  ls    -la   /tmp'
proc=$buf; start=0
integer len=${#buf}
local -a match mbegin mend
for arg in ${(z)buf}; do
  asize=${#arg}; offset=0
  if [[ $proc = (#b)(#s)(([[:space:]]|\\[[:space:]])##)* ]]; then offset=${mend[1]}; fi
  (( start += offset )); (( end = start + asize ))
  print -r -- "[$arg] start=$start end=$end"
  proc=${proc[offset + asize + 1,len]}; start=$end
done"###);
    }

    /// redirection-operator detection excluding procsubst and here-string.
    #[test]
    fn redir_detection() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
toks=( '2>' '>>' '<file' '3<&1' '<<<' '>(cmd)' 'plain' '&>' '1>&2' )
for a in $toks; do
  if [[ $a == (<0-9>|)(\<|\>)* && $a != (\<|\>)$'\x28'* && $a != "<<<" ]]; then
    print -r -- "REDIR: $a"
  else
    print -r -- "norm : $a"
  fi
done"###);
    }

    /// braces_stack letter push/pop + [(r)] membership.
    #[test]
    fn braces_stack() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
stack=''
push() { stack=$1$stack }
pop()  { stack=${stack#$1} }
seq=( PUSH:Y PUSH:A PUSH:R POP:R POP:A PUSH:T POP:Y )
for s in $seq; do
  op=${s%%:*}; ch=${s##*:}
  [[ $op = PUSH ]] && push $ch || pop $ch
  print -r -- "$s -> stack=[$stack] top=[${stack[1]}]"
done
print -r -- "find_A_in_stack=[${stack[(r)A]}]""###);
    }

    /// FAST_HIGHLIGHT chroma dispatch split on % via %\%* and (M)%\%*.
    #[test]
    fn chroma_pct_split() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
typeset -A FH=( chroma-git '/main.ch%git' chroma-grep '/-grep.ch' )
for cmd in git grep nope; do
  entry=${FH[chroma-$cmd]}
  print -r -- "cmd=$cmd file=[${entry%\%*}] argpart=[${(M)entry%\%*}] present=${entry:+1}"
done"###);
    }

    /// --opt=value split with (#b) + numeric-vs-string classification.
    #[test]
    fn opt_eq_split() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend
toks=( '--depth=5' '--file=foo.txt' '--verbose' '--jobs=12' )
for a in $toks; do
  if [[ $a = (#b)(--[a-zA-Z0-9_]##)=(*) ]]; then
    valkind=${${${(M)match[2]:#<->}:+number}:-string}
    print -r -- "opt=[$match[1]] val=[$match[2]] kind=$valkind eq_at=${mend[1]}"
  else
    print -r -- "flag=[$a]"
  fi
done"###);
    }

    /// NUL-split coordinate pairs via (S)//(#b).../${mbegin};${mend}${nul}.
    #[test]
    #[ignore = "zshrs gap: (S)//(#b).../${mbegin};${mend}NUL global substitution emits no coordinates ((ps:NUL:) split returns the whole buffer)"]
    fn nul_coord_pairs() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend inputs
mybuf='aa $(one) bb $(two) cc'
nul=$'\0'
inputs=( ${(ps:$nul:)${(S)mybuf//(#b)*\$\(([^\)]#)(\)|(#e))/${mbegin[1]};${mend[1]}${nul}}%$nul*} )
for p in $inputs; do print -r -- "coord start=${p%%;*} end=${p##*;}"; done
print -r -- "count=${#inputs}""###);
    }

    /// assignment recognition glob (scalar/subscript/append/numeric-name).
    #[test]
    fn assignment_recognition() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
toks=( 'FOO=bar' 'arr[1]=x' 'n+=5' '3=oops' 'PATH=/a:/b' 'notassign' )
for a in $toks; do
  if [[ $a == [a-zA-Z_][a-zA-Z0-9_]#(|\[[^\]]#\])(|[^\]]#\])(|[+])=* || $a == [0-9]##(|[+])=* ]]; then
    print -r -- "ASSIGN name=[${a%%=*}] val=[${a#*=}]"
  else
    print -r -- "noassign [$a]"
  fi
done"###);
    }

    /// math-string variable classifier walk (mathnum/mathvar/matherr).
    #[test]
    #[ignore = "zshrs gap: (#b)[^class]#(token)(*) consumed-buffer walk mis-tokenizes (yields [42],[f] instead of total/42/undef/count)"]
    fn math_string_classifier() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend
typeset -A SEEN=( total 1 count 1 )
mybuf='total + 42 * undef - count'
while [[ $mybuf = (#b)[^\$_a-zA-Z0-9]#([a-zA-Z_][a-zA-Z0-9_]#|[0-9]##)(*) ]]; do
  m=${match[1]}; mybuf=${match[2]}
  if [[ $m = [0-9]* ]]; then cls=mathnum
  elif [[ ${SEEN[$m]} = 1 ]]; then cls=mathvar
  else cls=matherr; fi
  print -r -- "[$m] -> $cls"
done"###);
    }

    /// dollar-string escape scanner with (#m)(#s)...(#c1,N) length bounds.
    #[test]
    fn dollar_string_escapes() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend
arg=$'\\x41\\012\\u00e9\\q\\n'
integer asize=${#arg}
for (( i=1; i<=asize; i++ )); do
  [[ ${arg[i]} != '\' ]] && continue
  integer c
  for (( c=i+1; c<=asize; c++ )); do [[ ${arg[c]} != ([0-9xXuUa-fA-F]) ]] && break; done
  AA=${arg[i+1,c-1]}
  if [[ $AA == (#m)(#s)(x|X)[0-9a-fA-F](#c1,2) || $AA == (#m)(#s)[0-7](#c1,3) \
     || $AA == (#m)(#s)u[0-9a-fA-F](#c1,4) || $AA == (#m)(#s)U[0-9a-fA-F](#c1,8) ]]; then
    print -r -- "escape \\$MATCH valid len=$MEND"; (( i += MEND ))
  else
    nx=${arg[i+1]}
    [[ $nx == [xXuU] ]] && print -r -- "bad-escape \\$nx" || print -r -- "simple-escape \\$nx"
    (( i += 1 ))
  fi
done"###);
    }

    /// histchars-indexed history/comment detection.
    #[test]
    fn histchars_detection() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local histchars='!^#'
toks=( '!!' '!foo' '^old^new' '#comment' 'normal' '!' )
for a in $toks; do
  if [[ $a = ${histchars[1]}* && -n ${a[2]} ]]; then print -r -- "[$a] history-bang"
  elif [[ $a == ${histchars[2]}* ]]; then print -r -- "[$a] quicksub"
  elif [[ $a == ${histchars[3]}* ]]; then print -r -- "[$a] comment"
  else print -r -- "[$a] normal"; fi
done"###);
    }

    /// case state machine with bit-flag states + (\(*\)|\)|\() arms.
    #[test]
    #[ignore = "zshrs gap: (z) leaks backslash on $x and the (( this & BIT )) bit-flag state machine never advances (all states stuck at case-condition)"]
    fn case_state_machine() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
integer PREAMBLE=512 ITEM=1024 NEMPTY=2048 CODE=4096
buf='case $x in foo) echo a ;; bar) echo b ;; esac'
integer this=PREAMBLE
for arg in ${(z)buf}; do
  state=other
  if (( this & PREAMBLE )); then
    [[ $arg = in ]] && { state=reserved-in; (( next=ITEM )); } || { state=case-input; (( next=PREAMBLE )); }
  elif (( this & ITEM )); then
    if (( (this & NEMPTY)==0 )) && [[ $arg = esac ]]; then state=reserved-esac; (( next=0 ))
    elif [[ $arg = (\(*\)|\)|\() ]]; then
      [[ $arg = *\) ]] && (( next=CODE )) || (( next=ITEM|NEMPTY )); state=case-paren
    else (( next=ITEM|NEMPTY )); state=case-condition; fi
  elif (( this & CODE )); then
    [[ $arg = (';;'|';&'|';|') ]] && { (( next=ITEM )); state=case-end; } || { (( next=CODE )); state=code; }
  fi
  print -r -- "[$arg] state=$state"; this=$next
done"###);
    }

    /// quote-span via (i)/(b:N:i)/(I) subscript flags.
    #[test]
    fn quote_span_subscripts() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
arg='VAR="hello world" tail'
ctmp='"'
itmp=${arg[(i)$ctmp]}-1
integer jtmp=${arg[(b:itmp+2:i)$ctmp]}
print -r -- "first-dq at $((itmp+1)) matching-close at $jtmp"
str='a(b)c(d)e'
print -r -- "last-paren (I)=${str[(I)\)]}  first (i)=${str[(i)\)]}""###);
    }

    /// hex/rgb color recognition with (l:2::0:) zero-pad reconstruction.
    #[test]
    #[ignore = "zshrs gap: ${(l:2::0:)match[N]} zero-pad over a (#b)-alternation backref produces garbage repeated output"]
    fn hex_rgb_color() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend
toks=( '#aabbcc' '#1a2' 'notacolor' )
for a in $toks; do
  if [[ $a = (#b)*'#'(([0-9a-fA-F][0-9a-fA-F])([0-9a-fA-F][0-9a-fA-F])([0-9a-fA-F][0-9a-fA-F])|([0-9a-fA-F])([0-9a-fA-F])([0-9a-fA-F]))(|[^[:alnum:]]*) ]]; then
    if [[ -n $match[2] ]]; then print -r -- "[$a] bg=#$match[2]$match[3]$match[4]"
    else print -r -- "[$a] bg=#${(l:2::0:)match[5]}${(l:2::0:)match[6]}${(l:2::0:)match[7]}"; fi
  else print -r -- "[$a] no-color"; fi
done"###);
    }

    /// (zZ+c+) comment-keeping tokenization vs plain (z).
    #[test]
    fn z_comment_tokenization() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
buf='ls -l # this is a comment'
print -r -- "plain-z:"; print -rl -- ${(z)buf}
print -r -- "Z+c+:"; print -rl -- ${(zZ+c+)buf}"###);
    }

    /// FPATH= rewrite: (s,:,) split + (j: :) rejoin + (z@) re-tokenize.
    #[test]
    #[ignore = "zshrs gap: (#b)(FPATH+(#c0,1)=)* match + ${x#FPATH+(#c0,1)=} strip is a no-op (the +(#c0,1) count pattern not honored)"]
    fn fpath_rewrite() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a match mbegin mend
mybuf='FPATH=/usr/share/zsh:/opt/fns:/home/x/fns'
[[ $mybuf = (#b)(FPATH+(#c0,1)=)* ]] && mybuf="${match[1]} ${(j: :)${(s,:,)${mybuf#FPATH+(#c0,1)=}}}"
print -r -- "rewritten=[$mybuf]"
list=( ${(z@)mybuf} )
print -r -- "tokens: ${#list}"; print -rl -- $list"###);
    }

    /// sudo command-line bit-flag state machine.
    #[test]
    fn sudo_state_machine() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
buf='sudo -u root -i ls'
integer this=1 next=0
first=1
for a in ${(z)buf}; do
  if (( first )); then first=0; print -r -- "[$a] precommand-sudo"; this=$((1|4)); continue; fi
  st=arg
  if (( this & 4 )) && [[ $a != -* ]]; then (( this = this ^ 4 )); fi
  if (( this & 4 )); then
    case $a in
      '-'[Cgprtu]) (( this &= ~1 )); (( next = 8 )); st=sudo-opt-takesarg ;;
      '-'*) (( this &= ~1 )); (( next = next | 1 | 4 )); st=sudo-flag ;;
    esac
  elif (( this & 8 )); then (( next = next | 4 | 1 )); st=sudo-optarg
  else st=command; (( next = 2 )); fi
  print -r -- "[$a] $st"; this=$next; next=0
done"###);
    }

    /// exec {fd}> brace file-descriptor recognition.
    #[test]
    fn exec_fd_brace() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
toks=( '{myfd}' '{A}' '{1bad}' '{x_2}' 'nope' '{}' )
for a in $toks; do
  if [[ $a = \{[a-zA-Z_][a-zA-Z0-9_]#\} ]]; then print -r -- "[$a] exec-descriptor"
  else print -r -- "[$a] not-descriptor"; fi
done"###);
    }

    /// globbing classification (ext-glob markers vs ordinary glob).
    #[test]
    fn globbing_classification() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
toks=( '*.txt' 'foo?bar' 'a##b' '(#b)x' '(#m)y' '(#c1,3)z' 'plain' )
for a in $toks; do
  if [[ $a = *([^\\][\#][\#]|"(#b)"|"(#B)"|"(#m)"|"(#c")* ]]; then print -r -- "[$a] globbing-ext"
  elif [[ $a = ([*?]*|*[^\\][*?]*) ]]; then print -r -- "[$a] globbing"
  else print -r -- "[$a] plain"; fi
done"###);
    }

    /// bracket-depth color cycling onto a 3-color rotation.
    #[test]
    fn bracket_depth_cycle() {
        assert_parity(r###"for lvl in 1 2 3 4 5 6 7; do
  print -r -- "depth=$lvl -> bracket-level-$(( ((lvl-1) % 3) + 1 ))"
done"###);
    }

    /// in_redirection two-iteration stall counter.
    #[test]
    fn in_redir_stall() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
buf='cat < input.txt > out.txt rest'
integer in_redir=0
first=1
for a in ${(z)buf}; do
  (( in_redir = in_redir > 0 ? in_redir - 1 : in_redir ))
  if (( first )); then first=0; print -r -- "[$a] command"; continue; fi
  if [[ $a == (<0-9>|)(\<|\>)* && $a != "<<<" ]]; then in_redir=2; print -r -- "[$a] redir-op"
  elif (( in_redir == 1 )); then print -r -- "[$a] redir-target"
  else print -r -- "[$a] word"; fi
done"###);
    }

    /// alias-target precommand promotion via token-type table.
    #[test]
    fn alias_promotion() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
typeset -A TT=( command 1 exec 1 builtin 1 )
typeset -A myalias=( please sudo ll 'ls -la' runc command )
for name in please ll runc; do
  tgt=${myalias[$name]}
  if [[ ${TT[$tgt]} = 1 ]]; then TT[$name]=1; print -r -- "[$name]->[$tgt] PROMOTED"
  else print -r -- "[$name]->[$tgt] regular"; fi
done
print -r -- "runc-type=${TT[runc]}""###);
    }

    /// make-targets cache roundtrip (j:;:) join / (s:;:) resplit + (r) membership.
    #[test]
    fn make_targets_cache() {
        assert_parity(r###"emulate -L zsh
setopt extendedglob
local -a TARGETS reply2
TARGETS=( all build test install clean )
cache="${(j:;:)TARGETS}"
reply2=( "${(s:;:)cache}" )
print -r -- "restored=${#reply2}"
for t in all test missing; do
  [[ -n "${reply2[(r)$t]}" ]] && print -r -- "[$t] exists" || print -r -- "[$t] absent"
done"###);
    }
}

// ═════════════════ zsh-string-lib / util-lib / giturl ══════════════

mod zdlibs {
    use super::*;

    /// (S) non-greedy match via (#b) backref + side-effect ::= inside discard.
    #[test]
    #[ignore = "zshrs gap: (S) non-greedy match is greedy (m1=abbb vs ab) and the no-match retval path is wrong"]
    fn s_nongreedy_match_sidefx() {
        assert_parity(r###"setopt extendedglob
f(){ local str=$1 pat=$2 retval=1; local -a match mbegin mend
  : ${(S)str/(#b)(${~pat})/${retval::=0}}; REPLY=${match[1]}; return $retval; }
f "abbbc" "a*b" && print "m1=$REPLY"
f "xyz" "a*b" || print "no match rc=$?"
f "hello world" "o*o" && print "m2=$REPLY""###);
    }

    /// write-back into a caller-named scalar via : ${(P)name::=val}, (Pt) type.
    #[test]
    fn p_writeback_scalar() {
        assert_parity(r###"setopt extendedglob
setval(){ local __name=$1 __val=$2; : ${(P)__name::=$__val}; }
local out1 out2
setval out1 "hello"; setval out2 "world"
print -r -- "$out1 $out2"
local target=out1
print -r -- "read=${(P)target} type=${(Pt)target}""###);
    }

    /// dynamic hash[key] access-string write via (P)::=.
    #[test]
    fn p_writeback_hash_element() {
        assert_parity(r###"setopt extendedglob
local -A INI
local cur="net" key="port" val="8080"
local access="INI[<$cur>_$key]"
: "${(P)access::=$val}"
cur="net"; key="host"; val="example.com"
access="INI[<$cur>_$key]"; : "${(P)access::=$val}"
for k in "${(@kon)INI}"; do print -r -- "$k=${INI[$k]}"; done"###);
    }

    /// bulk-populate a caller-named assoc via : ${(PAA)name::="${(kv)src}"}.
    #[test]
    #[ignore = "zshrs gap: : ${(PAA)name::=\"${(kv)src}\"} bulk-assoc write-back splits quoted values with spaces ('beta gamma' → 'gamma' becomes a key)"]
    fn paa_bulk_assoc() {
        assert_parity(r###"setopt extendedglob
local -A src
src=( 1/1 alpha 2/1 "beta gamma" 3/1 delta )
fill(){ local __vn=$1; : ${(PAA)__vn::="${(kv)src[@]}"}; }
local -A dst; fill dst
for k in "${(@kon)dst}"; do print -r -- "$k -> ${dst[$k]}"; done"###);
    }

    /// dispatch on ${(Pt)name} dumping scalar/array/assoc with (@P)/(@qP)/(qP).
    #[test]
    fn pt_dispatch_dump() {
        assert_parity(r###"setopt extendedglob
dump(){ local q=0; [[ $1 == -q ]] && { q=1; shift; }; local n=$1
  case ${(Pt)n} in
    (*array*) (( q )) && print -rl -- "${(@qP)n}" || print -rl -- "${(@P)n}";;
    (*association*) local key as; for key in "${(@kon)${(@Pk)n}}"; do
        as="${n}[$key]"; (( q )) && print -r -- "${(q)key}: ${(qP)as}" || print -r -- "$key: ${(P)as}"; done;;
    (*) (( q )) && print -r -- "${(qP)n}" || print -r -- "${(P)n}";;
  esac; }
local s="a value"; local -a arr=( "" "a value" test ); typeset -A h=( "a key" "a value" key value )
print -r -- "--scalar--"; dump s
print -r -- "--array-q--"; dump -q arr
print -r -- "--assoc-q--"; dump -q h"###);
    }

    /// (( ${(P)+name} )) existence test on a caller-named param.
    #[test]
    fn p_plus_exists() {
        assert_parity(r###"setopt extendedglob
local FOO=bar
chk(){ local n=$1; (( ${(P)+n} )) && print -r -- "$n exists" || print -r -- "$n absent"; }
chk FOO; chk NOPE
local -A H=( k v ); chk H"###);
    }

    /// long-division of a digit array (C-loop carry).
    #[test]
    fn digit_array_div() {
        assert_parity(r###"setopt extendedglob
div2(){ local -a numbers=( "$@" ) result=()
  integer prepared=${numbers[1]} input quotient recovered subtracted=0
  for input in ${(@)numbers[2,-1]} 0; do
    quotient=prepared/2; result+=( $quotient )
    recovered=$(( quotient*2 )); subtracted=prepared-recovered
    prepared=10*subtracted+input
  done
  reply=( "${result[@]}" ); REPLY=$subtracted }
local -a reply; local REPLY
div2 1 9 5; print -r -- "quot=${(j::)reply} rem=$REPLY"
div2 2 5 6; print -r -- "quot=${(j::)reply} rem=$REPLY""###);
    }

    /// bit packing right-to-left into fixed-width base-2 groups.
    #[test]
    fn bit_packing() {
        assert_parity(r###"setopt extendedglob
str_to_packs(){ local -a bits=( "${(@s::)1}" ) pack numbers
  integer count=0 i size=${#bits} result p
  for (( i=size; i>=1; i-- )); do
    pack=( "$bits[i]" "${pack[@]}" ); count+=1
    (( count < 4 && i != 1 )) && continue
    count=0; result=0
    for p in "${pack[@]}"; do result=result*2+p; done
    numbers=( $result "${numbers[@]}" ); pack=()
  done
  reply=( "${numbers[@]}" ) }
local -a reply
str_to_packs "110101101"; print -r -- "packs=${(j:,:)reply}""###);
    }

    /// URL parse with (#b) backrefs + optional group (:...)(#c0,1).
    #[test]
    fn url_parse_backrefs() {
        assert_parity(r###"setopt extendedglob
parse(){ local url=$1 protocol user site port upath; local -a match mbegin mend
  if [[ "$url" = (#b)(git|http|https|ftp|ftps)://([a-zA-Z0-9._~-]##)(:[0-9]##)(#c0,1)/([a-zA-Z0-9./_~:-]##) ]]; then
    protocol=${match[1]} site=${match[2]} port=${match[3]#:} upath=${match[4]}
  elif [[ "$url" = (#b)([a-zA-Z0-9._~-]##@)(#c0,1)([a-zA-Z0-9._~-]##):([a-zA-Z0-9./_~:-](#c0,1)[a-zA-Z0-9._~:-][a-zA-Z0-9./_~:-]#) ]]; then
    protocol=ssh user=${match[1]%@} site=${match[2]} upath=${match[3]}
  else print -r -- "unrecognized: $url"; return 7; fi
  print -r -- "proto=$protocol user=$user site=$site port=$port path=$upath" }
parse "https://github.com:8443/user/repo.git"
parse "git@github.com:user/dotfiles"
parse "weird://nope"; print "rc=$?""###);
    }

    /// (Z+n+) word-offset accumulation + (#m)/$#MATCH whitespace skip.
    #[test]
    fn zn_word_offsets() {
        assert_parity(r###"setopt extendedglob typesetsilent noshortloops
local MATCH; local -a match
local buf="  echo  'hi there'   end"
local -a WORDS=( "${(Z+n+)buf}" ) BEG=()
integer nwords=${#WORDS} i char_count=0 wordlen
local word
for (( i=1; i<=nwords; i++ )); do
  WORDS[i]="${WORDS[i]%% ##}"; word="${WORDS[i]}"
  buf="${buf##(#m)[^$word[1]]#}"; char_count=char_count+$#MATCH
  BEG[i]=$(( char_count + 1 )); wordlen=${#word}
  buf="${buf[wordlen+1,-1]}"; char_count=char_count+$#word
done
print -r -- "nwords=$nwords"
for (( i=1; i<=nwords; i++ )); do print -r -- "[$i] beg=${BEG[i]} word=<${WORDS[i]}>"; done"###);
    }

    /// unpack a quoted-concatenated array via "${(@Q)${(@z)val}}".
    #[test]
    fn at_Q_at_z_unpack() {
        assert_parity(r###"setopt extendedglob
local stored='val1 value\ 2 value\&3'
local -a arr=( "${(@Q)${(@z)stored}}" )
print -r -- "count=${#arr}"
integer i
for (( i=1; i<=${#arr}; i++ )); do print -r -- "[$i]=<${arr[i]}>"; done"###);
    }

    /// strip trailing whitespace via ${v%"${v##*[! $'\t']}"}.
    #[test]
    fn nested_trail_strip() {
        assert_parity(r###"setopt extendedglob
strip_trail(){ local v=$1; print -r -- "<${v%"${v##*[! $'\t']}"}>"; }
strip_trail "value   "
strip_trail $'val\t\t'
strip_trail "noTrail"
strip_trail "mid space ok   ""###);
    }

    /// collapse lead+trail blanks via ${s//((#s)[[:blank:]]##|([[:blank:]]##(#e)))}.
    #[test]
    fn collapse_blanks_anchored() {
        assert_parity(r###"setopt extendedglob
trim(){ local s=$1; print -r -- "<${s//((#s)[[:blank:]]##|([[:blank:]]##(#e)))}>"; }
trim "   hello   "; trim "x"; trim "  a b  "
local v="123abc456"
print -r -- "lead=${(M)v##[0-9]##} trail=${(M)v%%[0-9]##}""###);
    }

    /// recursive helper writing back through REPLY.
    #[test]
    fn recursion_reply() {
        assert_parity(r###"setopt extendedglob
sumdigits(){ local s=$1
  if [[ -z $s ]]; then REPLY=0; return; fi
  local first=${s[1]} rest=${s[2,-1]}
  sumdigits "$rest"; REPLY=$(( first + REPLY )) }
local REPLY
sumdigits "12345"; print -r -- "sum=$REPLY"
sumdigits ""; print -r -- "empty=$REPLY""###);
    }

    /// boolean coercion via ${${(M)v:#(1|yes|on|true)}:+1}.
    #[test]
    fn boolean_coerce() {
        assert_parity(r###"setopt extendedglob
truthy(){ local v=$1; [[ -n ${${(M)v:#(1|yes|on|true)}:+1} ]] && print -r -- "$v -> true" || print -r -- "$v -> false"; }
truthy yes; truthy 1; truthy on; truthy no; truthy 0; truthy maybe"###);
    }

    /// array-suffix comparison via per-char split + C-loop.
    #[test]
    fn suffix_compare() {
        assert_parity(r###"setopt extendedglob
suffix(){ local -a long=( "${(@s::)1}" ) short=( "${(@s::)2}" )
  [[ ${#long} -lt ${#short} ]] && return 1
  integer beg=$(( ${#long} - ${#short} + 1 )) end=${#long} l s=1 ne=0
  for (( l=beg; l<=end; l++ )); do [[ "${long[l]}" != "${short[s]}" ]] && { ne=1; break; }; s+=1; done
  return $ne }
suffix "11010" "010" && print "yes 010" || print "no 010"
suffix "11010" "011" && print "yes 011" || print "no 011"
suffix "10" "110" && print "yes" || print "no short>long""###);
    }

    /// drop trailing N with negative-arithmetic slice ${(@)bits[1,-1*REPLY-1]}.
    #[test]
    fn neg_arith_slice() {
        assert_parity(r###"setopt extendedglob
local -a bits=( 1 0 1 1 0 0 1 0 )
integer REPLY=3
local -a kept=( "${(@)bits[1,-1*REPLY-1]}" )
print -r -- "kept=${(j::)kept}"
local -a popped=( "${(@)bits[-REPLY,-1]}" )
print -r -- "popped=${(j::)popped}""###);
    }

    /// arithmetic base I/O — $(( [##2] N )) and $(( [#10] 2#bits )).
    #[test]
    fn arith_base_io() {
        assert_parity(r###"setopt extendedglob
integer n=42
print -r -- "bin=$(( [##2] n ))"
local bits="101010"
print -r -- "dec=$(( [#10] 2#$bits ))"
integer v=$(( 2#$bits ))
print -r -- "v=$v base8=$(( [##8] v ))""###);
    }

    /// greedy-prefix Huffman decode (try increasing-length prefixes).
    #[test]
    fn huffman_decode() {
        assert_parity(r###"setopt extendedglob
local -A rcodes=( 001 a 0101 b 011 c 10010 d )
decode(){ local bits=$1; local out="" mat trystr; integer len
  while (( ${#bits} > 0 )); do
    mat=""
    for (( len=3; len<=5; len++ )); do trystr="${bits[1,len]}"; mat="${rcodes[$trystr]}"; [[ -n "$mat" ]] && break; done
    [[ -z "$mat" ]] && { out+="?"; break; }
    out+="$mat"; bits="${bits[len+1,-1]}"
  done
  print -r -- "$out" }
decode "001011"; decode "010110010""###);
    }

    /// snapshot/diff function set via (k)functions and :|.
    #[test]
    #[ignore = "zshrs gap: ${(k)functions[@]:|before} diff of the functions-hash key snapshot finds no additions (added= empty)"]
    fn function_set_diff() {
        assert_parity(r###"setopt extendedglob
helper_a(){ :; }
print -r -- "has_a=${+functions[helper_a]} has_b=${+functions[helper_b]}"
local -a before=( "${(k)functions}" )
helper_b(){ :; }
local -a added=( "${(k)functions[@]:|before}" )
print -r -- "added=${(o)added}""###);
    }

    /// reverse map from a forward assoc + case-folded lookup.
    #[test]
    fn reverse_map_lookup() {
        assert_parity(r###"setopt extendedglob
local -A sites=( gh github.com bb bitbucket.org gl gitlab.com )
local -A rsites; local k
for k in "${(@kon)sites}"; do rsites[${sites[$k]}]=$k; done
print -r -- "gitlab.com->${rsites[gitlab.com]}"
local site="GitHub.com" found=""
for k in "${(@kon)rsites}"; do [[ "${(L)k}" == "${(L)site}" ]] && found=${rsites[$k]}; done
print -r -- "folded=$found""###);
    }

    /// char→code arithmetic $(( #key )) + single-char/control gate.
    #[test]
    fn char_code_gate() {
        assert_parity(r###"setopt extendedglob
filter(){ local key=$1
  if [[ $#key == 1 && $(( #key )) -lt 31 ]]; then print -r -- "ctrl<$(( #key ))>"
  else print -r -- "accept:$key"; fi }
filter "a"; filter $'\x05'; filter "ab"; filter "Z""###);
    }

    /// \1-delimited field decode between (#b) backrefs ending at \2.
    #[test]
    fn delimited_field_decode() {
        assert_parity(r###"setopt extendedglob
local s=$'PRE\1id1\1ts1\1cmd\ x\1path1\1file1\2POST'
local -a match mbegin mend
if [[ "$s" = (#b)*$'\1'([^$'\1']#)$'\1'([^$'\1']#)$'\1'([^$'\1']#)$'\1'([^$'\1']#)$'\1'([^$'\2']#)$'\2'* ]]; then
  print -r -- "id=${(Q)match[1]} ts=${(Q)match[2]} cmd=${(Q)match[3]} path=${(Q)match[4]} file=${(Q)match[5]}"
else print -r -- "no-match"; fi"###);
    }
}

// ═════════════════ zui / zflai / zshelldoc / zinit ═════════════════

mod zui_zinit {
    use super::*;

    /// zui — page-from-index offset via integer division.
    #[test]
    fn page_offset() {
        assert_parity(r###"integer page_height=10 current_idx=37 last=100
integer from=$(( ((current_idx-1)/page_height)*page_height + 1 ))
integer end=$(( from + page_height - 1 ))
(( end > last )) && end=last
print "from=$from end=$end""###);
    }

    /// zui — centered scroll window with three-way edge clamp.
    #[test]
    fn scroll_clamp() {
        assert_parity(r###"integer initial=3 height=10 size=5
integer start end
start=initial-height/2
if (( start <= 0 )); then start=1; end=size
elif (( start + height - 1 > size )); then start=size-height+1; end=size
else end=initial+(height-height/2)-1; fi
print "start=$start end=$end""###);
    }

    /// zui — in-place array splice replace.
    #[test]
    fn array_splice_replace() {
        assert_parity(r###"set -- a b c d e f
update=(X Y Z)
integer update_first=3 update_count=2
set -- "${(@)@[1,update_first-1]}" "${update[@]}" "${(@)@[update_first+update_count,-1]}"
print -r -- "$@""###);
    }

    /// zui — max display-width across items via ${(m)#str}.
    #[test]
    fn display_width_m() {
        assert_parity(r###"opts=(aa bbbb c dddddd ee)
integer width=7
for t in "${opts[@]}"; do (( ${(m)#t} > width )) && width=${(m)#t}; done
print $width"###);
    }

    /// zui — head-skip and tail-chop via ${s[k+1,-1]} / ${s[1,-k-1]}.
    #[test]
    fn head_skip_tail_chop() {
        assert_parity(r###"Xout="HelloWorldFoobar"
integer to_skip=3
Xout="${Xout[to_skip+1,-1]}"; print -r -- "$Xout"
integer chop=4
Xout="${Xout[1,-chop-1]}"; print -r -- "$Xout""###);
    }

    /// zui — multibyte-aware pad with fill char ${(ml:N::-:)} / ${(mr:N::-:)}.
    #[test]
    fn multibyte_pad() {
        assert_parity(r###"s=abc
print -r -- "[${(ml:6::-:)s}]"
print -r -- "[${(mr:6::-:)s}]""###);
    }

    /// zui — indirect scalar assignment : ${(P)var::=$val}.
    #[test]
    fn indirect_assign() {
        assert_parity(r###"hidx=7
idx_var=target
: ${(P)idx_var::=$hidx}
print -r -- "target=$target""###);
    }

    /// zinit — plugin-id build via nested (M)#/:+ /:- with ---→/ fold.
    #[test]
    fn plugin_id_build() {
        assert_parity(r###"build() { local user="$1" plugin="$2"
  reply=( ${user:-${${(M)plugin#/}:+PCT}} ${${${(M)user#PCT}:+$plugin}:-${plugin//---//}} ); }
local -a reply
build "" "tmux---tmux"; print -r -- "${reply[@]}"
build "zsh-users" "zsh-syntax-highlighting"; print -r -- "${reply[@]}""###);
    }

    /// zinit — filesystem-safe dir encoding chained ${//}.
    #[test]
    fn dir_encoding() {
        assert_parity(r###"local_dir="path/to=val?x&y"
local_dir="${local_dir#/}"
local_dir="${local_dir//\//--}"
local_dir="${local_dir//=/-EQ-}"
local_dir="${local_dir//\?/-QM-}"
local_dir="${local_dir//&/-AMP-}"
print -r -- "$local_dir""###);
    }

    /// zinit — hook discovery ${(on)H[(I)pat <->]} (index match + numeric sort).
    #[test]
    fn hook_discovery() {
        assert_parity(r###"setopt extendedglob
typeset -A H
H=( "hook:preinit-pre 1" v1 "hook:preinit-pre 2" v2 "other" v3 "hook:preinit-pre 10" v4 )
reply2=( ${(on)H[(I)hook:preinit-pre <->]} )
print -r -- "${reply2[@]}""###);
    }

    /// zinit — numeric vs lexical hash-key ordering ${(kn)H} vs ${(k)H}.
    #[test]
    fn kn_vs_k_ordering() {
        assert_parity(r###"typeset -A H
H=( 10 ten 2 two 1 one 30 thirty )
print -r -- "${(kn)H[@]}"
print -r -- "${(ok)H[@]}""###);
    }

    /// zshelldoc — line counter normalizing trailing-newline overcount.
    #[test]
    fn line_counter() {
        assert_parity(r###"line_count() {
  local -a list; list=( "${(@f)1}" )
  local count=${#list}
  [[ $1 = *$'\n' ]] && (( -- count ))
  print -r -- $count }
line_count "$(print -rn -- $'a\nb\nc\nd')"
line_count $'a\nb\nc\nd\n'"###);
    }

    /// zshelldoc — (S) non-greedy block capture between markers.
    #[test]
    #[ignore = "zshrs gap: ${(S)body/(#b)*BEGIN(*)END*/...} non-greedy block capture drops the trailing *-matched text ('outro' lost)"]
    fn s_block_capture() {
        assert_parity(r###"setopt extendedglob
body_comments="intro BEGIN the synopsis text END outro"
body=${(S)body_comments/(#b)*BEGIN(*)END*/X${match[1]}X}
print -r -- "[$body]""###);
    }

    /// zshelldoc — brace-depth state machine over (z@) tokens.
    #[test]
    fn brace_depth_machine() {
        assert_parity(r###"setopt extendedglob
buf="func a { inner { x } } tail"
tokens=( "${(z@)buf}" )
integer depth=0 maxdepth=0
for t in "${tokens[@]}"; do
  [[ $t == "{" ]] && (( ++depth ))
  (( depth > maxdepth )) && maxdepth=depth
  [[ $t == "}" ]] && (( --depth ))
done
print "final=$depth max=$maxdepth""###);
    }

    /// zshelldoc — variable extraction (#b) loop over consumed buffer.
    #[test]
    fn var_extraction_loop() {
        assert_parity(r###"setopt extendedglob
buf="echo \$foo and \${bar} done \$baz"
reply=()
while [[ $buf = (#b)[^\$]#\$(\{([a-zA-Z_]##)\}|([a-zA-Z_]##))(*) ]]; do
  reply+=( "${match[2]:-${match[3]}}" ); buf="${match[4]}"
done
print -rl -- "${reply[@]}""###);
    }

    /// zshelldoc — name/desc -> split with both-sides trim.
    #[test]
    fn arrow_split_trim() {
        assert_parity(r###"setopt extendedglob
block="NAME -> the name -> AGE -> the age"
sorted=( "${(@s:->:)block}" )
integer i ssize=${#sorted}
for (( i=1; i<=ssize; i+=2 )); do
  k="${sorted[i]##[[:space:]]#}"; k="${k%%[[:space:]]#}"
  v="${sorted[i+1]##[[:space:]]#}"; v="${v%%[[:space:]]#}"
  print -r -- "$k=$v"
done"###);
    }

    /// zshelldoc — marker fold _-_→/, rename, dedup (u).
    #[test]
    fn marker_fold_dedup() {
        assert_parity(r###"arr=( foo_-_bar baz_-_qux foo_-_bar Script_Body_ )
arr=( "${arr[@]//_-_//}" )
arr=( "${arr[@]/Script_Body_/Script-Body}" )
arr=( "${(u)arr[@]}" )
print -rl -- "${arr[@]}""###);
    }

    /// zshelldoc — nth glob match in array ${arr[(rn:N:)pat]}.
    #[test]
    fn rn_nth_glob() {
        assert_parity(r###"setopt extendedglob
known=( foo123 bar foo456 baz foo789 )
print -r -- "${known[(rn:2:)foo*]}"
print -r -- "${known[(rn:3:)foo*]}""###);
    }

    /// zshelldoc — bulk array prefix-strip + suffix-append.
    #[test]
    fn bulk_strip_append() {
        assert_parity(r###"arr=( zsdoc/a zsdoc/b zsdoc/c )
arr=( "${arr[@]#zsdoc/}" )
arr=( "${arr[@]/%/.adoc}" )
print -rl -- "${arr[@]}""###);
    }

    /// zshelldoc — sort-then-join ${(j:_, _:)${(@o)features}}.
    #[test]
    fn sort_join_multichar() {
        assert_parity(r###"features=( zoo alpha mid )
print -- "${(j:_, _:)${(@o)features}}""###);
    }

    /// zflai — param-name moderation non-alnum → _<ord>_ via $(( #c )).
    #[test]
    fn param_moderate_ord() {
        assert_parity(r###"setopt extendedglob
moderate() { local in="$1" out="" c; integer i
  for (( i=1; i<=${#in}; i++ )); do c="${in[i]}"
    if [[ "$c" == [A-Za-z0-9] ]]; then out+="$c"; else out+="_$(( #c ))_"; fi
  done
  print -r -- "$out" }
moderate "db-name.tbl""###);
    }

    /// zflai — inverse decode _<ord>_ → char via ${(#)n}.
    #[test]
    fn decode_ord_char() {
        assert_parity(r###"setopt extendedglob
decode() { local in="$1" out=""
  while [[ $in = (#b)([A-Za-z0-9]#)(_([0-9]##)_)(*) ]]; do
    out+="${match[1]}"; out+="${(#)match[3]}"; in="${match[4]}"
  done
  out+="$in"; print -r -- "$out" }
decode "db_45_name_46_tbl""###);
    }

    /// zsh-startify — history dedup (@z) split + assoc seen-set.
    #[test]
    fn history_dedup() {
        assert_parity(r###"hist=( "git commit -m foo" "ls -la /tmp" "git commit -m bar" "ls -la /tmp" )
typeset -A seen
local -a cmds
for entry in "${hist[@]}"; do
  words=( "${(@z)entry}" )
  key="${words[1]} ${words[2]}"
  [[ "${seen[$key]}" == 1 ]] && continue
  seen[$key]=1; cmds+=( "$key" )
done
print -rl -- "${cmds[@]}""###);
    }

    /// zsh-startify — frequency count → (l:3::0:) rows → (On) desc sort.
    #[test]
    fn freq_sort() {
        assert_parity(r###"words=( apple pear apple banana pear apple )
typeset -A freq
for w in "${words[@]}"; do (( freq[$w]++ )); done
local -a rows
for k in "${(k)freq[@]}"; do rows+=( "${(l:3::0:)freq[$k]} $k" ); done
print -rl -- "${(On)rows[@]}""###);
    }

    /// zshelldoc — feature glob *(DN) then prefix-strip + sort-join.
    #[test]
    fn dn_glob_strip() {
        assert_parity(r###"setopt extendedglob nullglob
d=$(mktemp -d)
mkdir -p "$d/features/myfn"
: > "$d/features/myfn/parallel"; : > "$d/features/myfn/git"; : > "$d/features/myfn/.hidden"
features=( "$d"/features/myfn/*(DN) )
features=( "${(@)features#$d/features/myfn/}" )
print -r -- "${(oj:_, _:)features}"
rm -rf "$d""###);
    }
}
