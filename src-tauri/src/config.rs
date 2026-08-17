use std::path::PathBuf;

/// Moves the whole config root — models/, hooks.ndjson, API key files, memory —
/// somewhere else. Set it to run shikigami against a throwaway profile.
pub const CONFIG_DIR_ENV: &str = "SHIKIGAMI_CONFIG_DIR";

/// shikigami's own config dir — models/, hooks.ndjson, API key files.
/// `SHIKIGAMI_CONFIG_DIR` overrides it when set and non-empty; otherwise the
/// shape mirrors `sumvox::config_dir()`, HOME missing yields a relative path.
pub fn config_dir() -> PathBuf {
    match std::env::var(CONFIG_DIR_ENV) {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".config")
            .join("shikigami"),
    }
}

/// `CONFIG_DIR_ENV` is process-global, so every test that touches it — here or
/// in another module — takes the same lock and restores what it found.
#[cfg(test)]
pub(crate) mod test_override {
    use super::CONFIG_DIR_ENV;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) struct ConfigDirOverride {
        previous: Option<String>,
        _guard: MutexGuard<'static, ()>,
    }

    impl ConfigDirOverride {
        pub(crate) fn set(dir: &Path) -> Self {
            let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var(CONFIG_DIR_ENV).ok();
            std::env::set_var(CONFIG_DIR_ENV, dir);
            Self {
                previous,
                _guard: guard,
            }
        }

        /// Serialize against overriding tests without setting the variable.
        pub(crate) fn none() -> Self {
            let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var(CONFIG_DIR_ENV).ok();
            std::env::remove_var(CONFIG_DIR_ENV);
            Self {
                previous,
                _guard: guard,
            }
        }
    }

    impl Drop for ConfigDirOverride {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(CONFIG_DIR_ENV, value),
                None => std::env::remove_var(CONFIG_DIR_ENV),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_override::ConfigDirOverride;
    use super::*;

    #[test]
    fn ends_with_config_shikigami() {
        let _dir = ConfigDirOverride::none();
        assert!(config_dir().ends_with(".config/shikigami"));
    }

    #[test]
    fn override_env_replaces_the_whole_config_root() {
        let elsewhere = std::env::temp_dir().join("shikigami-config-override");
        let _dir = ConfigDirOverride::set(&elsewhere);
        assert_eq!(config_dir(), elsewhere);

        // an empty override is no override
        std::env::set_var(CONFIG_DIR_ENV, "");
        assert!(config_dir().ends_with(".config/shikigami"));
    }
}
