//! Behavioural parity corpus mined from the zsh manual's MODULE and
//! USER-CONTRIBUTIONS chapters (zsh.sourceforge.io/Doc/Release: Zsh
//! Modules, User Contributions) — the part of the docs the `man zshall`
//! pass did not cover.
//!
//! Each test loads the relevant module (`zmodload zsh/X`) or autoloads
//! the contrib function, then exercises a documented behaviour, and
//! asserts `zshrs --zsh -fc` matches `/opt/homebrew/bin/zsh -fc` on
//! stdout + exit. Both run with `-f` (no rc). Dates pin `TZ=UTC` + fixed
//! epochs; float results use fixed precision; stat/files use mktemp
//! sandboxes. Every script was verified byte-identical across two
//! consecutive real-zsh runs before inclusion.
//!
//! Modules are where parity gaps cluster hardest: a module that zshrs
//! does not implement makes every test for it diverge at once.

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
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
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

// ════════════════════════════ zsh/zutil ════════════════════════════

mod zmod_zutil {
    use super::*;

    /// zstyle precedence — more colon components is more specific.
    #[test]
    fn zstyle_component_count() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":a:*" s shallow
zstyle ":a:b:*" s deep
zstyle -s ":a:b:c" s R; print $R"###,
        );
    }

    /// zstyle specificity — literal beats *.
    #[test]
    fn zstyle_literal_beats_star() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":a:b" s val_exact
zstyle ":a:*" s val_star
zstyle -s ":a:b" s R; print $R"###,
        );
    }

    /// zstyle -e evaluated style.
    #[test]
    fn zstyle_e_evaluated() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle -e ":ctx:*" dyn 'reply=(computed-$((1+1)))'
zstyle -s ":ctx:foo" dyn R; print $R"###,
        );
    }

    /// zstyle -e reply unset → style unset.
    #[test]
    fn zstyle_e_reply_unset() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle -e ":ctx:*" dyn 'true'
if zstyle -s ":ctx:foo" dyn R; then print "set:$R"; else print "unset"; fi"###,
        );
    }

    /// zstyle -b boolean retrieval.
    #[test]
    fn zstyle_b_boolean() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" flag yes
zstyle -b ":c" flag R; print "$R $?"
zstyle ":c2" flag bogus
zstyle -b ":c2" flag R2; print "$R2 $?""###,
        );
    }

    /// zstyle -a array, and into assoc.
    #[test]
    fn zstyle_a_array_assoc() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" arr one two three
zstyle -a ":c" arr R
print ${#R} ${R[1]} ${R[3]}
typeset -A H
zstyle ":c" pairs k1 v1 k2 v2
zstyle -a ":c" pairs H
print ${H[k1]} ${H[k2]}"###,
        );
    }

    /// zstyle -t / -T test status.
    #[test]
    fn zstyle_t_T_status() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" t1 yes
zstyle -t ":c" t1; print "tt:$?"
zstyle -t ":c" nope; print "tu:$?"
zstyle -T ":c" nope; print "Tu:$?"
zstyle ":c" t2 no
zstyle -t ":c" t2; print "tf:$?""###,
        );
    }

    /// zstyle -t with strings.
    #[test]
    fn zstyle_t_with_strings() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" lst alpha beta gamma
zstyle -t ":c" lst beta; print "m:$?"
zstyle -t ":c" lst zzz; print "n:$?""###,
        );
    }

    /// zstyle -m pattern match value.
    #[test]
    fn zstyle_m_match() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" colors red green blue
zstyle -m ":c" colors "gr*"; print "m:$?"
zstyle -m ":c" colors "x*"; print "n:$?""###,
        );
    }

    /// zstyle -s with separator.
    #[test]
    fn zstyle_s_separator() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" parts a b c
zstyle -s ":c" parts R "+"
print $R"###,
        );
    }

    /// zstyle -g retrieve definition.
    #[test]
    fn zstyle_g_retrieve() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":p1" sg one two
zstyle -g out ":p1" sg
print ${#out} ${out[1]} ${out[2]}"###,
        );
    }

    /// zstyle -d delete.
    #[test]
    fn zstyle_d_delete() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":c" k v
