//! Named directory hash table for zshrs
//!
//! Direct port from zsh/Src/hashnameddir.c
//!
//! Provides a hash table for named directories (~name expansion).

use std::collections::HashMap;

/// Flags for named directory entries
pub const ND_USERNAME: u32 = 1; // Entry from passwd database

/// A named directory entry
#[derive(Clone, Debug)]
pub struct NamedDir {
    pub name: String,
    pub dir: String,
    pub flags: u32,
    pub diff: i32, // strlen(dir) - strlen(name)
}

/// Named directory hash table
pub struct NamedDirTable {
    table: HashMap<String, NamedDir>,
    all_users_added: bool,
    finddir_cache: Option<(String, String)>,
}

impl Default for NamedDirTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NamedDirTable {
    pub fn new() -> Self {
        NamedDirTable {
            table: HashMap::with_capacity(201),
            all_users_added: false,
            finddir_cache: None,
        }
    }

    /// Clear the table
    pub fn clear(&mut self) {
        self.table.clear();
        self.all_users_added = false;
        self.finddir_cache = None;
    }

    /// Add a named directory entry
    pub fn add(&mut self, name: &str, dir: &str, flags: u32) {
        let diff = dir.len() as i32 - name.len() as i32;
        self.finddir_cache = None;

        self.table.insert(
            name.to_string(),
            NamedDir {
                name: name.to_string(),
                dir: dir.to_string(),
                flags,
                diff,
            },
        );
    }

    /// Add a user directory (from passwd database)
    pub fn add_user(&mut self, username: &str, homedir: &str, check_first: bool) {
        if check_first && self.table.contains_key(username) {
            return;
        }
        self.add(username, homedir, ND_USERNAME);
    }

    /// Get a named directory entry
    pub fn get(&self, name: &str) -> Option<&NamedDir> {
        self.table.get(name)
    }

    /// Remove a named directory entry
    pub fn remove(&mut self, name: &str) -> Option<NamedDir> {
        let result = self.table.remove(name);
        if result.is_some() {
            self.finddir_cache = None;
        }
        result
    }

    /// Check if a name exists
    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Fill table with all users from passwd database
    #[cfg(unix)]
    pub fn fill_from_passwd(&mut self) {
        if self.all_users_added {
            return;
        }

        // Try to use passwd database
        #[cfg(feature = "passwd")]
        {
            use std::ffi::CStr;
            unsafe {
                libc::setpwent();
                loop {
                    let pw = libc::getpwent();
                    if pw.is_null() {
                        break;
                    }
                    let name = CStr::from_ptr((*pw).pw_name).to_string_lossy();
                    let dir = CStr::from_ptr((*pw).pw_dir).to_string_lossy();
                    self.add_user(&name, &dir, true);
                }
                libc::endpwent();
            }
        }

        self.all_users_added = true;
    }

    #[cfg(not(unix))]
    pub fn fill_from_passwd(&mut self) {
        self.all_users_added = true;
    }

    /// Find the best matching named directory for a path
    /// Returns (name, matched_portion) or None
    pub fn finddir(&mut self, path: &str) -> Option<(String, String)> {
        // Check cache
        if let Some((cached_path, cached_name)) = &self.finddir_cache {
            if path.starts_with(cached_path.as_str()) {
                return Some((cached_name.clone(), cached_path.clone()));
            }
        }

        let mut best_match: Option<(&str, &str, i32)> = None;

        for nd in self.table.values() {
            if path.starts_with(&nd.dir) {
                let dir_len = nd.dir.len();
                // Must match full directory component
                if dir_len == path.len() || path.as_bytes().get(dir_len) == Some(&b'/') {
                    // Pick the one with best diff (saves most characters)
                    if best_match.is_none() || nd.diff > best_match.as_ref().unwrap().2 {
                        best_match = Some((&nd.name, &nd.dir, nd.diff));
                    }
                }
            }
        }

        if let Some((name, dir, _)) = best_match {
            let result = (name.to_string(), dir.to_string());
            self.finddir_cache = Some((dir.to_string(), name.to_string()));
            Some(result)
        } else {
            None
        }
    }

    /// Iterate over all entries
    pub fn iter(&self) -> impl Iterator<Item = (&String, &NamedDir)> {
        self.table.iter()
    }

    /// Print a named directory entry
    pub fn print_entry(&self, name: &str, list_format: bool) -> Option<String> {
        let nd = self.get(name)?;

        if list_format {
            let prefix = if name.starts_with('-') {
                "hash -d -- "
            } else {
                "hash -d "
            };
            Some(format!(
                "{}{}={}",
                prefix,
                shell_quote(name),
                shell_quote(&nd.dir)
            ))
        } else {
            Some(format!("{}={}", shell_quote(name), shell_quote(&nd.dir)))
        }
    }
}

/// Quote a string for shell output
fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '/' || c == '.' || c == '-')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_table() {
        let table = NamedDirTable::new();
        assert!(table.is_empty());
    }

    #[test]
    fn test_add_get() {
        let mut table = NamedDirTable::new();
        table.add("proj", "/home/user/projects", 0);

        let entry = table.get("proj").unwrap();
        assert_eq!(entry.name, "proj");
        assert_eq!(entry.dir, "/home/user/projects");
    }

    #[test]
    fn test_remove() {
        let mut table = NamedDirTable::new();
        table.add("test", "/tmp/test", 0);

        assert!(table.contains("test"));
        table.remove("test");
        assert!(!table.contains("test"));
    }

    #[test]
    fn test_finddir() {
        let mut table = NamedDirTable::new();
        table.add("home", "/home/user", 0);
        table.add("proj", "/home/user/projects", 0);

        // Should find the more specific match
        let result = table.finddir("/home/user/projects/foo");
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "proj");
    }

    #[test]
    fn test_diff_calculation() {
        let mut table = NamedDirTable::new();
        table.add("p", "/home/user/projects", 0);

        let entry = table.get("p").unwrap();
        // diff = len("/home/user/projects") - len("p") = 19 - 1 = 18
        assert_eq!(entry.diff, 18);
    }

    #[test]
    fn test_print_entry() {
        let mut table = NamedDirTable::new();
        table.add("home", "/home/user", 0);

        let output = table.print_entry("home", false).unwrap();
        assert_eq!(output, "home=/home/user");

        let list_output = table.print_entry("home", true).unwrap();
        assert!(list_output.starts_with("hash -d "));
    }
}
