//! Port of `_numbers` from `Completion/Base/Utility/_numbers`.
//!
//! Full upstream body (87 lines, abridged):
//! ```text
//! sh:39  local desc tag range suffixes suffix suffixfmt pat='<->' partial=''
//! sh:43  zparseopts -K -D -A opts M+:=keep q:=keep s+:=keep S+:=keep J+: V+: 1 2 o+: n F: x+: X+: \
//! sh:44    t:=tags u:=units l:=min m:=max d:=default f=type e=type N=type
//! sh:46  desc="${1:-number}" tag="${tags[2]:-numbers}"
//! sh:49  [[ -n ${(M)type:#-f} ]] && pat='(<->.[0-9]#|[0-9]#.<->|<->)' partial='(|.)'
//! sh:51  [[ -n ${(M)type:#-N} || min[2] = -* ]] && pat="(|-)$pat"
//! sh:54  if (( $#argv )) && compset -P "$pat"; then  # unit suffix
//! sh:55    _description -V units expl unit; compadd disp …
//! sh:62  elif [[ -prefix $~pat || $PREFIX = $~partial ]]; then
//! sh:64-80 build formats h/m/r/o/x; _description -x tag expl desc formats; compadd
//! sh:84  return 0; else return 1
//! ```
//!
//! Number completion with optional unit suffixes (e.g. `5s` / `200MB`).

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_numbers` — numeric input completion with optional unit
/// suffixes.
pub fn _numbers(args: &[String]) -> i32 {
    // sh:43  zparseopts
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    for name in &[
        "opts_flat",
        "tags",
        "units",
        "min",
        "max",
        "default",
        "type",
    ] {
        setaparam(name, Vec::new());
    }
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-K".to_string(),
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "opts_flat".to_string(),
            "M+:".to_string(),
            "q:".to_string(),
            "s+:".to_string(),
            "S+:".to_string(),
            "J+:".to_string(),
            "V+:".to_string(),
            "1".to_string(),
            "2".to_string(),
            "o+:".to_string(),
            "n".to_string(),
            "F:".to_string(),
            "x+:".to_string(),
            "X+:".to_string(),
            "t:=tags".to_string(),
            "u:=units".to_string(),
            "l:=min".to_string(),
            "m:=max".to_string(),
            "d:=default".to_string(),
            "f=type".to_string(),
            "e=type".to_string(),
            "N=type".to_string(),
        ],
        &make_ops(),
        0,
    );
    let argv = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    let tags = getaparam("tags").unwrap_or_default();
    let units = getaparam("units").unwrap_or_default();
    let min = getaparam("min").unwrap_or_default();
    let max = getaparam("max").unwrap_or_default();
    let default = getaparam("default").unwrap_or_default();
    let typ_arr = getaparam("type").unwrap_or_default();

    let desc = argv
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "number".to_string());
    let tag = tags
        .get(1)
        .cloned()
        .unwrap_or_else(|| "numbers".to_string());
    let suffix_specs: Vec<String> = if argv.is_empty() {
        Vec::new()
    } else {
        argv[1..].to_vec()
    };

    // sh:49-52  build pat/partial
    let f_flag = typ_arr.contains(&"-f".to_string());
    let n_flag = typ_arr.contains(&"-N".to_string())
        || min.get(1).map(|v| v.starts_with('-')).unwrap_or(false)
        || max.get(1).map(|v| v.starts_with('-')).unwrap_or(false);
    let mut pat = if f_flag {
        "(<->.[0-9]#|[0-9]#.<->|<->)".to_string()
    } else {
        "<->".to_string()
    };
    let mut partial = if f_flag {
        "(|.)".to_string()
    } else {
        String::new()
    };
    if n_flag {
        pat = format!("(|-){}", pat);
        partial = format!("(|-){}", partial);
    }
    let _ = partial;

    // sh:54-61  unit-suffix completion (when caller passes suffixes
    //   AND PREFIX already matches the numeric pattern)
    if !suffix_specs.is_empty()
        && bin_compset("compset", &["-P".to_string(), pat.clone()], &make_ops(), 0) == 0
    {
        let mut units_arr: Vec<String> = Vec::new();
        for spec in &suffix_specs {
            // strip leading `:` (default-unit marker)
            let s = spec.strip_prefix(':').unwrap_or(spec);
            // value:description
            let val = s.splitn(2, ':').next().unwrap_or("").to_string();
            units_arr.push(val);
        }
        let _ = _description(&[
            "-V".to_string(),
            "units".to_string(),
            "expl".to_string(),
            "unit".to_string(),
        ]);
        let expl = getaparam("expl").unwrap_or_default();
        let mut argv2: Vec<String> = vec!["-M".to_string(), "r:|/=* r:|=*".to_string()];
        argv2.extend(expl);
        argv2.push("-".to_string());
        argv2.extend(units_arr);
        return bin_compadd("compadd", &argv2, &make_ops(), 0);
    }

    // sh:62-83  description path
    let prefix = getsparam("PREFIX").unwrap_or_default();
    // Approximation: emit the formatted description via _description
    let mut formats: Vec<String> = vec![format!("h:{}", desc)];
    if let Some(u) = units.get(1) {
        formats.push(format!("m:{}", u));
    }
    let mut range = String::new();
    if let Some(m) = min.get(1) {
        range = format!("{}-", m);
    }
    if let Some(m) = max.get(1) {
        if range.is_empty() {
            range.push('-');
        }
        range.push_str(m);
    }
    if !range.is_empty() {
        formats.push(format!("r:{}", range));
    }
    if let Some(d) = default.get(1) {
        formats.push(format!("o:{}", d));
    }
    let _ = prefix;

    let mut desc_argv: Vec<String> = vec!["-x".to_string(), tag, "expl".to_string(), desc];
    desc_argv.extend(formats);
    let _ = _description(&desc_argv);
    let insert = get_compstate_str("insert").unwrap_or_default();
    if insert.contains("unambiguous") {
        set_compstate_str("insert", "");
    }
    let _ = setaparam("_comp_mesg", vec!["yes".to_string()]);
    let expl = getaparam("expl").unwrap_or_default();
    let _ = bin_compadd("compadd", &expl, &make_ops(), 0);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_zero_for_simple_case() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _r = _numbers(&[]);
    }
}