zstyle -d ":c" k
if zstyle -s ":c" k R; then print "still:$R"; else print "deleted"; fi"###,
        );
    }

    /// zstyle -L list as zstyle calls.
    #[test]
    fn zstyle_L_list() {
        assert_parity(
            r###"zmodload zsh/zutil
zstyle ":ml:ctx" mystyle valA valB
zstyle -L ":ml:*" mystyle"###,
        );
    }

    /// zformat -f basic substitution.
    #[test]
    fn zformat_basic() {
        assert_parity(
            r###"zmodload zsh/zutil
zformat -f REPLY "hello %n you are %a" n:world a:here
print $REPLY"###,
        );
    }

    /// zformat -f min/neg width.
    #[test]
    fn zformat_widths() {
        assert_parity(
            r###"zmodload zsh/zutil
zformat -f REPLY "[%10x]" x:hi
print $REPLY
zformat -f REPLY "[%-10x]" x:hi
print $REPLY"###,
        );
    }

    /// zformat -f max truncate and combined min.max.
    #[test]
    fn zformat_truncate() {
        assert_parity(
            r###"zmodload zsh/zutil
zformat -f REPLY "[%.3x]" x:abcdefgh
print $REPLY
zformat -f REPLY "[%8.3x]" x:abcdefgh
print $REPLY"###,
        );
    }

    /// zformat -f ternary expression.
    #[test]
    fn zformat_ternary() {
        assert_parity(
            r###"zmodload zsh/zutil
zformat -f REPLY "ans=%3(c.yes.no)" c:3
print $REPLY
zformat -f R2 "ans=%3(c.yes.no)" c:5
print $R2"###,
        );
    }

    /// zformat -a align.
    #[test]
    fn zformat_align() {
        assert_parity(
            r###"zmodload zsh/zutil
zformat -a out " -- " "foo:bar" "longerkey:val" "nocolon" "empty:"
print -l $out"###,
        );
    }

    /// zparseopts b:/c+: doc example.
    #[test]
    fn zparseopts_doc_example() {
        assert_parity(
            r###"zmodload zsh/zutil
set -- -a -bx -c y -cz baz -cend
zparseopts a=foo b:=bar c+:=bar
print -l "foo:" $foo "bar:" $bar"###,
        );
    }

    /// zparseopts -E -D extract+remove.
    #[test]
    fn zparseopts_E_D() {
        assert_parity(
            r###"zmodload zsh/zutil
set -- -a x -b y -c z arg1 arg2
zparseopts -E -D b:=bar
print "bar:" $bar
print "rest:" "$@""###,
        );
    }

    /// zparseopts optional :: and same-element :-.
    #[test]
    fn zparseopts_optional_and_dash() {
        assert_parity(
            r###"zmodload zsh/zutil
set -- -o -p val rest
zparseopts o::=oo p::=pp
print "oo:[$oo]"
print "pp:[$pp]"
set -- -fVALUE
zparseopts f:-=ff
printf "[%s]\n" "${ff[@]}""###,
        );
    }

    /// zparseopts name+ append and -A assoc.
    #[test]
    fn zparseopts_append_and_A() {
        assert_parity(
            r###"zmodload zsh/zutil
set -- -v -v -v
zparseopts v+=verbose
print ${#verbose}
set -- -a -b val
zparseopts -A H a b:
print "a:[${H[-a]}] b:[${H[-b]}]""###,
        );
    }

    /// zparseopts -K keep, GNU long, no =.
    ///
    /// The two long-option specs are introduced with `--`. Upstream zsh
    /// moved zparseopts's own flags onto the generic builtin option parser
    /// (`Src/Modules/zutil.c:2150`, optstring `"a:A:DEFGKMn:v:"`), so a bare
    /// `zparseopts -file:=ff` is now rejected by `Src/builtin.c:385-390` as
    /// `bad option: -f` before the builtin runs — exactly what upstream's
    /// Test/V12zparseopts.ztst "zparseopts long-option spec guarding" pins.
    /// `--` (or `-`) is the spelling that reaches the spec parser on both
    /// that revision and the zsh 5.9.2 this harness diffs against.
    #[test]
    fn zparseopts_K_long() {
        assert_parity(
            r###"zmodload zsh/zutil
arr=(default)
set -- foo
zparseopts -K -a arr x
print -l $arr
set -- --file data.txt
zparseopts -- -file:=ff
printf "[%s]\n" "${ff[@]}"
set -- --foo=bar
zparseopts -- -foo:=gg
printf "[%s]\n" "${gg[@]}""###,
        );
    }

    /// zparseopts -F validate fails on unknown.
    #[test]
    fn zparseopts_F_validate() {
        assert_parity(
            r###"zmodload zsh/zutil
set -- -a -z
zparseopts -F a=aa; print "exit=$?""###,
        );
    }
}

// ══════════════════════════ zsh/parameter ══════════════════════════

mod zmod_parameter {
    use super::*;

    /// dis_functions — disabled function moves out of functions.
    #[test]
    fn dis_functions() {
        assert_parity(
            r###"zmodload zsh/parameter
greet() { print hello; }
disable -f greet
print "en:${+functions[greet]} dis:${+dis_functions[greet]}""###,
        );
    }

    /// dis_aliases — disabled alias moves to dis_aliases.
    #[test]
    fn dis_aliases() {
        assert_parity(
            r###"zmodload zsh/parameter
alias zz="echo z"
disable -a zz
print "en:${+aliases[zz]} dis:${+dis_aliases[zz]} val:${dis_aliases[zz]}""###,
        );
    }

    /// builtins values defined/undefined.
    #[test]
    fn builtins_values() {
        assert_parity(
            r###"zmodload zsh/parameter
print ${builtins[print]}"###,
        );
    }

    /// dis_builtins — disabled builtin moves.
    #[test]
    fn dis_builtins() {
        assert_parity(
            r###"zmodload zsh/parameter
disable getln
print "en:${+builtins[getln]} dis:${+dis_builtins[getln]} disval:${dis_builtins[getln]}""###,
        );
    }

    /// dis_reswords — disabled reserved word moves.
    #[test]
    fn dis_reswords() {
        assert_parity(
            r###"zmodload zsh/parameter
disable -r foreach
print "en:${reswords[(r)foreach]:-none} dis:${dis_reswords[(r)foreach]}""###,
        );
    }

