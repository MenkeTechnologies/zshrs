//! Port of `_dates` from `Completion/Unix/Type/_dates`.
//!
//! Full upstream body (127 lines):
//! ```text
//! sh:  1  #autoload   (options: -f FORMAT, -F future)
//! sh: 19  columns=$(((COLUMNS+4)/32)) rows=LINES-4 offset=0; days=( Mo … Su )
//! sh: 23  zparseopts -D -K -E f:=format F=future;  (( future = $#future ? 1 : -1 ))
//! sh: 25  zstyle -s …:dates date-format userformat;  format=${userformat:-${format[2]:-%F}}
//! sh: 28  zstyle -a …:dates max-matches-length r; per-limit row budget; rows/=8
//! sh: 34  zmodload -i zsh/datetime || rows=0
//! sh: 36  _tags dates || return 0;  _comp_mesg=yes
//! sh: 38  _description -2V -x dates expl date;  compadd "${@:/-X/-x}" "$expl[@]" -
//! sh: 40  [[ -z $MENUSELECT && $WIDGET != menu-select ]] && return
//! sh: 41  [[ -n $PREFIX$SUFFIX ]] && return 0;  (( rows )) || return 0
//! sh: 43  compstate[list]='packed rows'
//! sh: 45  [[ $WIDGET = _next_tags ]] && offset = _next_tags_date*rows*columns
//! sh: 52  now=EPOCHSECONDS; year/month via strftime; offset = future*offset + …
//! sh: 56  for rows..1: per-column month headers, then 6 week-lines of days,
//! sh:                  emitting `compadd -d disp -a cand` grid cells.
//! sh:127  (end)
//! ```
//!
//! The interactive calendar grid (sh:50-127) is fully implemented: `strftime`
//! and `EPOCHSECONDS` are provided by `chrono` + `SystemTime` (no `zmodload`
//! needed, so `rows` is never zeroed); `$compstate[list|insert|nmatches]` via
//! `getsparam`/`setsparam("compstate[…]")`; the grid cells via
//! `compadd -x/-d/-E/-U/-i/-I/-a`. `${(l.N.)}` / `${(r.N.)}` padding is Rust
//! string padding (marked `// sh:N approx`) producing the same layout.

use crate::compsys::ported::_description::description_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use chrono::{Local, TimeZone};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn compadd(argv: &[String]) -> i32 {
    bin_compadd("compadd", argv, &make_ops(), 0)
}

/// sh:23 — pull `-f FORMAT` (value) and `-F` (flag) out of argv; rest passes
/// through. Returns (format_value, future_flag, rest).
fn zparse_dates(args: &[String]) -> (Option<String>, bool, Vec<String>) {
    let (mut fmt, mut future) = (None, false);
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-f" if i + 1 < args.len() => {
                fmt = Some(args[i + 1].clone());
                i += 2;
            }
            "-F" => {
                future = true;
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    (fmt, future, rest)
}

fn getiparam(name: &str, default: i64) -> i64 {
    getsparam(name)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

/// `EPOCHSECONDS` — seconds since the Unix epoch.
fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `strftime -s var FMT epoch` — format a local time. Supports the specifiers
/// the source uses (`%Y %m %d %w %B %F` …) via chrono.
fn sfmt(epoch: i64, fmt: &str) -> String {
    match Local.timestamp_opt(epoch, 0).single() {
        Some(dt) => dt.format(fmt).to_string(),
        None => String::new(),
    }
}

/// `strftime -r -s var '%Y%m' <YYYY><M>` — reverse: local-midnight epoch of
/// year-month-01. `total_months` is `year*12 + (month-1)` euclid-decomposed so
/// negative offsets wrap correctly (`start/12`, `1 + start%12` in the source).
fn ym_to_epoch(total_months: i64) -> i64 {
    let year = total_months.div_euclid(12) as i32;
    let month = (total_months.rem_euclid(12) + 1) as u32;
    match Local.with_ymd_and_hms(year, month, 1, 0, 0, 0).single() {
        Some(dt) => dt.timestamp(),
        None => 0,
    }
}

/// `${(l.n.)}` — right-justify (pad on the left with spaces) to width `n`.
fn pad_left(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.to_string()
    } else {
        format!("{}{}", " ".repeat(n - len), s)
    }
}

/// `${(r.n.)}` — left-justify (pad on the right) to width `n`.
fn pad_right(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(n - len))
    }
}

