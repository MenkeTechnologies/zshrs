//! Port of `_subscript` from `Completion/Zsh/Context/_subscript`.
//!
//! Full upstream body (134 lines), branch map:
//! ```text
//! sh:  1  #compdef -subscript-
//! sh:  3  local expl ind osuf flags sep
//! sh:  5  [[ $ISUFFIX = *\]* ]] || osuf=\]
//! sh:  7  if [[ "$1" = -q ]]; then compquote osuf; osuf+=' '; shift; fi
//! sh: 13  compset -P '\(([^\(\)]|\(*\))##\)'          # strip subscript flags
//! sh: 21  pos scan → BUFFER[1,pos-1] = (|*[[:space:]:=]##)~[ → _dynamic_directory_name
//! sh: 25  elif $PREFIX = :*  → _wanted characters … compadd -p: -S ':]' <classes>
//! sh: 29  elif compset -P '\('  → subscript-flag _values catalog (assoc/scalar/array)
//! sh: 84  elif (Pt)==assoc*  → _wanted association-keys … compadd -Q -S $suf -a keys
//! sh: 93  elif (Pt)==array*  → _tags indexes parameters loop (_all_labels / _parameters)
//! sh:132  else _dispatch -math- -math-
//! ```
//!
//! Local names mirror the source: `osuf`, `flags`, `ind`, `sep`,
//! `match`→`f`/`d`/`e`/`v`, `keys`, `suf`, `list`, `disp`, `ret`.
//! `compadd` is invoked as the action word of `_wanted`/`_all_labels`
//! exactly as the source does (those ports route the `compadd` action
//! word to `bin_compadd` internally). Scratch by-name arrays (`keys`,
//! `ind`, `list`) are unset after use.

