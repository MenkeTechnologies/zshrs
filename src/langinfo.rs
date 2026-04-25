//! Langinfo module - port of Modules/langinfo.c
//!
//! Provides access to locale information via the langinfo special parameter.

use std::collections::HashMap;

/// Available langinfo items
pub static LANGINFO_NAMES: &[&str] = &[
    "CODESET",
    "D_T_FMT",
    "D_FMT",
    "T_FMT",
    "RADIXCHAR",
    "THOUSEP",
    "YESEXPR",
    "NOEXPR",
    "CRNCYSTR",
    "ABDAY_1",
    "ABDAY_2",
    "ABDAY_3",
    "ABDAY_4",
    "ABDAY_5",
    "ABDAY_6",
    "ABDAY_7",
    "DAY_1",
    "DAY_2",
    "DAY_3",
    "DAY_4",
    "DAY_5",
    "DAY_6",
    "DAY_7",
    "ABMON_1",
    "ABMON_2",
    "ABMON_3",
    "ABMON_4",
    "ABMON_5",
    "ABMON_6",
    "ABMON_7",
    "ABMON_8",
    "ABMON_9",
    "ABMON_10",
    "ABMON_11",
    "ABMON_12",
    "MON_1",
    "MON_2",
    "MON_3",
    "MON_4",
    "MON_5",
    "MON_6",
    "MON_7",
    "MON_8",
    "MON_9",
    "MON_10",
    "MON_11",
    "MON_12",
    "T_FMT_AMPM",
    "AM_STR",
    "PM_STR",
    "ERA",
    "ERA_D_FMT",
    "ERA_D_T_FMT",
    "ERA_T_FMT",
    "ALT_DIGITS",
];

/// Get langinfo value by name
#[cfg(unix)]
pub fn get_langinfo(name: &str) -> Option<String> {
    use std::ffi::CStr;

    let item = match name {
        "CODESET" => libc::CODESET,
        "D_T_FMT" => libc::D_T_FMT,
        "D_FMT" => libc::D_FMT,
        "T_FMT" => libc::T_FMT,
        "RADIXCHAR" => libc::RADIXCHAR,
        "THOUSEP" => libc::THOUSEP,
        "YESEXPR" => libc::YESEXPR,
        "NOEXPR" => libc::NOEXPR,
        #[cfg(target_os = "linux")]
        "CRNCYSTR" => libc::CRNCYSTR,
        "ABDAY_1" => libc::ABDAY_1,
        "ABDAY_2" => libc::ABDAY_2,
        "ABDAY_3" => libc::ABDAY_3,
        "ABDAY_4" => libc::ABDAY_4,
        "ABDAY_5" => libc::ABDAY_5,
        "ABDAY_6" => libc::ABDAY_6,
        "ABDAY_7" => libc::ABDAY_7,
        "DAY_1" => libc::DAY_1,
        "DAY_2" => libc::DAY_2,
        "DAY_3" => libc::DAY_3,
        "DAY_4" => libc::DAY_4,
        "DAY_5" => libc::DAY_5,
        "DAY_6" => libc::DAY_6,
        "DAY_7" => libc::DAY_7,
        "ABMON_1" => libc::ABMON_1,
        "ABMON_2" => libc::ABMON_2,
        "ABMON_3" => libc::ABMON_3,
        "ABMON_4" => libc::ABMON_4,
        "ABMON_5" => libc::ABMON_5,
        "ABMON_6" => libc::ABMON_6,
        "ABMON_7" => libc::ABMON_7,
        "ABMON_8" => libc::ABMON_8,
        "ABMON_9" => libc::ABMON_9,
        "ABMON_10" => libc::ABMON_10,
        "ABMON_11" => libc::ABMON_11,
        "ABMON_12" => libc::ABMON_12,
        "MON_1" => libc::MON_1,
        "MON_2" => libc::MON_2,
        "MON_3" => libc::MON_3,
        "MON_4" => libc::MON_4,
        "MON_5" => libc::MON_5,
        "MON_6" => libc::MON_6,
        "MON_7" => libc::MON_7,
        "MON_8" => libc::MON_8,
        "MON_9" => libc::MON_9,
        "MON_10" => libc::MON_10,
        "MON_11" => libc::MON_11,
        "MON_12" => libc::MON_12,
        "T_FMT_AMPM" => libc::T_FMT_AMPM,
        "AM_STR" => libc::AM_STR,
        "PM_STR" => libc::PM_STR,
        "ERA" => libc::ERA,
        "ERA_D_FMT" => libc::ERA_D_FMT,
        "ERA_D_T_FMT" => libc::ERA_D_T_FMT,
        "ERA_T_FMT" => libc::ERA_T_FMT,
        "ALT_DIGITS" => libc::ALT_DIGITS,
        _ => return None,
    };

    unsafe {
        let ptr = libc::nl_langinfo(item);
        if ptr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

#[cfg(not(unix))]
pub fn get_langinfo(_name: &str) -> Option<String> {
    None
}

/// Get all langinfo values
pub fn get_all_langinfo() -> HashMap<String, String> {
    let mut result = HashMap::new();

    for name in LANGINFO_NAMES {
        if let Some(value) = get_langinfo(name) {
            result.insert(name.to_string(), value);
        }
    }

    result
}

/// Langinfo parameter interface
#[derive(Debug, Default)]
pub struct Langinfo;

impl Langinfo {
    pub fn new() -> Self {
        Self
    }

    pub fn get(&self, name: &str) -> Option<String> {
        get_langinfo(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (String, String)> {
        get_all_langinfo().into_iter()
    }

    pub fn to_hash(&self) -> HashMap<String, String> {
        get_all_langinfo()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_langinfo_names() {
        assert!(LANGINFO_NAMES.contains(&"CODESET"));
        assert!(LANGINFO_NAMES.contains(&"D_T_FMT"));
    }

    #[test]
    fn test_get_langinfo_codeset() {
        #[cfg(unix)]
        {
            let result = get_langinfo("CODESET");
            assert!(result.is_some());
        }
    }

    #[test]
    fn test_get_langinfo_invalid() {
        let result = get_langinfo("INVALID_NAME");
        assert!(result.is_none());
    }

    #[test]
    fn test_langinfo_struct() {
        let li = Langinfo::new();
        #[cfg(unix)]
        {
            assert!(li.get("CODESET").is_some());
        }
    }

    #[test]
    fn test_get_all_langinfo() {
        let all = get_all_langinfo();
        #[cfg(unix)]
        {
            assert!(!all.is_empty());
        }
    }
}