    /// modules values loaded/autoloaded.
    #[test]
    fn modules_values() {
        assert_parity(
            r###"zmodload zsh/parameter
zmodload zsh/zutil
print ${modules[zsh/zutil]}"###,
        );
    }

    /// parameters type strings for typeset variants.
    #[test]
    fn parameters_type_strings() {
        assert_parity(
            r###"zmodload zsh/parameter
typeset -i ivar=5
typeset -a avar=(x)
typeset -A hvar=(k v)
typeset -r rvar=ro
print "i:${parameters[ivar]} a:${parameters[avar]} A:${parameters[hvar]} r:${parameters[rvar]}"
export EV=1
print ${parameters[EV]}"###,
        );
    }

    /// functrace caller name:lineno.
    #[test]
    fn functrace() {
        assert_parity(
            r###"zmodload zsh/parameter
inner() { print ${functrace[1]%%:*}; }
outer() { inner; }
outer"###,
        );
    }

    /// funcsourcetrace filename:lineno — reports the def-statement line
    /// (`inner()` on line 2), not the body offset.
    #[test]
    fn funcsourcetrace() {
        assert_parity(
            r###"zmodload zsh/parameter
inner() { print ${funcsourcetrace[1]##*:}; }
inner"###,
        );
    }

    /// trace arrays equal length.
    #[test]
    fn trace_lengths_equal() {
        assert_parity(
            r###"zmodload zsh/parameter
inner() { print "eq:$(( ${#funcfiletrace}==${#funcsourcetrace} && ${#functrace}==${#funcstack} ? 1 : 0 ))"; }
outer() { inner; }
outer"###,
        );
    }

    /// options mutation affects shell state.
    #[test]
    fn options_mutation() {
        assert_parity(
            r###"zmodload zsh/parameter
setopt noextendedglob
options[extendedglob]=on
if [[ -o extendedglob ]]; then print active; else print inactive; fi"###,
        );
    }
}

// ══════════════════════════ zsh/mathfunc ═══════════════════════════

mod zmod_math {
    use super::*;

    /// sqrt/cbrt integer-valued print as N.
    #[test]
    fn sqrt_cbrt_int() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( sqrt(16) )) $(( cbrt(27) ))"###);
    }

    /// sqrt/cbrt fractional fixed precision.
    #[test]
    fn sqrt_cbrt_frac() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f\n" $(( sqrt(2) )) $(( cbrt(2) ))"###,
        );
    }

    /// exp(1) = e.
    #[test]
    fn exp_e() {
        assert_parity(r###"zmodload zsh/mathfunc; printf "%.6f\n" $(( exp(1) ))"###);
    }

    /// log/log10/log2 exact.
    #[test]
    fn logs() {
        assert_parity(
            r###"zmodload zsh/mathfunc; echo $(( log(1) )) $(( log10(1000) )) $(( log2(8) ))"###,
        );
    }

    /// sin/cos/tan at 0.
    #[test]
    fn trig_zero() {
        assert_parity(
            r###"zmodload zsh/mathfunc; echo $(( sin(0) )) $(( cos(0) )) $(( tan(0) ))"###,
        );
    }

    /// asin/acos/atan.
    #[test]
    fn inverse_trig() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f %.6f\n" $(( asin(1) )) $(( acos(0) )) $(( atan(1) ))"###,
        );
    }

    /// atan two-arg (atan2).
    #[test]
    fn atan2() {
        assert_parity(r###"zmodload zsh/mathfunc; printf "%.6f\n" $(( atan(1,1) ))"###);
    }

    /// hyperbolic at 0.
    #[test]
    fn hyperbolic_zero() {
        assert_parity(
            r###"zmodload zsh/mathfunc; echo $(( sinh(0) )) $(( cosh(0) )) $(( tanh(0) ))"###,
        );
    }

    /// ceil/floor/fabs.
    #[test]
    fn ceil_floor_fabs() {
        assert_parity(
            r###"zmodload zsh/mathfunc; echo $(( ceil(2.3) )) $(( floor(2.7) )) $(( fabs(-3.5) ))"###,
        );
    }

    /// fmod/hypot.
    #[test]
    fn fmod_hypot() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( fmod(10,3) )) $(( hypot(3,4) ))"###);
    }

    /// ldexp/scalb/logb.
    #[test]
    fn ldexp_scalb_logb() {
        assert_parity(
            r###"zmodload zsh/mathfunc; echo $(( ldexp(1,4) )) $(( scalb(1,4) )) $(( logb(8) ))"###,
        );
    }

    /// ilogb returns integer.
    #[test]
    fn ilogb_int() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( ilogb(8) ))"###);
    }

    /// expm1/log1p.
    #[test]
    fn expm1_log1p() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f\n" $(( expm1(0) )) $(( log1p(0) ))"###,
        );
    }

    /// gamma/lgamma.
    #[test]
    fn gamma_lgamma() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f\n" $(( gamma(5) )) $(( lgamma(5) ))"###,
        );
    }

    /// erf/erfc.
    #[test]
    fn erf_erfc() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f\n" $(( erf(0) )) $(( erfc(0) ))"###,
        );
    }

    /// abs preserves type.
    #[test]
    fn abs_type() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( abs(-7) )) $(( abs(-7.5) ))"###);
    }

    /// int/float conversions.
    #[test]
    fn int_float() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( int(3.9) )) $(( float(3) ))"###);
    }

    /// copysign.
    #[test]
    fn copysign() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( copysign(3,-1) ))"###);
    }

    /// inverse hyperbolic at identity points.
    #[test]
    fn inverse_hyperbolic() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f %.6f\n" $(( acosh(1) )) $(( asinh(0) )) $(( atanh(0) ))"###,
        );
    }

    /// pow NOT provided as a function (** operator instead).
    #[test]
    fn pow_not_function() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( pow(2,10) )) 2>&1"###);
    }

    /// min/max/sum NOT in module.
    #[test]
    fn min_not_in_module() {
        assert_parity(r###"zmodload zsh/mathfunc; echo $(( min(3,5) )) 2>&1"###);
    }

    /// rand48 seeded determinism.
    #[test]
    fn rand48_seeded() {
        assert_parity(
            r###"zmodload zsh/mathfunc; seed=0123456789abcdef; print $(( rand48(seed) )); print $seed"###,
        );
    }

    /// Bessel j0/j1.
    #[test]
    fn bessel() {
        assert_parity(
            r###"zmodload zsh/mathfunc; printf "%.6f %.6f\n" $(( j0(0) )) $(( j1(0) ))"###,
        );
    }
}

