//! Port of `_baudrates` from `Completion/Unix/Type/_baudrates`.
//!
//! Full upstream body (78 lines, abridged — the head is a usage comment):
//! ```text
//! sh: 1  #autoload
//! sh:30  local tmp; local -a expl rates; local -A opts
//! sh:33  zparseopts -E -A opts u: l: f:
//! sh:47  zstyle -a ":completion:${curcontext}:" baud-rates rates ||
//! sh:48    rates=( 50 75 110 … 4000000 )            # default table
//! sh:51  zstyle -s …:baud-rates max-value tmp && opts[-u]=$tmp
//! sh:52  zstyle -s …:baud-rates min-value tmp && opts[-l]=$tmp
//! sh:53  zstyle -s …:baud-rates filter    tmp && opts[-f]=$tmp
//! sh:55  if (( ${+opts[-u]} )) || (( ${+opts[-l]} )); then
//! sh:57    min=${opts[-l]:-0}; max=${opts[-u]:-${${(On)rates}[1]}}
//! sh:59    rates=( ${(M)rates:#${~:-<$min-$max>}} )  # numeric range keep
//! sh:61  fi
//! sh:63  if (( ${+opts[-f]} )); then                # predicate filter
//! sh:66    for item; do ${opts[-f]} $item && rates+=( $item ); done
//! sh:69  fi
//! sh:76  _description -1V baud-rates expl 'baud rate'
//! sh:77  compadd "${argv[@]}" "$expl[@]" -- "${rates[@]}"
//! ```

use crate::compsys::ported::_description::description_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::getaparam;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:48 — default table of baud rates (0 omitted; == hang-up).
const DEFAULT_RATES: &[&str] = &[
    "50", "75", "110", "134", "150", "200", "300", "600", "1200", "1800", "2400", "4800", "9600",
    "19200", "38400", "57600", "115200", "230400", "460800", "500000", "576000", "921600",
    "1000000", "1152000", "1500000", "2000000", "2500000", "3000000", "3500000", "4000000",
];

/// sh:33 — pull `-u`/`-l`/`-f VALUE` out of argv (each takes an argument);
/// everything else stays in `rest` (the `"${argv[@]}"` passed to compadd).
fn zparse_ulf(args: &[String]) -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    let (mut u, mut l, mut f) = (None, None, None);
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let dst = match args[i].as_str() {
            "-u" => Some(&mut u),
            "-l" => Some(&mut l),
            "-f" => Some(&mut f),
            _ => None,
        };
        match dst {
            Some(slot) => {
                if i + 1 < args.len() {
                    *slot = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            None => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    (u, l, f, rest)
}

/// `_baudrates` — offer a (style-configurable, range/predicate-filtered)
/// list of serial baud rates.
pub fn _baudrates(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_baudrates");
    let curcontext = crate::ported::params::getsparam("curcontext").unwrap_or_default();
    let base = format!(":completion:{}:", curcontext);
    let bctx = format!(":completion:{}:baud-rates", curcontext);

    // sh:33
    let (mut opt_u, mut opt_l, opt_f_arg, rest) = zparse_ulf(args);

    // sh:47-48
    let styled = lookupstyle(&base, "baud-rates");
    let mut rates: Vec<String> = if styled.is_empty() {
        DEFAULT_RATES.iter().map(|s| s.to_string()).collect()
    } else {
        styled
    };

    // sh:51-53 — style overrides for max/min/filter.
    if let Some(v) = lookupstyle(&bctx, "max-value").into_iter().next() {
        opt_u = Some(v);
    }
    if let Some(v) = lookupstyle(&bctx, "min-value").into_iter().next() {
        opt_l = Some(v);
    }
    let opt_f = opt_f_arg.or_else(|| lookupstyle(&bctx, "filter").into_iter().next());

    // sh:55-61 — numeric range keep `<min-max>`.
    if opt_u.is_some() || opt_l.is_some() {
        let min: i64 = opt_l.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
        // max default = largest rate (${(On)rates}[1] = numeric-desc sort head).
        let max: i64 = opt_u
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                rates
                    .iter()
                    .filter_map(|r| r.parse::<i64>().ok())
                    .max()
                    .unwrap_or(0)
            });
        rates.retain(|r| {
            r.parse::<i64>()
                .map(|n| n >= min && n <= max)
                .unwrap_or(false)
        });
    }

    // sh:63-69 — predicate filter: keep items for which `$filter $item` succeeds.
    if let Some(filter) = opt_f {
        let candidates = std::mem::take(&mut rates);
        for item in candidates {
            if dispatch_function_call(&filter, std::slice::from_ref(&item)) == Some(0) {
                rates.push(item);
            }
        }
    }

    // sh:76  _description -1V baud-rates expl 'baud rate'
    let _ = description_byname(&[
        "-1V".to_string(),
        "baud-rates".to_string(),
        "expl".to_string(),
        "baud rate".to_string(),
    ]);
    // sh:77  compadd "${argv[@]}" "$expl[@]" -- "${rates[@]}"
    let mut cadd: Vec<String> = rest;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("--".to_string());
    cadd.extend(rates);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_pulls_ulf_leaving_rest() {
        let (u, l, f, rest) = zparse_ulf(&[
            "-u".into(),
            "9600".into(),
            "-J".into(),
            "grp".into(),
            "-l".into(),
            "1200".into(),
        ]);
        assert_eq!(u.as_deref(), Some("9600"));
        assert_eq!(l.as_deref(), Some("1200"));
        assert_eq!(f, None);
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_baudrates(&[]), 1);
    }
}
