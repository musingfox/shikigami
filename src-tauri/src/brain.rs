// Brain: transcript -> short spoken reply, via Gemini / OpenAI / Anthropic.
// Provider picked by SHIKIGAMI_BRAIN env override, else first provider with a
// key available, cheapest first (gemini -> openai -> anthropic).
// Client shape (no_proxy + timeout) copied from SumVox — macOS CoreFoundation workaround.
// ponytail: requests/responses are serde_json::Value, no typed structs per vendor.

use std::fs;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

const SYSTEM_PROMPT: &str = "你是式神，使用者的桌面語音助理。用使用者說話的語言簡潔回答，最多兩句，純文字、不用 Markdown，內容要適合直接朗讀。";
const MAX_TOKENS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    OpenAi,
    Anthropic,
}

// cheap-first order for auto-detection
const PROVIDERS: [Provider; 3] = [Provider::Gemini, Provider::OpenAi, Provider::Anthropic];

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini-2.5-flash",
            Provider::OpenAi => "gpt-4.1-mini",
            Provider::Anthropic => "claude-haiku-4-5",
        }
    }

    /// (env var, key file under ~/.config/shikigami/)
    fn key_sources(self) -> (&'static str, &'static str) {
        match self {
            Provider::Gemini => ("GEMINI_API_KEY", "gemini_api_key"),
            Provider::OpenAi => ("OPENAI_API_KEY", "openai_api_key"),
            Provider::Anthropic => ("ANTHROPIC_API_KEY", "anthropic_api_key"),
        }
    }
}