// ══════════════════════════ zsh/datetime ═══════════════════════════

mod zmod_datetime {
    use super::*;

    /// strftime basic format (fixed epoch, UTC).
    #[test]
    fn strftime_basic() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime "%Y-%m-%d %H:%M:%S" 1000000000"###,
        );
    }

    /// strftime specifier set.
    #[test]
    fn strftime_specifiers() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime "%Y %m %d %H %M %S" 1700000000"###,
        );
    }

    /// %j day of year.
    #[test]
    fn strftime_doy() {
        assert_parity(r###"export TZ=UTC; zmodload zsh/datetime; strftime "%j" 1000000000"###);
    }

    /// weekday/month names.
    #[test]
    fn strftime_names() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime "%A %a %B %b" 1000000000"###,
        );
    }

    /// %p %u %w %s %F %T %C.
    #[test]
    fn strftime_more() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime "%p|%u|%w|%s|%F|%T|%C" 1700000000"###,
        );
    }

    /// strftime -s assign to scalar.
    #[test]
    fn strftime_s_assign() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime -s var "%Y-%m-%d" 1700000000; echo $var"###,
        );
    }

    /// strftime -n suppress newline.
    #[test]
    fn strftime_n_nonewline() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime -n "%Y" 1700000000; echo X"###,
        );
    }

    /// strftime -r reverse parse to epoch.
    #[test]
    fn strftime_r_parse() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime -r "%Y-%m-%d %H:%M:%S" "2023-11-14 22:13:20""###,
        );
    }

    /// strftime -r -s round-trip.
    #[test]
    fn strftime_r_s() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime -r -s e "%Y-%m-%d" "2001-09-09"; echo $e"###,
        );
    }

    /// strftime nanoseconds + %N.
    #[test]
    fn strftime_nanoseconds() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/datetime; strftime "%s.%N" 1700000000 123456789"###,
        );
    }

    /// epochtime array shape.
    #[test]
    fn epochtime_shape() {
        assert_parity(r###"zmodload zsh/datetime; echo ${#epochtime}"###);
    }

    /// EPOCHSECONDS readonly integer.
    #[test]
    fn epochseconds_readonly() {
        assert_parity(
            r###"zmodload zsh/datetime; [[ $EPOCHSECONDS == <-> ]] && echo intlike; EPOCHSECONDS=5 2>&1; echo "exit=$?""###,
        );
    }
}

// ════════════════════════════ zsh/stat ═════════════════════════════

mod zmod_stat {
    use super::*;

