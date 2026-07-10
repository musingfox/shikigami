// Api key resolution + Claude brain via Messages API.
// copy of client shape from SumVox (read-only): .no_proxy() + timeout for macOS workaround.
// ponytail: only reqwest+json + serde already present.

use std::fs;
use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};

const CONFIG_PATH: &str = ".config/shikigami/anthropic_api_key";
const ANTHROPIC_API_BASE: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MODEL: &str = "claude-haiku-4-5";
const SYSTEM_PROMPT: &str = "你是式神，使用者的桌面語音助理。用使用者說話的語言簡潔回答，最多兩句，純文字、不用 Markdown，內容要適合直接朗讀。";

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

// serde ignores unknown fields (model, usage, thinking) by default
#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
}

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

fn http_client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| Client::new())
}

fn build_request(transcript: &str) -> AnthropicRequest {
    AnthropicRequest {
        model: MODEL.to_string(),
        max_tokens: 300,
        messages: vec![Message {
            role: "user".to_string(),
            content: transcript.to_string(),
        }],
        system: Some(SYSTEM_PROMPT.to_string()),
    }
}

pub async fn ask_claude(transcript: &str) -> Result<String, String> {
    let api_key = load_api_key()?;
    let url = format!("{}/messages", ANTHROPIC_API_BASE);
    let req = build_request(transcript);

    let response = http_client()
        .post(&url)
        .header("x-api-key", &api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
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
        return Err(format!("Anthropic API {}: {}", status, body));
    }

    let parsed: AnthropicResponse =
        serde_json::from_str(&body).map_err(|e| format!("brain parse: {} body={}", e, body))?;

    extract_reply(parsed)
}

fn extract_reply(resp: AnthropicResponse) -> Result<String, String> {
    if resp.content.is_empty() {
        return Err("empty reply from brain".to_string());
    }
    let text: String = resp
        .content
        .iter()
        .filter_map(|c| match c.content_type.as_str() {
            "text" => c.text.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    if text.trim().is_empty() {
        return Err("empty reply from brain".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ApiKeyResolution tests kept
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

    // BrainReply contract tests (T1-T4 written first before body impl; T5 ignore manual)
    #[test]
    fn t1_serialize_request_for_hi() {
        let json = serde_json::to_string(&build_request("hi")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["model"], "claude-haiku-4-5");
        assert_eq!(v["max_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
        assert!(v.get("system").is_some() && !v["system"].as_str().unwrap().is_empty());
    }

    #[test]
    fn t2_extract_reply_simple_text() {
        let resp: AnthropicResponse = serde_json::from_str(
            r#"{"content":[{"type":"text","text":"你好"}],"model":"m","usage":{"input_tokens":1,"output_tokens":1}}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(resp).unwrap(), "你好");
    }

    #[test]
    fn t3_extract_reply_skips_thinking_and_joins_text() {
        let resp: AnthropicResponse = serde_json::from_str(
            r#"{"content":[
                {"type":"thinking","thinking":"ignored"},
                {"type":"text","text":"a"},
                {"type":"text","text":"b"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(extract_reply(resp).unwrap(), "ab");
    }

    #[test]
    fn t4_extract_reply_empty_content_err() {
        let resp: AnthropicResponse = serde_json::from_str(r#"{"content":[]}"#).unwrap();
        let err = extract_reply(resp).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    #[ignore]
    fn t5_real_api_with_chinese_word_answer() {
        // given #[ignore] real API: transcript "用一個字回答：好" -> expect non-empty reply (manual)
        // run: ANTHROPIC_API_KEY=sk-... cargo test --manifest-path src-tauri/Cargo.toml t5_real -- --ignored
    }
}