use crate::compsys::ported::_all_labels::all_labels_byname;
use crate::compsys::ported::_dynamic_directory_name::dynamic_directory_name_byname;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_parameters::call_parameters;
use crate::compsys::ported::_requested::requested_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::compsys::ported::_values::_values;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{bin_zformat, bin_zstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zle::computil::bin_compquote;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

/// `${(Pt)name}` — typeset-style type of the parameter *named* by
/// `name`. Returns `"association"`, `"array"`, `"integer"`,
/// `"float"`, `"scalar"`, or `""` (unset). Substring-compatible with
/// the source's `assoc*` / `array*` / `scalar*` case tests.
///
/// The type comes from the `paramtab` node's `PM_TYPE` flag bits —
/// the same read `${(t)param}` performs (`subst.rs:14836-14858`,
/// c:Src/subst.c:2817-2825) and the same predicate `zle_tricky.rs`
/// uses to decide assoc-key vs math subscript context
/// (`param_is_hashed`, zle_tricky.rs:1742-1752, c:Src/Zle/zle_tricky.c:
/// 1515). Flags — not a value lookup — is what upstream tests, and it
/// is the only read that answers for the module-magic specials
/// (`commands`, `aliases`, `functions`, …): those live in
/// `PARTAB`/`PARTAB_ARRAY` and are seeded into `paramtab` as
/// `PM_SPECIAL` stubs carrying the row's `PM_HASHED`/`PM_ARRAY` bit
/// (`vm_helper.rs:5121-5156 init_partab_params`) while their VALUES
/// are served by `partab_get`/`partab_scan_keys`/`partab_array_get`.
/// A value-based classification therefore reported `""` for every one
/// of them and `_subscript` fell through to `_dispatch -math- -math-`.
///
/// The value lookups are kept as a fallback for names with no
/// `paramtab` node at all (e.g. an assoc whose only trace is the
/// parallel `paramtab_hashed_storage` map) and for namerefs, whose
/// target type `getaparam`/`getsparam` already resolve.
fn param_type(name: &str) -> String {
    use crate::ported::zsh_h::{
        PM_ARRAY, PM_DECLARED, PM_EFLOAT, PM_FFLOAT, PM_HASHED, PM_INTEGER, PM_NAMEREF, PM_UNSET,
    };
    if name.is_empty() {
        return String::new();
    }
    // `paramtab` PM_TYPE read — c:Src/subst.c:2814. A shadowing local
    // replaces the special's node (c:Src/params.c:1090-1115
    // createparam), so this automatically reports the visible binding's
    // type rather than the special's.
    let flags: Option<u32> = crate::ported::params::paramtab()
        .read()
        .ok()
        .and_then(|tab| tab.get(name).map(|pm| pm.node.flags as u32));
    if let Some(f) = flags {
        // c:Src/subst.c:2855-2856 — an unset *and* undeclared param has
        // no type tag. Same guard as the `(t)` arm at subst.rs:14665.
        let live = (f & PM_DECLARED) != 0 || (f & PM_UNSET) == 0;
        if live {
            // c:2817-2825 — same precedence as the `(t)` tag builder.
            if f & PM_HASHED != 0 {
                return s("association");
            } else if f & PM_ARRAY != 0 {
                return s("array");
            } else if f & PM_INTEGER != 0 {
                return s("integer");
            } else if f & (PM_EFLOAT | PM_FFLOAT) != 0 {
                return s("float");
            } else if f & PM_NAMEREF == 0 {
                return s("scalar");
            }
            // PM_NAMEREF falls through: the value lookups below resolve
            // the reference and report the TARGET's type (c:2800-2806).
        }
    }
    let is_assoc = crate::ported::params::paramtab_hashed_storage()
        .lock()
        .map(|t| t.contains_key(name))
        .unwrap_or(false);
    if is_assoc {
        s("association")
    } else if getaparam(name).is_some() {
        s("array")
    } else if getsparam(name).is_some() {
        s("scalar")
    } else {
        String::new()
    }
}

/// sh:104 — `[[ "$i" = ${PREFIX}*${SUFFIX} ]]`. PREFIX/SUFFIX are
/// glob patterns in the source; for array indexes they are literal
/// digit strings, so this treats them literally (prefix + anything +
/// suffix). Approximation of the general glob case.
fn index_matches(i: &str, prefix: &str, suffix: &str) -> bool {
    i.len() >= prefix.len() + suffix.len() && i.starts_with(prefix) && i.ends_with(suffix)
}

/// sh:105 — `$(print -D -- <val>)`. `print -D` performs directory-name
/// abbreviation (longest named-directory / `$HOME` prefix → `~`).
/// Approximated here with the `$HOME` prefix case, which covers the
/// common path; named-directory abbreviation is not reproduced.
fn dir_abbrev(val: &str) -> String {
    if let Some(home) = getsparam("HOME") {
        if !home.is_empty() {
            if val == home {
                return s("~");
            }
            let with_slash = format!("{}/", home);
            if let Some(rest) = val.strip_prefix(&with_slash) {
                return format!("~/{}", rest);
            }
        }
    }
    val.to_string()
}

/// `_subscript` — `-subscript-` context: complete inside `${var[…]}`.
pub fn _subscript(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_subscript");
    // sh:5  [[ $ISUFFIX = *\]* ]] || osuf=\]
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let mut osuf: String = if isuffix.contains(']') {
        String::new()
    } else {
        s("]")
    };

    // sh:7-11  -q → compquote osuf; osuf+=' '; shift
    if args.first().map(|a| a == "-q").unwrap_or(false) {
        // `compquote osuf` quotes the value of the shell-local `osuf`
        // in place. Bridge through a same-named scratch param so the
        // real `bin_compquote` runs (no-op when the quote stack is
        // empty, matching the C guard).
        let _ = setsparam("osuf", &osuf);
        let _ = bin_compquote("compquote", &[s("osuf")], &make_ops(), 0);
        osuf = getsparam("osuf").unwrap_or_default();
        unsetparam("osuf");
        osuf.push(' ');
        // `shift` drops -q; the remaining positionals are unused below.
    }

    // sh:13  compset -P '\(([^\(\)]|\(*\))##\)' — strip subscript flags
    let _ = bin_compset(
        "compset",
        &[s("-P"), s("\\(([^\\(\\)]|\\(*\\))##\\)")],
        &make_ops(),
        0,
    );

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let buffer = getsparam("BUFFER").unwrap_or_default();
    let cursor: i64 = getsparam("CURSOR")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // sh:21-24  dynamic-name expansion at `~[`.
    //   pos = CURSOR+1; while pos>1 && BUFFER[pos-1] != '[' : pos--
    //   then BUFFER[1,pos-1] must match `(|*[[:space:]:=]##)~[`.
    //   Byte-indexed (subscript context is ASCII in practice).
    {
        let bytes = buffer.as_bytes();
        let mut pos: i64 = cursor + 1;
        while pos > 1 && bytes.get((pos - 2) as usize).copied() != Some(b'[') {
            pos -= 1;
        }
        if pos >= 2 {
            let end = (pos - 1) as usize; // BUFFER[1,pos-1] → bytes[0..end]
            if end <= buffer.len() {
                let segment = &buffer[..end];
                if let Some(before_bracket) = segment.strip_suffix('[') {
                    if let Some(before_tilde) = before_bracket.strip_suffix('~') {
                        // `~` at word start, or preceded by a run of
                        // whitespace / ':' / '='.
                        let ok = before_tilde.is_empty()
                            || before_tilde
                                .chars()
                                .last()
                                .map(|c| c.is_whitespace() || c == ':' || c == '=')
                                .unwrap_or(false);
                        if ok {
                            return dynamic_directory_name_byname();
                        }
                    }
                }
            }
        }
    }

    // sh:25-28  :class:  — character-class completion (literal compadd args)
    if prefix.starts_with(':') {
        return wanted_byname(&[
            s("characters"),
            s("expl"),
            s("character class"),
            s("compadd"),
            s("-p:"),
            s("-S"),
            s(":]"),
            s("alnum"),
            s("alpha"),
            s("ascii"),
            s("blank"),
            s("cntrl"),
            s("digit"),
            s("graph"),
            s("lower"),
            s("print"),
            s("punct"),
            s("space"),
            s("upper"),
            s("xdigit"),
            s("IFS"),
            s("IDENT"),
            s("IFSSPACE"),
            s("WORD"),
        ]);
    }

    let param = get_compstate_str("parameter").unwrap_or_default();

    // sh:29  elif compset -P '\('  — subscript-flag catalog via _values
    if bin_compset("compset", &[s("-P"), s("\\(")], &make_ops(), 0) == 0 {
        // sh:31  compset -S '\)*'
        let _ = bin_compset("compset", &[s("-S"), s("\\)*")], &make_ops(), 0);

        // sh:33  if [[ $PREFIX = (#b)*([bns])(?|)(*) ]]
        //   Greedy `*` places [bns] at the rightmost b/n/s; match[2]=d
        //   is the single following char (or empty); match[3]=v is the
        //   remainder.
        let prefix2 = getsparam("PREFIX").unwrap_or_default();
        let pchars: Vec<char> = prefix2.chars().collect();
        if let Some(i) = pchars
            .iter()
            .rposition(|&c| c == 'b' || c == 'n' || c == 's')
        {
            // sh:34  f=match[1] d=match[2] e=match[2] v=match[3]
            let f = pchars[i];
            let d: String = pchars.get(i + 1).map(|c| c.to_string()).unwrap_or_default();
            let mut e: String = d.clone();
            let v: String = if i + 2 <= pchars.len() {
                pchars[i + 2..].iter().collect()
            } else {
                String::new()
            };

            // sh:35  [[ $f = s && (Pt) != scalar* ]] && return 1
            if f == 's' && !param_type(&param).starts_with("scalar") {
                return 1;
            }

            // sh:36
            if d.is_empty() {
                // sh:37  _message -e delimiters 'delimiter'; return
                return message_byname(&[s("-e"), s("delimiters"), s("delimiter")]);
            } else {
                // sh:40-44  case $d
                match d.as_str() {
                    "(" => e = s(")"),
                    "[" => e = s("]"),
                    "{" => e = s("}"),
                    _ => {}
                }
                // sh:45  if [[ $v != *$e* ]]
                if !v.contains(&e) {
                    // sh:46-49  case $f
                    match f {
                        's' => {
                            // sh:47  _message 'separator string'
                            let _ = message_byname(&[s("separator string")]);
                        }
                        'b' | 'n' => {
                            // sh:48  [[ $v = <-># ]] && _message 'number' || return 1
                            //   <-># matches zero or more integers → all digits (or empty).
                            if v.chars().all(|c| c.is_ascii_digit()) {
                                let _ = message_byname(&[s("number")]);
                            } else {
                                return 1;
                            }
                        }
                        _ => {}
                    }
                    // sh:50  [[ -n $v && $SUFFIX$ISUFFIX != *$e* ]] && _message 'delimiter'
                    let suffix = getsparam("SUFFIX").unwrap_or_default();
                    let isuffix2 = getsparam("ISUFFIX").unwrap_or_default();
                    let combined = format!("{}{}", suffix, isuffix2);
                    if !v.is_empty() && !combined.contains(&e) {
                        let _ = message_byname(&[s("delimiter")]);
                    }
                    // sh:51  return 0
                    return 0;
                }
                // v contains e — delimiter closed; fall through to catalog.
            }
        }

        // sh:56-81  case (Pt) → build $flags
        //   assoc*)  assoc specs
        //   (|scalar*))  scalar specs ;&  (fallthrough) → array specs
        //   array*)  array specs
        let ptype = param_type(&param);
        let assoc_specs: [&str; 7] = [
            "(R k K i I)r[any one value matched by subscript as pattern]",
            "(r k K i I)R[all values matched by subscript as pattern]",
            "(r R K i I)k[any one value where subscript matched by key as pattern]",
            "(r R k i I)K[all values where subscript matched by key as pattern]",
            "(r R k K I)i[any one key matched by subscript as pattern]",
            "(r R k K i)I[all keys matched by subscript as pattern]",
            "e[interpret * or @ as a single key]",
        ];
        let scalar_specs: [&str; 4] = [
            "f[make subscripting work on lines of scalar]",
            "w[make subscripting work on words of scalar]",
            "s[specify word separator]",
            "p[recognise escape sequences in subsequent s flag]",
        ];
        let array_specs: [&str; 7] = [
            "e[interpret * or @ as a single key and use plain string matching]",
            "n[Nth lowest/highest index with i/I/r/R flag]",
            "b[begin with specified element]",
            "(r R k K i)I[highest index of value matched by subscript]",
            "(r R k K I)i[lowest index of value matched by subscript]",
            "(r k K i I)R[value matched by subscript at highest index]",
            "(R k K i I)r[value matched by subscript at lowest index]",
        ];

        let mut flags: Vec<String> = Vec::new();
        if ptype.starts_with("assoc") {
            flags.extend(assoc_specs.iter().map(|x| s(x)));
        } else if ptype.is_empty() || ptype.starts_with("scalar") {
            // (|scalar*)) … ;&  → scalar specs then fall through to array specs
            flags.extend(scalar_specs.iter().map(|x| s(x)));
            flags.extend(array_specs.iter().map(|x| s(x)));
        } else if ptype.starts_with("array") {
            flags.extend(array_specs.iter().map(|x| s(x)));
        }

        // sh:83  _values -s '' 'subscript flag' $flags
        let mut vargs: Vec<String> = vec![s("-s"), s(""), s("subscript flag")];
        vargs.extend(flags);
        // By NAME so `_values` gets its own `comp_wrapper` frame (c:1556):
        // it rewrites PREFIX/SUFFIX/IPREFIX and sets `compstate[restore]=''`
        // (`_values.rs:235-283`, `:388`), and the frame is what undoes both
        // instead of letting them leak into `_subscript`'s caller.
        return crate::compsys::ported::shared::call_compfn("_values", &vargs, || _values(&vargs));
    }

    // sh:84  elif (Pt) == assoc*  → association-key completion
    if param_type(&param).starts_with("assoc") {
        // sh:86-88  keys with special chars backslash-escaped, and a
        //   key that is exactly `*` or `@` rewritten as `(e)*` / `(e)@`.
        //   `${(@k)${(P)compstate[parameter]}}` — the key set. A user
        //   assoc's keys live in the parallel `paramtab_hashed_storage`
        //   map; a module-magic assoc (`commands`, `aliases`, …) has NO
        //   entry there and is enumerated through its `PARTAB` row's
        //   canonical `scanfn` instead (vm_helper.rs:5025
        //   `partab_scan_keys`, c:Src/Modules/parameter.c scanpm* family).
        //   Without the second source this array came back empty for
        //   every special.
        let mut keys: Vec<String> = Vec::new();
        let mut from_storage = false;
        if let Ok(tab) = crate::ported::params::paramtab_hashed_storage().lock() {
            if let Some(h) = tab.get(&param) {
                keys.extend(h.keys().cloned());
                from_storage = true;
            }
        }
        if !from_storage {
            if let Some(k) = crate::vm_helper::partab_scan_keys(&param) {
                keys = k;
            }
        }
        for k in keys.iter_mut() {
            let mut esc = String::with_capacity(k.len());
            for c in k.chars() {
                if matches!(c, '$' | '\\' | '[' | ']' | '(' | ')' | '{' | '}') {
                    esc.push('\\');
                }
                esc.push(c);
            }
            *k = esc;
        }
        for k in keys.iter_mut() {
            if k == "*" || k == "@" {
                *k = format!("(e){}", k);
            }
        }

        // sh:89  [[ "$RBUFFER" != (|\\)\]* ]] && suf="$osuf"
        let rbuffer = getsparam("RBUFFER").unwrap_or_default();
        let starts_bracket = rbuffer.starts_with(']') || rbuffer.starts_with("\\]");
        let suf: String = if !starts_bracket {
            osuf.clone()
        } else {
            String::new()
        };

        // sh:91-92  _wanted association-keys expl 'association key'
        //           compadd -Q -S "$suf" -a keys
        setaparam("keys", keys);
        let r = wanted_byname(&[
            s("association-keys"),
            s("expl"),
            s("association key"),
            s("compadd"),
            s("-Q"),
            s("-S"),
            suf,
            s("-a"),
            s("keys"),
        ]);
        unsetparam("keys");
        return r;
    }

    // sh:93  elif (Pt) == array*  → array-index completion
    if param_type(&param).starts_with("array") {
        // sh:94  local … ret=1
        let mut ret: i32 = 1;

        // sh:96  _tags indexes parameters
        let _ = tags_byname(&[s("indexes"), s("parameters")]);

        // sh:98  while _tags; do
        loop {
            if tags_byname(&[]) != 0 {
                break;
            }

            // sh:99  if _requested indexes; then
            if requested_byname(&[s("indexes")]) == 0 {
                // sh:100  ind=( {1..${#${(P)compstate[parameter]}}} )
                //   `getaparam` reads `pm.u_arr` only (params.rs:5275-5281),
                //   which is `None` on the `PM_SPECIAL` stub seeded for a
                //   `PARTAB_ARRAY` magic array (`dirstack`, `funcstack`,
                //   `reswords`, …) — those compute their elements in the
                //   row's whole-array getfn. Fall back to that dispatch
                //   (vm_helper.rs:4998 `partab_array_get`, c:Src/Modules/
                //   parameter.c:2239-2291) so the index list is non-empty.
                let arr = getaparam(&param)
                    .or_else(|| crate::vm_helper::partab_array_get(&param))
                    .unwrap_or_default();
                let n = arr.len();
                let ind: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
                setaparam("ind", ind.clone());

                // sh:101  zstyle -T ":completion:${curcontext}:indexes" verbose
                let curcontext = getsparam("curcontext").unwrap_or_default();
                let ctx = format!(":completion:{}:indexes", curcontext);
                let verbose = bin_zstyle(
                    "zstyle",
                    &[s("-T"), ctx.clone(), s("verbose")],
                    &make_ops(),
                    0,
                ) == 0;

                // sh:112 disp default empty
                let disp: Vec<String>;
                if verbose {
                    // sh:103-109  build the per-index display list
                    let pfx = getsparam("PREFIX").unwrap_or_default();
                    let sfx = getsparam("SUFFIX").unwrap_or_default();
                    let mut list: Vec<String> = Vec::new();
                    for (idx0, iv) in ind.iter().enumerate() {
                        if index_matches(iv, &pfx, &sfx) {
                            // sh:105  "${i}:$(print -D -- ${(P)…[$i]})"
                            let val = arr.get(idx0).cloned().unwrap_or_default();
                            list.push(format!("{}:{}", iv, dir_abbrev(&val)));
                        } else {
                            list.push(String::new());
                        }
                    }
                    // sh:110  zstyle -s … list-separator sep || sep=--
                    unsetparam("sep");
                    let got = bin_zstyle(
                        "zstyle",
                        &[s("-s"), ctx.clone(), s("list-separator"), s("sep")],
                        &make_ops(),
                        0,
                    );
                    let sep = if got == 0 {
                        getsparam("sep").unwrap_or_else(|| s("--"))
                    } else {
                        s("--")
                    };
                    // sh:111  zformat -a list " $sep " "$list[@]"
                    let mut zf: Vec<String> = vec![s("-a"), s("list"), format!(" {} ", sep)];
                    zf.extend(list);
                    let _ = bin_zformat("zformat", &zf, &make_ops(), 0);
                    // sh:112  disp=( -d list )
                    disp = vec![s("-d"), s("list")];
                } else {
                    disp = Vec::new();
                }

                // sh:117-123  compadd with -S '' or -S "$osuf" depending on
                //   whether RBUFFER already opens with `]` / `\]`.
                let rbuffer = getsparam("RBUFFER").unwrap_or_default();
                let starts_bracket = rbuffer.starts_with(']') || rbuffer.starts_with("\\]");
                let mut la: Vec<String> = vec![
                    s("-V"),
                    s("indexes"),
                    s("expl"),
                    s("array index"),
                    s("compadd"),
                    s("-S"),
                ];
                if starts_bracket {
                    la.push(s(""));
                } else {
                    la.push(osuf.clone());
                }
                la.extend(disp); // "$disp[@]"
                la.push(s("-a"));
                la.push(s("ind"));
                if all_labels_byname(&la) == 0 {
                    ret = 0;
                }
                unsetparam("list");
                unsetparam("sep");
            }

            // sh:125  _requested parameters && _parameters && ret=0
            if requested_byname(&[s("parameters")]) == 0 && call_parameters(&[]) == 0 {
                ret = 0;
            }

            // sh:127  (( ret )) || return 0
            if ret == 0 {
                unsetparam("ind");
                return 0;
            }
        }

        unsetparam("ind");
        // sh:130  return 1
        return 1;
    }

    // sh:132  else _dispatch -math- -math-
    dispatch_function_call("_dispatch", &[s("-math-"), s("-math-")]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_no_executor_falls_to_dispatch() {
        // Empty PREFIX / ISUFFIX / parameter → no branch matches;
        //   falls through to `_dispatch -math- -math-`, which returns
        //   1 without an executor (dispatch_function_call → None).
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("ISUFFIX", "");
        let _ = crate::ported::params::setsparam("BUFFER", "");
        let _ = crate::ported::params::setsparam("CURSOR", "0");
        crate::ported::zle::compcore::set_compstate_str("parameter", "");
        assert_eq!(_subscript(&[]), 1);
    }

    #[test]
    fn osuf_empty_when_isuffix_has_bracket() {
        // sh:5 — ISUFFIX containing `]` suppresses the `]` suffix.
        //   Exercised indirectly: with a `]` in ISUFFIX and PREFIX=`:`,
        //   the character-class branch fires (returns via _wanted).
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("ISUFFIX", "]rest");
        let _ = crate::ported::params::setsparam("PREFIX", ":");
        let _ = crate::ported::params::setsparam("BUFFER", "");
        let _ = crate::ported::params::setsparam("CURSOR", "0");
        // Without registered comptags, _wanted returns 1; we only
        //   assert we reached the class branch without panicking.
        let _ = _subscript(&[]);
    }

    #[test]
    fn param_type_classifies_scalar_array_assoc_unset() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("st_scalar");
        crate::ported::params::unsetparam("st_array");
        let _ = crate::ported::params::setsparam("st_scalar", "x");
        assert_eq!(param_type("st_scalar"), "scalar");
        setaparam("st_array", vec![s("a"), s("b")]);
        assert_eq!(param_type("st_array"), "array");
        assert_eq!(param_type("never_ever_set_qwerty"), "");
        assert_eq!(param_type(""), "");
    }

    #[test]
    fn param_type_classifies_module_magic_specials_from_flags() {
        // sh:84 / sh:93 — `${(Pt)${compstate[parameter]}}` must report
        //   `assoc*` for a PM_HASHED magic special and `array*` for a
        //   PM_ARRAY one. These names have no `paramtab_hashed_storage`
        //   entry and no `pm.u_arr`, so a value-based classification
        //   returned "" and `echo $commands[<TAB>` fell through to
        //   `_dispatch -math- -math-` instead of completing keys.
        let _g = crate::test_util::global_state_lock();
        // `init_partab_params` seeds the PROCESS-WIDE paramtab and the lock
        // only serializes access — it does not undo the mutation. Left in
        // place, the seeded `commands` stub changes what a later test in the
        // same process sees (`_x_color`'s `setaparam("commands", …)` then
        // reads it back), so this test passed alone and failed in a full run.
        // Snapshot the table and remove exactly what we added.
        let before: std::collections::HashSet<String> = crate::ported::params::paramtab()
            .read()
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        crate::vm_helper::init_partab_params();
        assert_eq!(param_type("commands"), "association");
        assert_eq!(param_type("aliases"), "association");
        assert_eq!(param_type("dirstack"), "array");
        assert_eq!(param_type("funcstack"), "array");
        assert_eq!(param_type("reswords"), "array");
        let added: Vec<String> = crate::ported::params::paramtab()
            .read()
            .map(|t| t.keys().filter(|k| !before.contains(*k)).cloned().collect())
            .unwrap_or_default();
        // Drop the seeded nodes straight out of the table. `unsetparam`
        // refuses PM_SPECIAL/PM_READONLY entries, which is exactly what these
        // magic rows are, so it leaves them behind and the next test in the
        // process still sees a `commands` stub.
        if let Ok(mut t) = crate::ported::params::paramtab().write() {
            for name in &added {
                t.remove(name);
            }
        }
    }

    #[test]
    fn index_matches_prefix_suffix_literal() {
        // sh:104 — literal ${PREFIX}*${SUFFIX} interpretation.
        assert!(index_matches("123", "1", "3"));
        assert!(index_matches("12", "1", "2"));
        assert!(!index_matches("12", "3", ""));
        assert!(index_matches("5", "", ""));
        // len guard: prefix+suffix longer than value → no match.
        assert!(!index_matches("1", "1", "1"));
    }

    #[test]
    fn dir_abbrev_replaces_home_prefix() {
        // sh:105 — print -D $HOME abbreviation approximation.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("HOME", "/home/u");
        assert_eq!(dir_abbrev("/home/u"), "~");
        assert_eq!(dir_abbrev("/home/u/x/y"), "~/x/y");
        assert_eq!(dir_abbrev("/etc/passwd"), "/etc/passwd");
    }
}