    /// +size.
    #[test]
    fn size() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hello" > $t/f; cd "$t"; zstat +size f; rm -rf "$t""###,
        );
    }

    /// -s symbolic mode.
    #[test]
    fn symbolic_mode() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; chmod 644 $t/f; cd "$t"; zstat -s +mode f; rm -rf "$t""###,
        );
    }

    /// -o octal raw mode.
    #[test]
    fn octal_mode() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; chmod 644 $t/f; cd "$t"; zstat -o +mode f; rm -rf "$t""###,
        );
    }

    /// +nlink.
    #[test]
    fn nlink() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; cd "$t"; zstat +nlink f; rm -rf "$t""###,
        );
    }

    /// -l element name list.
    #[test]
    fn element_list() {
        assert_parity(r###"zmodload zsh/stat; zstat -l"###);
    }

    /// -H hash keys + value access.
    #[test]
    fn hash_keys() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hello" > $t/f; cd "$t"; zstat -H h +size f; print -l ${(ok)h}; echo $h[size]; rm -rf "$t""###,
        );
    }

    /// -A array.
    #[test]
    fn array_out() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hello" > $t/f; cd "$t"; zstat -A arr +size f; echo $arr; rm -rf "$t""###,
        );
    }

    /// -t element-name prefix.
    #[test]
    fn name_prefix() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hello" > $t/f; cd "$t"; zstat -A arr -t +size f; echo $arr; rm -rf "$t""###,
        );
    }

    /// element abbreviation +si.
    #[test]
    fn abbreviation() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hello" > $t/f; cd "$t"; zstat +si f; rm -rf "$t""###,
        );
    }

    /// ambiguous abbreviation error.
    #[test]
    fn ambiguous_error() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; cd "$t"; zstat +m f 2>&1; echo "exit=$?"; rm -rf "$t""###,
        );
    }

    /// +mtime raw epoch.
    #[test]
    fn mtime_epoch() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; touch -t 202001010000.00 $t/f; cd "$t"; zstat +mtime f; rm -rf "$t""###,
        );
    }

    /// -F time format + -g GMT.
    #[test]
    fn time_format_gmt() {
        assert_parity(
            r###"export TZ=UTC; zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; touch -t 199501020304.05 $t/f; cd "$t"; zstat -g -F "%Y-%m-%dT%H:%M:%S" +mtime f; rm -rf "$t""###,
        );
    }

    /// -L lstat symlink size.
    #[test]
    fn lstat_symlink() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; ln -s f $t/lnk; cd "$t"; zstat -L +size lnk; rm -rf "$t""###,
        );
    }

    /// +link target.
    #[test]
    fn link_target() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; ln -s f $t/lnk; cd "$t"; zstat -L +link lnk; rm -rf "$t""###,
        );
    }

    /// -n always show filename.
    #[test]
    fn force_name() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "x" > $t/f; cd "$t"; zstat -n +size f; rm -rf "$t""###,
        );
    }

    /// multiple files names, -N suppresses.
    #[test]
    fn multifile_names() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "ab" > $t/a; print -n "cdef" > $t/b; cd "$t"; zstat +size a b; echo ---; zstat -N +size a b; rm -rf "$t""###,
        );
    }

    /// stat alias name.
    #[test]
    fn stat_alias() {
        assert_parity(
            r###"zmodload zsh/stat; t=$(mktemp -d); print -n "hi" > $t/f; cd "$t"; stat +size f; rm -rf "$t""###,
        );
    }

    /// nonexistent file error.
    #[test]
    fn nonexistent_error() {
        assert_parity(r###"zmodload zsh/stat; zstat +size /no/such/zzz 2>&1; echo "exit=$?""###);
    }
}

// ═══════════════════════════ zsh/system ════════════════════════════

mod zmod_system {
    use super::*;

    /// zsystem flock acquire+release.
    #[test]
    fn flock_acquire_release() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); f=$t/lk; : >$f; (zsystem flock -f myfd $f; print locked $?; zsystem flock -u $myfd; print unlocked $?); rm -rf $t"###,
        );
    }

    /// zsystem flock subshell auto-release.
    #[test]
    fn flock_subshell() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); : >$t/f; ( zsystem flock $t/f && print held ); print after=$?; rm -rf $t"###,
        );
    }

    /// zsystem supports known/unknown/syntax.
    #[test]
    fn supports() {
        assert_parity(
            r###"zmodload zsh/system; zsystem supports flock; print rc=$?; zsystem supports nonsuchthing; print rc=$?; zsystem supports; print rc=$?"###,
        );
    }

    /// $errnos membership + readonly.
    #[test]
    fn errnos() {
        assert_parity(r###"zmodload zsh/system; print ${errnos[(I)EINVAL]:+yes}"###);
    }

    /// $signals[1] EXIT and index lookup.
    #[test]
    fn signals() {
        assert_parity(
            r###"print $signals[1]; print $signals[(i)INT]; print ${signals[(r)TERM]:+T}${signals[(r)ZERR]:+Z}"###,
        );
    }

    /// syserror -e to var.
    #[test]
    fn syserror() {
        assert_parity(r###"zmodload zsh/system; syserror -e ev ENOENT; print $ev"###);
    }

    /// sysread into var + count.
    #[test]
    fn sysread_count() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); print -n "abcde" >$t/f; exec {fd}<$t/f; sysread -c n -i $fd buf; print "$n:$buf"; exec {fd}<&-; rm -rf $t"###,
        );
    }

    /// sysread EOF returns 5.
    #[test]
    fn sysread_eof() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); : >$t/f; exec {fd}<$t/f; sysread -i $fd buf; print rc=$?; exec {fd}<&-; rm -rf $t"###,
        );
    }

    /// syswrite -o fd + count.
    #[test]
    fn syswrite() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); exec {fd}>$t/f; syswrite -c c -o $fd "hello"; print "wrote=$c"; exec {fd}>&-; print "$(<$t/f)"; rm -rf $t"###,
        );
    }

    /// sysseek + systell() — the zsh/system systell math function now
    /// dispatches to the ported math_systell (gated on zmodload).
    #[test]
    fn sysseek_systell() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); print -n "0123456789" >$t/f; exec {fd}<$t/f; sysseek -u $fd 3; sysread -i $fd buf; print "[$buf] tell=$(( systell($fd) ))"; exec {fd}<&-; rm -rf $t"###,
        );
    }

    /// sysopen -r -u var.
    #[test]
    fn sysopen() {
        assert_parity(
            r###"zmodload zsh/system; t=$(mktemp -d); print -n "xyz" >$t/f; sysopen -r -u myfd $t/f; sysread -i $myfd b; print "[$b]"; exec {myfd}<&-; rm -rf $t"###,
        );
    }
}

