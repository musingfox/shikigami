// Api key resolution: env first, then ~/.config/shikigami/anthropic_api_key (trimmed).
// Pure fn for unit tests; caller loads the strings.
// ponytail: no new deps, std fs only.

use std::fs;

const CONFIG_PATH: &str = ".config/shikigami/anthropic_api_key";

pub fn resolve_api_key(env: Option<&str>, file_content: Option<&str>) -> Result<String, String> {
    if let Some(e) = env {
        let t = e.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    if let Some(f) = file_content {
        let t = f.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    Err(format!(
        "Anthropic API key not found. Set ANTHROPIC_API_KEY env or put key in ~/{}.",
        CONFIG_PATH
    ))
}

pub fn load_api_key() -> Result<String, String> {
    let env = std::env::var("ANTHROPIC_API_KEY").ok();
    let file = home_config_path().and_then(|p| fs::read_to_string(p).ok());
    resolve_api_key(env.as_deref(), file.as_deref())
}

fn home_config_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| {
        let mut p = std::path::PathBuf::from(h);
        p.push(CONFIG_PATH);
        p
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_env_takes_precedence() {
        assert_eq!(
            resolve_api_key(Some("sk-env"), Some("sk-file")).unwrap(),
            "sk-env"
        );
    }

    #[test]
    fn t2_file_trimmed_when_no_env() {
        assert_eq!(
            resolve_api_key(None, Some(" sk-file\n")).unwrap(),
            "sk-file"
        );
    }

    #[test]
    fn t3_empty_env_falls_to_file() {
        assert_eq!(
            resolve_api_key(Some(""), Some("sk-file")).unwrap(),
            "sk-file"
        );
    }

    #[test]
    fn t4_missing_both_errors_with_both_locations() {
        let err = resolve_api_key(None, None).unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY"));
        assert!(err.contains("anthropic_api_key"));
    }
}
