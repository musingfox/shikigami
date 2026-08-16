use std::path::PathBuf;

/// shikigami's own config dir — models/, hooks.ndjson, API key files.
/// Shape mirrors `sumvox::config_dir()`; HOME missing yields a relative path.
pub fn config_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".config")
        .join("shikigami")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_with_config_shikigami() {
        assert!(config_dir().ends_with(".config/shikigami"));
    }
}