// ════════════════════════════ zsh/files ════════════════════════════

mod zmod_files {
    use super::*;

    /// zf_mkdir -p.
    #[test]
    fn mkdir_p() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); zf_mkdir -p $t/a/b/c; [[ -d $t/a/b/c ]] && print made; rm -rf $t"###,
        );
    }

    /// zf_mkdir -m mode.
    #[test]
    fn mkdir_m() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); zf_mkdir -m 700 $t/d; [[ -x $t/d ]] && print ok; rm -rf $t"###,
        );
    }

    /// zf_ln -s symlink.
    #[test]
    fn ln_s() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); print hi >$t/orig; zf_ln -s $t/orig $t/link; [[ -L $t/link ]] && print "[$(<$t/link)]"; rm -rf $t"###,
        );
    }

    /// zf_ln hard link.
    #[test]
    fn ln_hard() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); print z >$t/a; zf_ln $t/a $t/b; print "$(<$t/b)"; rm -rf $t"###,
        );
    }

    /// zf_ln -sf replaces existing.
    #[test]
    fn ln_sf() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); print one >$t/a; print two >$t/b; zf_ln -sf $t/a $t/b; print "$(<$t/b)"; rm -rf $t"###,
        );
    }

    /// zf_chmod then [[ -x ]].
    #[test]
    fn chmod() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); : >$t/f; zf_chmod 755 $t/f; [[ -x $t/f ]] && print exec; zf_chmod 644 $t/f; [[ -x $t/f ]] || print noexec; rm -rf $t"###,
        );
    }

    /// zf_mv.
    #[test]
    fn mv() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); print mvd >$t/a; zf_mv $t/a $t/b; [[ -e $t/a ]] || print "gone:[$(<$t/b)]"; rm -rf $t"###,
        );
    }

    /// zf_rm.
    #[test]
    fn rm() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); : >$t/f; zf_rm $t/f; [[ -e $t/f ]] && print here || print removed; rm -rf $t"###,
        );
    }

    /// zf_rm -r recursive.
    #[test]
    fn rm_r() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); mkdir -p $t/d/sub; : >$t/d/sub/f; zf_rm -r $t/d; [[ -e $t/d ]] && print here || print gone; rm -rf $t"###,
        );
    }

    /// zf_rm -f missing file ok.
    #[test]
    fn rm_f_missing() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); zf_rm -f $t/nope; print rc=$?; rm -rf $t"###,
        );
    }

    /// zf_rmdir empty / non-empty.
    #[test]
    fn rmdir() {
        assert_parity(
            r###"zmodload zsh/files; t=$(mktemp -d); mkdir $t/e; zf_rmdir $t/e; [[ -d $t/e ]] && print here || print removed; mkdir $t/d; : >$t/d/f; zf_rmdir $t/d 2>/dev/null; print rc=$?; rm -rf $t"###,
        );
    }

    /// zf_sync.
    #[test]
    fn sync() {
        assert_parity(r###"zmodload zsh/files; zf_sync; print rc=$?"###);
    }

    /// features-only zf_ load.
    #[test]
    fn features_only_load() {
        assert_parity(
            r###"zmodload -m -F zsh/files b:zf_\*; t=$(mktemp -d); zf_mkdir $t/q; [[ -d $t/q ]] && print made; rm -rf $t"###,
        );
    }

    /// no cp/zf_cp builtin (documented set).
    #[test]
    fn no_zf_cp() {
        assert_parity(r###"zmodload zsh/files; zf_cp 2>/dev/null; print rc=$?"###);
    }
}

// ═══════════════════════ zsh/pcre + zsh/regex ══════════════════════

mod zmod_pcre_regex {
    use super::*;

    /// pcre_compile + pcre_match MATCH.
    #[test]
    fn pcre_match_basic() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "[0-9]+"; pcre_match "abc123def"; print "[$MATCH]""###,
        );
    }

    /// capture groups set $match.
    #[test]
    fn pcre_captures() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "([a-z]+)-([0-9]+)"; pcre_match "foo-42"; print "$match[1]/$match[2]""###,
        );
    }

    /// named groups.
    #[test]
    fn pcre_named() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "(?<word>[a-z]+)"; pcre_match "hello"; print "$match[1]""###,
        );
    }

    /// \d perl class.
    #[test]
    fn pcre_perl_class() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "\d{3}"; pcre_match "x456y"; print "[$MATCH]""###,
        );
    }

    /// lookahead success/failure.
    #[test]
    fn pcre_lookahead() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "foo(?=bar)"; pcre_match "foobar"; print "rc=$? [$MATCH]"; pcre_match "foobaz"; print rc=$?"###,
        );
    }

    /// ZPCRE_OP byte offsets (-b).
    #[test]
    fn pcre_zpcre_op() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "\d+"; pcre_match -b "ab1234"; print "$ZPCRE_OP""###,
        );
    }

    /// pcre_compile -i case-insensitive.
    #[test]
    fn pcre_i() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile -i "abc"; pcre_match "XABCY"; print "[$MATCH]""###,
        );
    }

    /// pcre_match -a / -v custom dests.
    #[test]
    fn pcre_a_v() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile "(a)(b)"; pcre_match -a arr "ab"; print "$arr[1]$arr[2]"; pcre_compile "x+"; pcre_match -v mv "axxxb"; print "[$mv]""###,
        );
    }

    /// [[ -pcre-match ]] condition.
    #[test]
    fn pcre_condition() {
        assert_parity(
            r###"zmodload zsh/pcre; [[ "abc123" -pcre-match "[0-9]+" ]]; print "rc=$? [$MATCH]""###,
        );
    }

    /// pcre_compile -x extended.
    #[test]
    fn pcre_x() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile -x "\d+ # the number"; pcre_match "abc42"; print "[$MATCH]""###,
        );
    }

    /// pcre_compile -s dotall / -m multiline.
    #[test]
    fn pcre_s_m() {
        assert_parity(
            r###"zmodload zsh/pcre; pcre_compile -s "a.b"; pcre_match $'a\nb'; print "rc=$?"; pcre_compile -m "^bar$"; pcre_match $'foo\nbar\nbaz'; print "rc=$? [$MATCH]""###,
        );
    }

    /// REMATCH_PCRE makes =~ use PCRE.
    #[test]
    fn rematch_pcre() {
        assert_parity(
            r###"zmodload zsh/pcre; setopt REMATCH_PCRE; [[ "ab12" =~ "\d+" ]]; print "rc=$? [$MATCH]""###,
        );
    }

    /// zsh/regex -regex-match sets MATCH and match.
    #[test]
    fn regex_match_builtin() {
        assert_parity(
            r###"zmodload zsh/regex; [[ alphabetical -regex-match "^a([^a]+)a([^a]+)a" ]]; print "$MATCH|$match[1]|$match[2]""###,
        );
    }

    /// =~ defaults to POSIX ERE with $match.
    #[test]
    fn regex_default_posix() {
        assert_parity(r###"[[ "key=val" =~ "([a-z]+)=([a-z]+)" ]]; print "$match[1]/$match[2]""###);
    }

    /// no match leaves MATCH unchanged.
    #[test]
    fn regex_no_match_unchanged() {
        assert_parity(
            r###"zmodload zsh/regex; MATCH=orig; [[ "abc" -regex-match "[0-9]+" ]]; print "rc=$? [$MATCH]""###,
        );
    }

    /// BASH_REMATCH option.
    #[test]
    fn bash_rematch() {
        assert_parity(
            r###"zmodload zsh/regex; setopt BASH_REMATCH; [[ "x99y" -regex-match "([0-9]+)" ]]; print "[$BASH_REMATCH[1]][$BASH_REMATCH[2]]""###,
        );
    }
}

