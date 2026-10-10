//! `iana_time_zone::get_timezone()` without CoreFoundation.
//!
//! The upstream crate asks `CFTimeZoneCopySystem` on Apple targets, which
//! makes dyld load and initialise CoreFoundation in every process that links
//! it. The system zone is what `/etc/localtime` points at, on macOS and
//! Linux alike (`.../zoneinfo/<Area>/<City>`), so read that link instead.
//! chrono only calls this when `TZ` is unset and treats an error as "no zone
//! name", the same outcome as upstream's own failure paths.

use std::fmt;

/// Why the system zone name could not be determined.
#[derive(Debug)]
pub enum GetTimezoneError {
    /// `/etc/localtime` is not a link into a zoneinfo tree.
    FailedParsingString,
    /// Reading `/etc/localtime` failed.
    IoError(std::io::Error),
    /// Kept for API parity with upstream; never produced here.
    OsError,
}

impl std::error::Error for GetTimezoneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GetTimezoneError::IoError(err) => Some(err),
            _ => None,
        }
    }
}

impl fmt::Display for GetTimezoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            GetTimezoneError::FailedParsingString => "GetTimezoneError::FailedParsingString",
            GetTimezoneError::IoError(err) => return err.fmt(f),
            GetTimezoneError::OsError => "OsError",
        })
    }
}

impl From<std::io::Error> for GetTimezoneError {
    fn from(orig: std::io::Error) -> Self {
        GetTimezoneError::IoError(orig)
    }
}

/// The IANA name of the system time zone, e.g. `America/New_York`.
pub fn get_timezone() -> Result<String, GetTimezoneError> {
    let link = std::fs::read_link("/etc/localtime")?;
    let link = link.to_str().ok_or(GetTimezoneError::FailedParsingString)?;
    match link.rsplit_once("zoneinfo/") {
        Some((_, name)) if !name.is_empty() => Ok(name.to_owned()),
        _ => Err(GetTimezoneError::FailedParsingString),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_has_no_zoneinfo_prefix() {
        if let Ok(name) = get_timezone() {
            assert!(!name.contains("zoneinfo"));
            assert!(!name.starts_with('/'));
        }
    }
}