pub fn resolve_api_key(
    provider: Provider,
    env: Option<&str>,
    file_content: Option<&str>,
) -> Result<String, String> {
    for candidate in [env, file_content].into_iter().flatten() {
        let t = candidate.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    let (env_var, file_name) = provider.key_sources();
    Err(format!(
        "{} API key not found. Set {} env or put key in ~/.config/shikigami/{}.",
        provider.name(),
        env_var,
        file_name
    ))
}

fn load_key(provider: Provider) -> Result<String, String> {
    let (env_var, file_name) = provider.key_sources();
    let env = std::env::var(env_var).ok();
    let file = std::env::var("HOME").ok().and_then(|h| {
        fs::read_to_string(format!("{}/.config/shikigami/{}", h, file_name)).ok()
    });
    resolve_api_key(provider, env.as_deref(), file.as_deref())
}

/// Pure selection logic: explicit override wins, else first provider (cheap-first)
/// whose key resolves. `available` mirrors PROVIDERS order.
pub fn pick_provider(explicit: Option<&str>, available: [bool; 3]) -> Result<Provider, String> {
    if let Some(name) = explicit {
        let t = name.trim().to_lowercase();
        if !t.is_empty() {
            return PROVIDERS
                .into_iter()
                .find(|p| p.name() == t)
                .ok_or_else(|| format!("unknown SHIKIGAMI_BRAIN provider: {} (gemini|openai|anthropic)", t));
        }
    }
    PROVIDERS
        .into_iter()
        .zip(available)
        .find_map(|(p, ok)| ok.then_some(p))
        .ok_or_else(|| {
            "no brain API key found. Set one of GEMINI_API_KEY / OPENAI_API_KEY / \
             ANTHROPIC_API_KEY (env or key file under ~/.config/shikigami/)."
                .to_string()
        })
}

fn detect() -> Result<(Provider, String), String> {
    let explicit = std::env::var("SHIKIGAMI_BRAIN").ok();
    let available = PROVIDERS.map(|p| load_key(p).is_ok());
    let provider = pick_provider(explicit.as_deref(), available)?;
    let key = load_key(provider)?;
    Ok((provider, key))
}

fn build_request(provider: Provider, transcript: &str) -> Value {
    match provider {
        Provider::Gemini => json!({
            "system_instruction": { "parts": [{ "text": SYSTEM_PROMPT }] },
            "contents": [{ "parts": [{ "text": transcript }] }],
            "generationConfig": { "maxOutputTokens": MAX_TOKENS },
        }),
        Provider::OpenAi => json!({
            "model": provider.model(),
            "max_completion_tokens": MAX_TOKENS,
            "messages": [
                { "role": "system", "content": SYSTEM_PROMPT },
                { "role": "user", "content": transcript },
            ],
        }),
        Provider::Anthropic => json!({
            "model": provider.model(),
            "max_tokens": MAX_TOKENS,
            "system": SYSTEM_PROMPT,
            "messages": [{ "role": "user", "content": transcript }],
        }),
    }
}

fn extract_reply(provider: Provider, v: &Value) -> Result<String, String> {
    let text: String = match provider {
        Provider::Gemini => v["candidates"][0]["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        Provider::OpenAi => v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        Provider::Anthropic => v["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
    };
    if text.trim().is_empty() {
        return Err(format!("empty reply from brain ({})", provider.name()));
    }
    Ok(text)
}

fn http_client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

pub async fn ask(transcript: &str) -> Result<String, String> {
    let (provider, key) = detect()?;
    let req = build_request(provider, transcript);

    let mut request = match provider {
        Provider::Gemini => http_client()
            .post(format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
                provider.model()
            ))
            .header("x-goog-api-key", &key),
        Provider::OpenAi => http_client()
            .post("https://api.openai.com/v1/chat/completions")
            .header("authorization", format!("Bearer {}", key)),
        Provider::Anthropic => http_client()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01"),
    };
    request = request.header("content-type", "application/json");

    let response = request
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("brain request failed: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("brain read body: {}", e))?;

    if !status.is_success() {
        return Err(format!("{} API {}: {}", provider.name(), status, body));
    }

    let parsed: Value =
        serde_json::from_str(&body).map_err(|e| format!("brain parse: {} body={}", e, body))?;

    extract_reply(provider, &parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ApiKeyResolution tests kept
    #[test]
    fn t1_env_takes_precedence() {
        assert_eq!(
            resolve_api_key(Provider::Anthropic, Some("sk-env"), Some("sk-file")).unwrap(),
            "sk-env"
        );
    }

    #[test]
    fn t2_file_trimmed_when_no_env() {
        assert_eq!(
            resolve_api_key(Provider::Anthropic, None, Some(" sk-file\n")).unwrap(),
            "sk-file"
        );
    }

    #[test]
    fn t3_empty_env_falls_to_file() {
        assert_eq!(
            resolve_api_key(Provider::Anthropic, Some(""), Some("sk-file")).unwrap(),
            "sk-file"
        );
    }

    #[test]
    fn t4_missing_both_errors_with_both_locations() {
        let err = resolve_api_key(Provider::Anthropic, None, None).unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY"));
        assert!(err.contains("anthropic_api_key"));
    }

    #[test]
    fn t4b_gemini_error_names_gemini_sources() {
        let err = resolve_api_key(Provider::Gemini, None, None).unwrap_err();
        assert!(err.contains("GEMINI_API_KEY"));
        assert!(err.contains("gemini_api_key"));
    }

    // provider selection
    #[test]
    fn p1_explicit_override_wins() {
        let p = pick_provider(Some("anthropic"), [true, true, true]).unwrap();
        assert_eq!(p, Provider::Anthropic);
    }

    #[test]
    fn p2_auto_picks_cheapest_available() {
        assert_eq!(pick_provider(None, [true, true, true]).unwrap(), Provider::Gemini);
        assert_eq!(pick_provider(None, [false, true, true]).unwrap(), Provider::OpenAi);
        assert_eq!(pick_provider(None, [false, false, true]).unwrap(), Provider::Anthropic);
    }

    #[test]
    fn p3_no_keys_errors_naming_all_envs() {
        let err = pick_provider(None, [false, false, false]).unwrap_err();
        assert!(err.contains("GEMINI_API_KEY"));
        assert!(err.contains("OPENAI_API_KEY"));
        assert!(err.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn p4_unknown_explicit_errors() {
        assert!(pick_provider(Some("groq"), [true, true, true]).is_err());
    }

    // BrainReply contract tests (request/response per provider)
    #[test]
    fn t1_anthropic_request_shape() {
        let v = build_request(Provider::Anthropic, "hi");
        assert_eq!(v["model"], "claude-haiku-4-5");
        assert_eq!(v["max_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
        assert!(!v["system"].as_str().unwrap().is_empty());
    }

    #[test]
    fn t1b_gemini_request_shape() {
        let v = build_request(Provider::Gemini, "hi");
        assert_eq!(v["contents"][0]["parts"][0]["text"], "hi");
        assert!(!v["system_instruction"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(v["generationConfig"]["maxOutputTokens"], 300);
    }

    #[test]
    fn t1c_openai_request_shape() {
        let v = build_request(Provider::OpenAi, "hi");
        assert_eq!(v["model"], "gpt-4.1-mini");
        assert_eq!(v["max_completion_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "hi");
    }

    #[test]
    fn t2_extract_anthropic_text() {
        let v: Value = serde_json::from_str(
            r#"{"content":[{"type":"text","text":"你好"}],"model":"m","usage":{"input_tokens":1,"output_tokens":1}}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(Provider::Anthropic, &v).unwrap(), "你好");
    }

    #[test]
    fn t2b_extract_gemini_text() {
        let v: Value = serde_json::from_str(
            r#"{"candidates":[{"content":{"parts":[{"text":"你"},{"text":"好"}],"role":"model"},"finishReason":"STOP"}]}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(Provider::Gemini, &v).unwrap(), "你好");
    }

    #[test]
    fn t2c_extract_openai_text() {
        let v: Value = serde_json::from_str(
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"你好"},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(Provider::OpenAi, &v).unwrap(), "你好");
    }

    #[test]
    fn t3_extract_anthropic_skips_thinking_and_joins_text() {
        let v: Value = serde_json::from_str(
            r#"{"content":[
                {"type":"thinking","thinking":"ignored"},
                {"type":"text","text":"a"},
                {"type":"text","text":"b"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(Provider::Anthropic, &v).unwrap(), "ab");
    }

    #[test]
    fn t4_extract_empty_content_err_names_provider() {
        let v: Value = serde_json::from_str(r#"{"content":[]}"#).unwrap();
        let err = extract_reply(Provider::Anthropic, &v).unwrap_err();
        assert!(err.contains("empty"));
        let v: Value = serde_json::from_str(r#"{"candidates":[]}"#).unwrap();
        assert!(extract_reply(Provider::Gemini, &v).unwrap_err().contains("gemini"));
        let v: Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert!(extract_reply(Provider::OpenAi, &v).unwrap_err().contains("openai"));
    }

    #[test]
    #[ignore]
    fn t5_real_api_with_chinese_word_answer() {
        // given #[ignore] real API: transcript "用一個字回答：好" -> expect non-empty reply (manual)
        // run: <PROVIDER>_API_KEY=... cargo test t5_real -- --ignored
    }
}