// ════════════════════ zsh/mapfile + zsh/langinfo ═══════════════════

mod zmod_mapfile_langinfo {
    use super::*;

    /// mapfile read whole file.
    #[test]
    fn mapfile_read() {
        assert_parity(
            r###"zmodload zsh/mapfile; t=$(mktemp -d); print -n "line1
line2" >$t/f; print "${mapfile[$t/f]}"; rm -rf $t"###,
        );
    }

    /// mapfile assignment writes file.
    #[test]
    fn mapfile_write() {
        assert_parity(
            r###"zmodload zsh/mapfile; t=$(mktemp -d); mapfile[$t/g]="written via mapfile"; print "$(<$t/g)"; rm -rf $t"###,
        );
    }

    /// mapfile unset deletes file.
    #[test]
    fn mapfile_unset() {
        assert_parity(
            r###"zmodload zsh/mapfile; t=$(mktemp -d); print x >$t/h; unset "mapfile[$t/h]"; [[ -e $t/h ]] && print exists || print gone; rm -rf $t"###,
        );
    }

    /// mapfile split into array with (f@).
    #[test]
    fn mapfile_split() {
        assert_parity(
            r###"zmodload zsh/mapfile; t=$(mktemp -d); print -n "a
b
c" >$t/f; a=("${(f@)mapfile[$t/f]}"); print "${#a} ${a[2]}"; rm -rf $t"###,
        );
    }

    /// langinfo CODESET key present + type.
    #[test]
    fn langinfo_codeset() {
        assert_parity(
            r###"zmodload zsh/langinfo; print ${+langinfo[CODESET]}; print ${(t)langinfo}"###,
        );
    }
}

// ════════════════════════ User Contributions ═══════════════════════

mod contrib {
    use super::*;

