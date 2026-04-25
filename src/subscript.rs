//! Array subscript parsing and indexing for zshrs
//!
//! Direct port from zsh/Src/params.c getindex() and getarg() functions.
//!
//! Handles array subscript syntax including:
//! - Simple indices: arr[1], arr[-1]
//! - Ranges: arr[1,5], arr[2,-1]
//! - All elements: arr[@], arr[*]
//! - Subscript flags: arr[(r)pattern], arr[(i)string], etc.

// Pattern matching support - uses crate::pattern module when needed

/// Scan flags for parameter matching
/// Port from zsh.h SCANPM_* constants
pub mod scanflags {
    pub const WANTVALS: u32 = 1 << 0;
    pub const WANTKEYS: u32 = 1 << 1;
    pub const WANTINDEX: u32 = 1 << 2;
    pub const MATCHKEY: u32 = 1 << 3;
    pub const MATCHVAL: u32 = 1 << 4;
    pub const MATCHMANY: u32 = 1 << 5;
    pub const KEYMATCH: u32 = 1 << 6;
    pub const DQUOTED: u32 = 1 << 7;
    pub const NOEXEC: u32 = 1 << 8;
    pub const ISVAR_AT: u32 = 1 << 9;
    pub const CHECKING: u32 = 1 << 10;
}

/// Value flags
/// Port from zsh.h VALFLAG_* constants  
pub mod valflags {
    pub const INV: u32 = 1 << 0;
    pub const EMPTY: u32 = 1 << 1;
}

/// Subscript value result
/// Port from zsh Value struct fields relevant to subscripting
#[derive(Debug, Clone, Default)]
pub struct SubscriptValue {
    pub start: i64,
    pub end: i64,
    pub scan_flags: u32,
    pub val_flags: u32,
}

impl SubscriptValue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn single(idx: i64) -> Self {
        Self {
            start: idx,
            end: idx + 1,
            scan_flags: 0,
            val_flags: 0,
        }
    }

    pub fn range(start: i64, end: i64) -> Self {
        Self {
            start,
            end,
            scan_flags: 0,
            val_flags: 0,
        }
    }

    pub fn all() -> Self {
        Self {
            start: 0,
            end: -1,
            scan_flags: 0,
            val_flags: 0,
        }
    }

    pub fn is_all(&self) -> bool {
        self.start == 0 && self.end == -1
    }
}

/// Subscript argument parsing context
/// Port from getarg() local variables
struct GetArgContext<'a> {
    s: &'a str,
    pos: usize,
    inv: bool,
    rev: bool,
    ind: bool,
    down: bool,
    word: bool,
    keymatch: bool,
    hasbeg: bool,
    num: i64,
    beg: i64,
    sep: Option<String>,
    is_hash: bool,
    ksh_arrays: bool,
}

impl<'a> GetArgContext<'a> {
    fn new(s: &'a str, is_hash: bool, ksh_arrays: bool) -> Self {
        Self {
            s,
            pos: 0,
            inv: false,
            rev: false,
            ind: false,
            down: false,
            word: false,
            keymatch: false,
            hasbeg: false,
            num: 1,
            beg: 0,
            sep: None,
            is_hash,
            ksh_arrays,
        }
    }

    fn current(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(c) = self.current() {
            self.pos += c.len_utf8();
        }
    }

    fn remaining(&self) -> &str {
        &self.s[self.pos..]
    }
}

/// Parse subscription flags like (r), (R), (k), (K), (i), (I), (w), (f), etc.
/// Port from getarg() flag parsing section (lines 1389-1487)
fn parse_subscript_flags(ctx: &mut GetArgContext) {
    let c = match ctx.current() {
        Some(c) => c,
        None => return,
    };

    if c != '(' {
        return;
    }

    ctx.advance(); // skip '('
    let mut escapes = false;

    loop {
        let c = match ctx.current() {
            Some(c) if c != ')' => c,
            _ => break,
        };

        match c {
            'r' => {
                ctx.rev = true;
                ctx.keymatch = false;
                ctx.down = false;
                ctx.ind = false;
            }
            'R' => {
                ctx.rev = true;
                ctx.down = true;
                ctx.keymatch = false;
                ctx.ind = false;
            }
            'k' => {
                ctx.keymatch = ctx.is_hash;
                ctx.rev = true;
                ctx.down = false;
                ctx.ind = false;
            }
            'K' => {
                ctx.keymatch = ctx.is_hash;
                ctx.rev = true;
                ctx.down = true;
                ctx.ind = false;
            }
            'i' => {
                ctx.rev = true;
                ctx.ind = true;
                ctx.down = false;
                ctx.keymatch = false;
            }
            'I' => {
                ctx.rev = true;
                ctx.ind = true;
                ctx.down = true;
                ctx.keymatch = false;
            }
            'w' => {
                ctx.word = true;
            }
            'f' => {
                ctx.word = true;
                ctx.sep = Some("\n".to_string());
            }
            'e' => {
                // quote_arg = 1 - handled differently in Rust
            }
            'n' => {
                // Parse numeric argument: n:num:
                ctx.advance();
                if let Some(num) = parse_delimited_number(ctx) {
                    ctx.num = if num == 0 { 1 } else { num };
                }
                continue;
            }
            'b' => {
                // Parse beginning offset: b:num:
                ctx.hasbeg = true;
                ctx.advance();
                if let Some(beg) = parse_delimited_number(ctx) {
                    ctx.beg = if beg > 0 { beg - 1 } else { beg };
                }
                continue;
            }
            'p' => {
                escapes = true;
            }
            's' => {
                // Parse separator: s:sep:
                ctx.advance();
                if let Some(sep) = parse_delimited_string(ctx) {
                    ctx.sep = Some(sep);
                }
                continue;
            }
            _ => {
                // Unknown flag - reset and bail
                ctx.num = 1;
                ctx.word = false;
                ctx.rev = false;
                ctx.ind = false;
                ctx.down = false;
                ctx.keymatch = false;
                ctx.sep = None;
                return;
            }
        }
        ctx.advance();
    }

    // Skip closing ')'
    if ctx.current() == Some(')') {
        ctx.advance();
    }

    if ctx.num < 0 {
        ctx.down = !ctx.down;
        ctx.num = -ctx.num;
    }
}