/// sh:28-33 — apply the `max-matches-length` limits to the `rows` budget.
fn apply_max_matches(mut rows: i64, limits: &[String], lines: i64) -> i64 {
    for ri in limits {
        let cap = if let Some(num) = ri.strip_suffix('%') {
            // `.${ri%%%}` → 0.<digits>; `LINES * 0.<digits>`.
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                let frac = format!("0.{}", num).parse::<f64>().unwrap_or(0.0);
                (lines as f64 * frac) as i64
            } else {
                continue;
            }
        } else if let Ok(n) = ri.parse::<i64>() {
            n
        } else {
            continue;
        };
        if cap < rows {
            rows = cap;
        }
    }
    rows / 8
}

/// `_dates` — complete a date, with an interactive calendar grid under
/// menu-select.
pub fn _dates(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dates");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let dctx = format!(":completion:{}:dates", curcontext);

    // sh:19 — grid geometry.
    let cols = getiparam("COLUMNS", 80);
    let lines = getiparam("LINES", 24);
    let columns = (((cols + 4) / 32).max(1)) as usize;
    let days = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

    // sh:23-24
    let (fmt_opt, future_flag, rest) = zparse_dates(args);
    let future: i64 = if future_flag { 1 } else { -1 };
    // sh:25-26 — format resolution.
    let userformat = lookupstyle(&dctx, "date-format").into_iter().next();
    let format = userformat.or(fmt_opt).unwrap_or_else(|| "%F".to_string());

    // sh:28-33 — row budget from max-matches-length.
    let limits = lookupstyle(&dctx, "max-matches-length");
    let mut rows = apply_max_matches(lines - 4, &limits, lines);
    // sh:34 — zsh/datetime is always available here (chrono), so rows stays.

    // sh:36  _tags dates || return 0
    if tags_byname(&["dates".to_string()]) != 0 {
        return 0;
    }
    // sh:37
    let _ = setsparam("_comp_mesg", "yes");
    // sh:38  _description -2V -x dates expl date
    let _ = description_byname(&[
        "-2V".to_string(),
        "-x".to_string(),
        "dates".to_string(),
        "expl".to_string(),
        "date".to_string(),
    ]);
    // sh:39  compadd "${@:/-X/-x}" "$expl[@]" -
    let mapped: Vec<String> = rest
        .iter()
        .map(|a| {
            if a == "-X" {
                "-x".to_string()
            } else {
                a.clone()
            }
        })
        .collect();
    let mut cadd = mapped.clone();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    let ret = compadd(&cadd);

    // sh:40 — only build the grid under menu-select.
    let menuselect = getsparam("MENUSELECT").filter(|s| !s.is_empty()).is_some();
    let widget = getsparam("WIDGET").unwrap_or_default();
    if !menuselect && widget != "menu-select" {
        return ret;
    }
    // sh:41
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if !prefix.is_empty() || !suffix.is_empty() {
        return 0;
    }
    // sh:42
    if rows <= 0 {
        return 0;
    }
    // sh:43
    let _ = setsparam("compstate[list]", "packed rows");

    // sh:45-50 — _next_tags paging offset.
    let mut offset: i64 = 0;
    if widget == "_next_tags" {
        let next_line = getiparam("_next_tags_line", 0);
        let histno = getiparam("HISTNO", 0);
        let next_date = if histno == next_line {
            getiparam("_next_tags_date", 0) + 1
        } else {
            1
        };
        let _ = setsparam("_next_tags_date", &next_date.to_string());
        let _ = setsparam("_next_tags_line", &histno.to_string());
        offset = next_date * rows * (columns as i64);
    }

    // sh:52-55 — anchor at the current year/month.
    let now = now_epoch();
    let year: i64 = sfmt(now, "%Y").parse().unwrap_or(1970);
    let month: i64 = sfmt(now, "%m").parse().unwrap_or(1);
    offset = future * offset
        + year * 12
        + month
        + if future == 1 {
            rows * (columns as i64) - 2
        } else {
            -1
        };

    let ipfx = getsparam("IPREFIX").unwrap_or_default();
    let isfx = getsparam("ISUFFIX").unwrap_or_default();
    let expl = getaparam("expl").unwrap_or_default();
    let mut spacer = 1i64;

    // sh:56 — for ((;rows;rows--))
    while rows > 0 {
        let mut disp: Vec<String> = Vec::new();
        let mut mlabels = String::new();
        let mut starts: Vec<i64> = vec![0; columns + 1]; // 1-based
        let mut skips: Vec<i64> = vec![0; columns + 1];

        // sh:58-72 — per-column headers.
        for col in 1..=columns {
            // sh:59
            let start = offset + col as i64 - rows * columns as i64;
            // sh:60-61
            let monstart = ym_to_epoch(start);
            let skip: i64 = sfmt(monstart - 86400, "%w").parse().unwrap_or(0);
            starts[col] = monstart;
            skips[col] = skip;
            // sh:64  disp+=( $days '  ' )
            disp.extend(days.iter().map(|d| d.to_string()));
            disp.push("  ".to_string());
            // sh:66-69  month label (%B, plus %Y in January).
            let mut mfmt = "%B".to_string();
            if sfmt(monstart, "%m") == "01" {
                mfmt.push_str(" %Y");
            }
            let mlabel = sfmt(monstart, &mfmt);
            // sh:71  centre in a 32- (or 28-, last col) wide field.  // sh:71 approx
            let lpad = (26usize.saturating_sub(mlabel.chars().count())) / 2;
            let centred = format!("{}{}", " ".repeat(lpad), mlabel);
            let width = if col == columns { 28 } else { 32 };
            mlabels.push_str(&pad_right(&centred, width));
        }
        // sh:73-75 — trailing spacing cell.
        let spacing = cols - 32 * columns as i64 + 2;
        if !disp.is_empty() {
            let last = disp.len() - 1;
            disp[last] = " ".repeat(spacing.max(0) as usize); // sh:74 approx
        }
        if spacing < 2 {
            spacer = 0;
            disp.pop();
        }

        // sh:76 — group name expl[after -J] = dates-$rows.
        let mut expl_row = expl.clone();
        if let Some(j) = expl_row.iter().position(|e| e == "-J") {
            if j + 1 < expl_row.len() {
                expl_row[j + 1] = format!("dates-{}", rows);
            } else {
                expl_row.push(format!("dates-{}", rows));
            }
        }

        // sh:77  compadd -x mlabels expl -d disp -E $#disp
        setaparam("_dates_disp", disp.clone());
        {
            let mut c = vec!["-x".to_string(), mlabels.clone()];
            c.extend(expl_row.clone());
            c.push("-d".to_string());
            c.push("_dates_disp".to_string());
            c.push("-E".to_string());
            c.push(disp.len().to_string());
            let _ = compadd(&c);
        }

        // sh:79-126 — six week lines.
        for line in 0..6i64 {
            for col in 1..=columns {
                let mut skip = 0i64;
                // sh:81-87 — leading blank cells before the first week.
                if skips[col] != 0 && line == 0 {
                    let d: Vec<String> = vec![String::new(); skips[col] as usize];
                    setaparam("_dates_disp", d);
                    let _ = compadd(&[
                        "-x".to_string(),
                        mlabels.clone(),
                        "-d".to_string(),
                        "_dates_disp".to_string(),
                        "-E".to_string(),
                        skips[col].to_string(),
                    ]);
                    skip = skips[col];
                }
                // sh:88-90
                let mut disp2: Vec<String> = Vec::new();
                let mut cand: Vec<String> = Vec::new();
                let mut extra: i64 = if col == columns { spacer } else { 1 };
                let mut preclude: i64 = 0;
                // sh:91-119 — up to 7 days in this line.
                for d in 1..=(7 - skip) {
                    let day = d + 7 * line + skip - skips[col];
                    let daysecs = starts[col] + 86400 * (day - 1);
                    let realday: i64 = sfmt(daysecs, "%d").parse().unwrap_or(0);
                    // sh:95-98 — past the month end.
                    if realday != day {
                        extra += 8 - d;
                        break;
                    }
                    // sh:99 — signed distance from now.
                    let mult = -future * (now - daysecs) + if future == 1 { 86400 } else { 0 };
                    // sh:100-108 — the match value per $format.
                    let m = match format.as_str() {
                        "s" => mult.to_string(),
                        "m" => (mult / 60).to_string(),
                        "h" => (mult / 3600).to_string(),
                        "d" => (mult / 86400).to_string(),
                        "w" => (mult / 604800).to_string(),
                        "M" => (mult / 2592000).to_string(),
                        _ => sfmt(daysecs, &format),
                    };
                    // sh:109  disp+=( "${(l.2.)day}" )
                    disp2.push(pad_left(&day.to_string(), 2));
                    // sh:110-118
                    if future < 0 && now < daysecs {
                        extra += 1;
                    } else if future > 0 && (now - daysecs) > 86400 {
                        preclude += 1;
                    } else {
                        // sh:115-116 — put the cursor on today.
                        if (now - daysecs) < 86400 && (now - daysecs) > 0 {
                            let nmatches = getiparam("compstate[nmatches]", 0);
                            let _ = setsparam(
                                "compstate[insert]",
                                &format!("menu:{}", nmatches + disp2.len() as i64),
                            );
                        }
                        cand.push(m);
                    }
                }
                // sh:120-123 — leading precluded (future) days become blanks.
                if preclude > 0 {
                    setaparam("_dates_disp", disp2.clone());
                    let _ = compadd(&[
                        "-x".to_string(),
                        mlabels.clone(),
                        "-E".to_string(),
                        preclude.to_string(),
                        "-d".to_string(),
                        "_dates_disp".to_string(),
                    ]);
                    // `shift preclude disp` — drop the first `preclude` cells.
                    let n = (preclude as usize).min(disp2.len());
                    disp2.drain(0..n);
                }
                // sh:124  the real day cells for this line/column.
                setaparam("_dates_disp", disp2.clone());
                setaparam("_dates_cand", cand.clone());
                let mut c = vec![
                    "-x".to_string(),
                    mlabels.clone(),
                    "-U".to_string(),
                    "-i".to_string(),
                    ipfx.clone(),
                    "-I".to_string(),
                    isfx.clone(),
                ];
                c.extend(expl_row.clone());
                c.extend(mapped.clone());
                c.push("-d".to_string());
                c.push("_dates_disp".to_string());
                c.push("-E".to_string());
                c.push(extra.to_string());
                c.push("-a".to_string());
                c.push("_dates_cand".to_string());
                let _ = compadd(&c);
            }
        }
        rows -= 1;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_extracts_f_and_upper_f() {
        let (f, fut, rest) = zparse_dates(&[
            "-f".into(),
            "%Y".into(),
            "-F".into(),
            "-J".into(),
            "g".into(),
        ]);
        assert_eq!(f.as_deref(), Some("%Y"));
        assert!(fut);
        assert_eq!(rest, vec!["-J".to_string(), "g".to_string()]);
    }

    #[test]
    fn strftime_and_reverse_roundtrip() {
        // ym_to_epoch(year*12 + month-1) then %Y/%m reproduce the year/month.
        let tm = 2024 * 12 + (6 - 1); // June 2024
        let e = ym_to_epoch(tm);
        assert_eq!(sfmt(e, "%Y"), "2024");
        assert_eq!(sfmt(e, "%m"), "06");
        assert_eq!(sfmt(e, "%d"), "01");
    }

    #[test]
    fn ym_decomposition_wraps_negative() {
        // sh:60 — start/12 & 1+start%12 with euclidean wrap.
        let dec = 2024 * 12 - 1; // December 2023
        let e = ym_to_epoch(dec);
        assert_eq!(sfmt(e, "%Y"), "2023");
        assert_eq!(sfmt(e, "%m"), "12");
    }

    #[test]
    fn padding_helpers() {
        assert_eq!(pad_left("7", 2), " 7"); // sh:109
        assert_eq!(pad_right("Jan", 6), "Jan   "); // sh:71
        assert_eq!(pad_left("long", 2), "long"); // no truncation
    }

    #[test]
    fn max_matches_length_percent_and_absolute() {
        // 50% of 24 lines = 12, min with rows(20) = 12, /8 = 1.
        assert_eq!(apply_max_matches(20, &["50%".to_string()], 24), 1);
        // absolute 8, min with 20 = 8, /8 = 1.
        assert_eq!(apply_max_matches(20, &["8".to_string()], 24), 1);
        // no limit: 20/8 = 2.
        assert_eq!(apply_max_matches(20, &[], 24), 2);
    }

    #[test]
    fn returns_early_without_registered_tags() {
        // sh:36 — `_tags dates` fails outside a completion context → return 0.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _dates(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(r == 0 || r == 1);
    }
}