    /// zmv -n dry-run pattern rename.
    #[test]
    fn zmv_dryrun() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > foo.txt; : > bar.txt; zmv -n "(*).txt" "$1.bak"; cd /; rm -rf $t"###,
        );
    }

    /// zmv (#b) backreference swap.
    #[test]
    fn zmv_backref_swap() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > a-b.c; : > x-y.c; zmv "(#b)(*)-(*).c" "$2-$1.c"; print -l *(N:t) | sort; cd /; rm -rf $t"###,
        );
    }

    /// zmv -W wildcard shorthand.
    #[test]
    fn zmv_W() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > one.txt; : > two.txt; zmv -W "*.txt" "*.bak"; print -l *(N:t) | sort; cd /; rm -rf $t"###,
        );
    }

    /// zmv -C copy mode.
    #[test]
    fn zmv_C_copy() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > src.txt; zmv -C "(*).txt" "$1.copy"; print -l *(N:t) | sort; cd /; rm -rf $t"###,
        );
    }

    /// zmv -L hardlink mode.
    #[test]
    fn zmv_L_link() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > orig.txt; zmv -L "(*).txt" "$1.lnk"; print -l *(N:t) | sort; [[ orig.lnk -ef orig.txt ]] && print SAMEINODE; cd /; rm -rf $t"###,
        );
    }

    /// zmv numeric range capture.
    #[test]
    fn zmv_numeric() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > file2; : > file10; zmv -n "file(<->)" "img$1"; cd /; rm -rf $t"###,
        );
    }

    /// zmv (#m) MATCH with modifier.
    #[test]
    fn zmv_match_modifier() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > abc.log; zmv -n "(#m)*.log" "${MATCH:r}.txt"; cd /; rm -rf $t"###,
        );
    }

    /// zmv case flag on capture.
    #[test]
    fn zmv_case_flag() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > FOO.txt; zmv -n "(#b)(*).txt" "${(L)1}.txt"; cd /; rm -rf $t"###,
        );
    }

    /// zmv -p program.
    #[test]
    fn zmv_p_program() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > z.txt; zmv -n -p print "(*).txt" "$1.zzz"; cd /; rm -rf $t"###,
        );
    }

    /// zmv collision aborts.
    #[test]
    fn zmv_collision() {
        assert_parity(
            r###"autoload -U zmv; t=$(mktemp -d); cd $t; : > a.txt; : > b.txt; zmv -n "(?).txt" "x.txt" 2>&1; cd /; rm -rf $t"###,
        );
    }

    /// zargs -n batching.
    #[test]
    fn zargs_n() {
        assert_parity(r###"autoload -U zargs; zargs -n 2 -- a b c d e -- print"###);
    }

    /// zargs default command.
    #[test]
    fn zargs_default() {
        assert_parity(r###"autoload -U zargs; zargs -- x y z -- print -r --"###);
    }

    /// zargs -i replacement.
    #[test]
    fn zargs_i() {
        assert_parity(r###"autoload -U zargs; zargs -i -- a b -- print got: {} end"###);
    }

    /// colors $color name→code, attribute, bg, reverse, British.
    #[test]
    fn colors_color_map() {
        assert_parity(
            r###"autoload -U colors; colors; print -- ${color[red]} ${color[bold]} ${color[bg-blue]} ${color[31]} ${colour[green]}"###,
        );
    }

    /// colors $fg/$bg escapes.
    #[test]
    fn colors_fg_bg() {
        assert_parity(
            r###"autoload -U colors; colors; print -- ${fg[red]}${bg[green]} | cat -v"###,
        );
    }

    /// colors $fg_bold / $reset_color / $bold_color.
    #[test]
    fn colors_bold_reset() {
        assert_parity(
            r###"autoload -U colors; colors; print -- ${fg_bold[cyan]}${reset_color}${bold_color} | cat -v"###,
        );
    }

    /// is-at-least true/false/dash-segment.
    #[test]
    fn is_at_least() {
        assert_parity(
            r###"autoload -U is-at-least; is-at-least 5.0 5.9.1; print $?; is-at-least 6.0 5.9.1; print $?; is-at-least 3.1.6-15 3.1.6-14; print $?"###,
        );
    }

    /// regexp-replace global + class + $MATCH + no-match.
    #[test]
    fn regexp_replace() {
        assert_parity(
            r###"autoload -U regexp-replace; v="foo123bar456"; regexp-replace v "[0-9]+" "#"; print $v; v2="aaa"; regexp-replace v2 "x" "y"; print "rc=$? v=$v2""###,
        );
    }

    /// zmathfuncdef define + optional default.
    #[test]
    fn zmathfuncdef() {
        assert_parity(
            r###"autoload -U zmathfuncdef; zmathfuncdef cube "$1*$1*$1"; print $(( cube(3) )); zmathfuncdef inc "$1+${2:-1}"; print $(( inc(5) )) $(( inc(5,10) ))"###,
        );
    }

    /// zmathfunc min/max/sum.
    #[test]
    fn zmathfunc() {
        assert_parity(
            r###"autoload -U zmathfunc; zmathfunc; print $(( max(3,7,2) )) $(( min(3,7,2) )) $(( sum(1,2,3,4) ))"###,
        );
    }

    /// zcalc -e non-interactive eval + float + PI + multiple.
    #[test]
    fn zcalc_e() {
        assert_parity(
            r###"autoload -U zcalc; zcalc -e "2+3"; zcalc -e -f "3/4"; zcalc -e "2+3" "10*10""###,
        );
    }

    /// add-zsh-hook register + remove.
    #[test]
    fn add_zsh_hook() {
        assert_parity(
            r###"autoload -U add-zsh-hook; myfn(){ :; }; add-zsh-hook precmd myfn; print -l $precmd_functions; add-zsh-hook -d precmd myfn; print "count=${#precmd_functions}""###,
        );
    }
}