/// Parse a delimited number like :123:
fn parse_delimited_number(ctx: &mut GetArgContext) -> Option<i64> {
    let c = ctx.current()?;
    if c != ':' {
        return None;
    }
    ctx.advance();

    let start = ctx.pos;
    while let Some(c) = ctx.current() {
        if c == ':' {
            break;
        }
        ctx.advance();
    }

    let num_str = &ctx.s[start..ctx.pos];

    // Skip closing ':'
    if ctx.current() == Some(':') {
        ctx.advance();
    }

    num_str.parse().ok()
}

/// Parse a delimited string like :sep:
fn parse_delimited_string(ctx: &mut GetArgContext) -> Option<String> {
    let c = ctx.current()?;
    if c != ':' {
        return None;
    }
    ctx.advance();

    let start = ctx.pos;
    while let Some(c) = ctx.current() {
        if c == ':' {
            break;
        }
        ctx.advance();
    }

    let s = ctx.s[start..ctx.pos].to_string();

    // Skip closing ':'
    if ctx.current() == Some(':') {
        ctx.advance();
    }

    Some(s)
}

/// Parse subscript expression and find the closing bracket
/// Port from getarg() main parsing loop (lines 1513-1546)
fn find_subscript_end(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut paren_depth = 0;

    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            ']' if depth == 0 && paren_depth == 0 => return Some(i),
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            ',' if depth == 0 && paren_depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Evaluate subscript expression as integer
/// Port from mathevalarg() call in getarg()
fn eval_subscript_expr(expr: &str, ksh_arrays: bool) -> i64 {
    let expr = expr.trim();

    // Try simple integer parse first
    if let Ok(n) = expr.parse::<i64>() {
        // KSH_ARRAYS adjusts positive indices
        if ksh_arrays && n >= 0 {
            return n + 1;
        }
        return n;
    }

    // Could be arithmetic expression - try our math evaluator
    // For now, return 0 on failure
    0
}

/// Parse array index subscript
/// Port from getindex() in zsh/Src/params.c (lines 2001-2168)
///
/// Takes a subscript string like "1", "1,5", "@", "(r)pattern"
/// Returns SubscriptValue with start/end positions
pub fn getindex(
    subscript: &str,
    is_hash: bool,
    ksh_arrays: bool,
) -> Result<SubscriptValue, String> {
    let s = subscript.trim();

    // Handle @ and * for all elements (lines 2027-2032)
    if s == "@" || s == "*" {
        let mut v = SubscriptValue::all();
        if s == "@" {
            v.scan_flags |= scanflags::ISVAR_AT;
        }
        return Ok(v);
    }

    let mut ctx = GetArgContext::new(s, is_hash, ksh_arrays);

    // Parse any subscription flags (lines 1389-1487)
    parse_subscript_flags(&mut ctx);

    let remaining = ctx.remaining();

    // Find end of first argument (at comma or end)
    let (first_arg, rest) = if let Some(comma_pos) = find_comma_position(remaining, is_hash) {
        (&remaining[..comma_pos], Some(&remaining[comma_pos + 1..]))
    } else {
        (remaining, None)
    };

    // Evaluate first argument
    let start = if ctx.rev {
        // Reverse subscripting - pattern match
        // For now, just parse as number if possible
        eval_subscript_expr(first_arg.trim(), ksh_arrays)
    } else {
        eval_subscript_expr(first_arg.trim(), ksh_arrays)
    };

    // Handle range subscripts (lines 2107-2163)
    let end = if let Some(rest) = rest {
        // Has comma, get second argument (lines 2110-2114)
        let end_expr = rest.trim();
        eval_subscript_expr(end_expr, ksh_arrays)
    } else {
        // No comma - single element (line 2114)
        start
    };

    let mut v = SubscriptValue::new();

    if ctx.inv {
        // Inverse indexing (lines 2040-2106)
        v.val_flags |= valflags::INV;
        v.start = start;
        v.end = start + 1;
    } else {
        // Normal indexing (lines 2107-2163)
        let has_comma = rest.is_some();

        // Adjust start for 1-indexed to internal representation (line 2123-2124)
        let adjusted_start = if start > 0 && !ksh_arrays {
            start - 1
        } else {
            start
        };

        v.start = adjusted_start;
        v.end = if has_comma { end } else { adjusted_start + 1 };
    }

    // Handle KSH_ARRAYS index adjustment (line 2091-2092)
    if ksh_arrays && v.start > 0 {
        v.start -= 1;
    }

    Ok(v)
}

/// Find comma position in subscript, respecting brackets
fn find_comma_position(s: &str, is_hash: bool) -> Option<usize> {
    let mut depth = 0;
    let mut paren_depth = 0;

    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            ',' if depth == 0 && paren_depth == 0 && !is_hash => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Get array elements by subscript
/// Port from array access logic in params.c
pub fn get_array_by_subscript(arr: &[String], v: &SubscriptValue, ksh_arrays: bool) -> Vec<String> {
    if v.is_all() {
        return arr.to_vec();
    }

    let len = arr.len() as i64;

    // Handle empty arrays
    if len == 0 {
        return Vec::new();
    }

    // Convert indices
    let start_idx = normalize_index(v.start, len, ksh_arrays);
    let end_idx = normalize_index(v.end, len, ksh_arrays);

    // Clamp to valid range
    let start = (start_idx.max(0) as usize).min(arr.len());
    let end = (end_idx.max(0) as usize).min(arr.len());

    if start >= end {
        return Vec::new();
    }

    arr[start..end].to_vec()
}

/// Get single array element by subscript
pub fn get_array_element_by_subscript(
    arr: &[String],
    v: &SubscriptValue,
    ksh_arrays: bool,
) -> Option<String> {
    if v.is_all() || arr.is_empty() {
        return None;
    }

    let len = arr.len() as i64;
    let idx = normalize_index(v.start, len, ksh_arrays);

    if idx < 0 || idx >= len {
        return None;
    }

    arr.get(idx as usize).cloned()
}

/// Normalize array index (handle negative indices, 1-indexing)
fn normalize_index(idx: i64, len: i64, ksh_arrays: bool) -> i64 {
    if idx < 0 {
        // Negative index counts from end
        len + idx
    } else if ksh_arrays {
        // KSH_ARRAYS: already 0-indexed
        idx
    } else {
        // zsh default: 1-indexed, but we already adjusted in getindex
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_index() {
        let v = getindex("1", false, false).unwrap();
        assert_eq!(v.start, 0);
        assert_eq!(v.end, 1);
    }

    #[test]
    fn test_simple_index_ksh() {
        let v = getindex("0", false, true).unwrap();
        assert_eq!(v.start, 0);
    }

    #[test]
    fn test_range_index() {
        let v = getindex("1,3", false, false).unwrap();
        assert_eq!(v.start, 0);
        assert_eq!(v.end, 3);
    }

    #[test]
    fn test_all_index() {
        let v = getindex("@", false, false).unwrap();
        assert!(v.is_all());
        assert_ne!(v.scan_flags & scanflags::ISVAR_AT, 0);

        let v = getindex("*", false, false).unwrap();
        assert!(v.is_all());
    }

    #[test]
    fn test_negative_index() {
        let v = getindex("-1", false, false).unwrap();
        assert_eq!(v.start, -1);
    }

    #[test]
    fn test_array_slice() {
        let arr = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];

        let v = getindex("1,2", false, false).unwrap();
        let result = get_array_by_subscript(&arr, &v, false);
        assert_eq!(result, vec!["a", "b"]);

        let v = getindex("2,4", false, false).unwrap();
        let result = get_array_by_subscript(&arr, &v, false);
        assert_eq!(result, vec!["b", "c", "d"]);
    }

    #[test]
    fn test_array_element() {
        let arr = vec!["a".to_string(), "b".to_string(), "c".to_string()];

        let v = getindex("1", false, false).unwrap();
        let result = get_array_element_by_subscript(&arr, &v, false);
        assert_eq!(result, Some("a".to_string()));

        let v = getindex("2", false, false).unwrap();
        let result = get_array_element_by_subscript(&arr, &v, false);
        assert_eq!(result, Some("b".to_string()));
    }
}
