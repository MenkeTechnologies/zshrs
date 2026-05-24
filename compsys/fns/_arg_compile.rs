//! Port of `_arg_compile` — compile argument specifications (internal).
//! Moved from `compsys/functions.rs`. Renamed from `arg_compile` to
//! mirror zsh shell function name `_arg_compile`. Also exposes
//! `CompiledArgSpec`, the compiled-spec struct.

/// _arg_compile - Compile argument specifications (internal)
pub fn _arg_compile(specs: &[String]) -> Vec<CompiledArgSpec> {
    specs
        .iter()
        .filter_map(|s| CompiledArgSpec::parse(s))
        .collect()
}

/// Compiled argument specification
#[derive(Clone, Debug)]
pub struct CompiledArgSpec {
    pub pattern: String,
    pub action: String,
    pub description: String,
}

impl CompiledArgSpec {
    pub fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.splitn(3, ':').collect();
        if parts.is_empty() {
            return None;
        }
        Some(Self {
            pattern: parts[0].to_string(),
            description: parts.get(1).unwrap_or(&"").to_string(),
            action: parts.get(2).unwrap_or(&"").to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiled_arg_spec() {
        let spec = CompiledArgSpec::parse("*:file:_files").unwrap();
        assert_eq!(spec.pattern, "*");
        assert_eq!(spec.description, "file");
        assert_eq!(spec.action, "_files");
    }
}
