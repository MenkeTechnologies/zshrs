//! Port of `_ktrace_points` from `Completion/BSD/Type/_ktrace_points`.
//!
//! Full upstream body (51 lines, abridged — the head is a usage comment):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local points=( 'c[trace system calls]' 'i[trace I/O]'
//! sh: 4    'n[trace namei translations]' 's[trace signal processing]'
//! sh: 5    'u[trace user data]' '+[trace the default points]' )
//! sh:12  case $OSTYPE in
//! sh:13    dragonfly*|freebsd*|netbsd*)
//! sh:15      points+=( 'w[context switches]' )
//! sh:17      ;|                                   # test next pattern too
//! sh:18    freebsd*|openbsd*)
//! sh:20      points+=( 't[trace various structures]' )
//! sh:22      ;|                                   # test next pattern too
//! sh:23    freebsd*)
//! sh:25      points+=( 'f[trace page faults]'
//! sh:26                'p[trace capability check failures]'
//! sh:27                'y[trace sysctl(3) requests]' )
//! sh:29      ;;
//! sh:30    netbsd*)
//! sh:32      points+=( 'A[trace all tracepoints]' 'a[trace exec arguments]'
//! sh:34                'e[trace emulation changes]'
//! sh:35                'f[trace open file descriptors after exec]'
//! sh:36                'S[trace MIB access (sysctl)]'
//! sh:37                'v[trace exec environment]'
//! sh:38                '-[do not trace following trace points]' )
//! sh:40      ;;
//! sh:41    openbsd*)
//! sh:43      points+=( 'p[trace violation of pledge(2) restrictions]'
//! sh:45                'x[trace argument vector in execve(2)]'
//! sh:46                'X[trace environment in execve(2)]' )
//! sh:47      ;;
//! sh:48  esac
//! sh:50  _values -s '' 'ktrace point' $points
//! ```

use crate::compsys::ported::_values::_values;
use crate::ported::params::getsparam;

/// sh:3-48 — build the `$points` array. Pure/testable: `;|` in the zsh
/// `case` means "execute this clause, then keep testing the next pattern"
/// (as opposed to `;;` which stops), so `dragonfly*|freebsd*|netbsd*` and
/// `freebsd*|openbsd*` are independent additive guards, while the final
/// `freebsd*` / `netbsd*` / `openbsd*` triple is mutually exclusive (`;;`)
/// and those three prefixes never overlap, so an if/else-if chain is
/// equivalent to the zsh `;;` semantics there.
fn build_points(ostype: &str) -> Vec<String> {
    // sh:3-10 — base points, always offered.
    let mut points: Vec<String> = vec![
        "c[trace system calls]".to_string(),
        "i[trace I/O]".to_string(),
        "n[trace namei translations]".to_string(),
        "s[trace signal processing]".to_string(),
        "u[trace user data]".to_string(),
        "+[trace the default points]".to_string(),
    ];

    // sh:13-17  dragonfly*|freebsd*|netbsd*) points+=( w ) ;|
    if ostype.starts_with("dragonfly")
        || ostype.starts_with("freebsd")
        || ostype.starts_with("netbsd")
    {
        points.push("w[context switches]".to_string());
    }
    // sh:18-22  freebsd*|openbsd*) points+=( t ) ;|
    if ostype.starts_with("freebsd") || ostype.starts_with("openbsd") {
        points.push("t[trace various structures]".to_string());
    }
    // sh:23-47  freebsd*) … ;; / netbsd*) … ;; / openbsd*) … ;;
    if ostype.starts_with("freebsd") {
        points.push("f[trace page faults]".to_string());
        points.push("p[trace capability check failures]".to_string());
        points.push("y[trace sysctl(3) requests]".to_string());
    } else if ostype.starts_with("netbsd") {
        points.push("A[trace all tracepoints]".to_string());
        points.push("a[trace exec arguments]".to_string());
        points.push("e[trace emulation changes]".to_string());
        points.push("f[trace open file descriptors after exec]".to_string());
        points.push("S[trace MIB access (sysctl)]".to_string());
        points.push("v[trace exec environment]".to_string());
        points.push("-[do not trace following trace points]".to_string());
    } else if ostype.starts_with("openbsd") {
        points.push("p[trace violation of pledge(2) restrictions]".to_string());
        points.push("x[trace argument vector in execve(2)]".to_string());
        points.push("X[trace environment in execve(2)]".to_string());
    }

    points
}

/// `_ktrace_points` — offer ktrace(1)/`-t` trace-point letters, BSD-flavor
/// dependent (via `$OSTYPE`).
pub fn _ktrace_points(args: &[String]) -> i32 {
    // sh:12  case $OSTYPE in
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let points = build_points(&ostype);

    // sh:50  _values -s '' 'ktrace point' $points
    let mut vargs: Vec<String> = vec!["-s".to_string(), String::new(), "ktrace point".to_string()];
    vargs.extend(points);
    // sh: $@ is not consulted by this function body (`_values` is called
    // with a fixed argv, not `"${argv[@]}"`) — `args` is accepted per the
    // port convention but faithfully unused here, matching the shell.
    let _ = args;
    _values(&vargs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_points_only_for_unrecognized_ostype() {
        let points = build_points("linux-gnu");
        assert_eq!(
            points,
            vec![
                "c[trace system calls]",
                "i[trace I/O]",
                "n[trace namei translations]",
                "s[trace signal processing]",
                "u[trace user data]",
                "+[trace the default points]",
            ]
        );
    }

    #[test]
    fn dragonfly_gets_only_w() {
        let points = build_points("dragonfly");
        assert_eq!(points.len(), 7);
        assert_eq!(points[6], "w[context switches]");
    }

    #[test]
    fn freebsd_gets_w_t_and_freebsd_specific() {
        let points = build_points("freebsd11.0");
        let suffix = &points[6..];
        assert_eq!(
            suffix,
            &[
                "w[context switches]".to_string(),
                "t[trace various structures]".to_string(),
                "f[trace page faults]".to_string(),
                "p[trace capability check failures]".to_string(),
                "y[trace sysctl(3) requests]".to_string(),
            ]
        );
    }

    #[test]
    fn netbsd_gets_w_and_netbsd_specific_but_not_t() {
        let points = build_points("netbsd");
        let suffix = &points[6..];
        assert_eq!(
            suffix,
            &[
                "w[context switches]".to_string(),
                "A[trace all tracepoints]".to_string(),
                "a[trace exec arguments]".to_string(),
                "e[trace emulation changes]".to_string(),
                "f[trace open file descriptors after exec]".to_string(),
                "S[trace MIB access (sysctl)]".to_string(),
                "v[trace exec environment]".to_string(),
                "-[do not trace following trace points]".to_string(),
            ]
        );
    }

    #[test]
    fn openbsd_gets_t_and_openbsd_specific_but_not_w() {
        let points = build_points("openbsd");
        let suffix = &points[6..];
        assert_eq!(
            suffix,
            &[
                "t[trace various structures]".to_string(),
                "p[trace violation of pledge(2) restrictions]".to_string(),
                "x[trace argument vector in execve(2)]".to_string(),
                "X[trace environment in execve(2)]".to_string(),
            ]
        );
    }
}
