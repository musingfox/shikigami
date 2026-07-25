// Brain: transcript -> short spoken reply, via Gemini / OpenAI / Anthropic.
// Provider picked by SHIKIGAMI_BRAIN env override, else first provider with a
// key available, cheapest first (gemini -> openai -> anthropic).
// Client shape (no_proxy + timeout) copied from SumVox — macOS CoreFoundation workaround.
// ponytail: requests/responses are serde_json::Value, no typed structs per vendor.

use std::fs;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

use crate::events::AgentEntry;

const SYSTEM_PROMPT: &str = "你是式神，使用者的桌面語音助理。用使用者說話的語言簡潔回答，最多兩句，純文字、不用 Markdown，內容要適合直接朗讀。直接給答案，不要輸出思考過程、前言或自我說明。";
const MAX_TOKENS: u32 = 300;

// Short multi-turn memory (R2c): the most recent successful (user, assistant)
// pairs, provider-neutral, so a follow-up question can reference the prior
// answer. Process memory only — no fs/persistence, so it clears on restart.
// Mirrors herdr::ROSTER's static-Mutex convention.
const HISTORY_DEPTH: usize = 6;
static HISTORY: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Prior turns oldest-first, for placing before the current question in a
/// request. Never includes the in-flight turn (committed only after it lands).
fn history_snapshot() -> Vec<(String, String)> {
    HISTORY.lock().map(|h| h.clone()).unwrap_or_default()
}

/// Record one turn's outcome: append the complete (user, assistant) pair only
/// on Ok, so a failed brain call leaves no trace and the strict user/assistant
/// alternation never breaks. Oldest pairs beyond HISTORY_DEPTH are forgotten.
fn commit(user: &str, result: &Result<String, String>) {
    let Ok(assistant) = result else { return };
    let mut h = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
    h.push((user.to_string(), assistant.clone()));
    let len = h.len();
    if len > HISTORY_DEPTH {
        h.drain(0..len - HISTORY_DEPTH);
    }
}

/// What one brain reply amounts to: something to say out loud, or a request to
/// summon a new agent. `parse_action` is total — anything that is not a
/// well-formed summon instruction is speech, so a malformed reply is spoken,
/// never acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummonAction {
    Speak(String),
    Summon { project: String, task: String },
}

/// Strip a Markdown code fence some providers wrap JSON in, so the payload can
/// be parsed. Returns the input untouched when there is no fence.
fn strip_fence(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("```") else {
        return s;
    };
    let body = match rest.find('\n') {
        Some(i) => &rest[i + 1..], // drop the info string ("json", "", …)
        None => return s,
    };
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

/// Classify a brain reply. Only a JSON object with action="summon" and both a
/// non-empty project and task becomes a Summon; everything else — prose, broken
/// JSON, unknown actions, missing fields — is spoken verbatim.
pub fn parse_action(reply: &str) -> SummonAction {
    let speak = || SummonAction::Speak(reply.to_string());
    let Ok(v) = serde_json::from_str::<Value>(strip_fence(reply.trim())) else {
        return speak();
    };
    if v["action"] != "summon" {
        return speak();
    }
    let field = |k: &str| v[k].as_str().unwrap_or_default().trim().to_string();
    let (project, task) = (field("project"), field("task"));
    if project.is_empty() || task.is_empty() {
        return speak();
    }
    SummonAction::Summon { project, task }
}

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

/// Roster snapshot -> a model-readable Chinese text block. Empty roster states
/// plainly that nothing is observed, so the model never invents an agent.
fn render_roster(roster: &[AgentEntry]) -> String {
    if roster.is_empty() {
        return "目前沒有觀測到 agent。".to_string();
    }
    let mut out = String::from(
        "目前觀測到的 agent（狀態詞彙：idle 閒置｜working 工作中｜blocked 受阻｜done 完成｜unknown 未知）：",
    );
    for e in roster {
        out.push('\n');
        out.push_str(&format!("- {}（{}）", e.name, e.status));
        let mut detail = String::new();
        if !e.title.trim().is_empty() {
            detail.push_str(e.title.trim());
        }
        if !e.cwd.trim().is_empty() {
            if !detail.is_empty() {
                detail.push_str(" @ ");
            }
            detail.push_str(e.cwd.trim());
        }
        if !detail.is_empty() {
            out.push('：');
            out.push_str(&detail);
        }
    }
    out
}

/// Base persona + a live roster block, sent in the system position so the
/// model can name real working agents without the user transcript being touched.
fn system_prompt(roster: &[AgentEntry]) -> String {
    format!(
        "{}\n\n{}\n\n被問到 agent 的狀態或「誰在工作」時，只依上述名冊點名回答，不要臆測名冊未列出的 agent。使用者的輸入來自語音辨識，agent 名稱可能被辨識成發音相近的其他詞；遇到與名冊名稱發音或拼寫相近的詞，解讀為該 agent。",
        SYSTEM_PROMPT,
        render_roster(roster)
    )
}

/// Prior turns + the current question as strict user/assistant alternating
/// messages ({role, content}) — the Anthropic/OpenAI wire shape. The current
/// question is always the final `user` message.
fn chat_messages(history: &[(String, String)], transcript: &str) -> Vec<Value> {
    let mut msgs = Vec::with_capacity(history.len() * 2 + 1);
    for (u, a) in history {
        msgs.push(json!({ "role": "user", "content": u }));
        msgs.push(json!({ "role": "assistant", "content": a }));
    }
    msgs.push(json!({ "role": "user", "content": transcript }));
    msgs
}

/// Prior turns + the current question as Gemini `contents` — parts[].text with
/// an explicit role ("user"/"model") on every entry, single-turn included.
fn gemini_contents(history: &[(String, String)], transcript: &str) -> Vec<Value> {
    let mut contents = Vec::with_capacity(history.len() * 2 + 1);
    for (u, a) in history {
        contents.push(json!({ "role": "user", "parts": [{ "text": u }] }));
        contents.push(json!({ "role": "model", "parts": [{ "text": a }] }));
    }
    contents.push(json!({ "role": "user", "parts": [{ "text": transcript }] }));
    contents
}

fn build_request(
    provider: Provider,
    transcript: &str,
    roster: &[AgentEntry],
    history: &[(String, String)],
) -> Value {
    let system = system_prompt(roster);
    match provider {
        Provider::Gemini => json!({
            "system_instruction": { "parts": [{ "text": system }] },
            "contents": gemini_contents(history, transcript),
            "generationConfig": {
                "maxOutputTokens": MAX_TOKENS,
                // 2.5-flash thinks by default AND thinking tokens eat
                // maxOutputTokens — a spoken reply needs neither
                "thinkingConfig": { "thinkingBudget": 0 },
            },
        }),
        Provider::OpenAi => {
            let mut messages = vec![json!({ "role": "system", "content": system })];
            messages.extend(chat_messages(history, transcript));
            json!({
                "model": provider.model(),
                "max_completion_tokens": MAX_TOKENS,
                "messages": messages,
            })
        }
        Provider::Anthropic => json!({
            "model": provider.model(),
            "max_tokens": MAX_TOKENS,
            "system": system,
            "messages": chat_messages(history, transcript),
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
                    .filter(|p| p["thought"] != true) // skip thought-summary parts
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

pub async fn ask(transcript: &str, roster: &[AgentEntry]) -> Result<String, String> {
    // Snapshot prior turns BEFORE the call; commit this turn AFTER it resolves.
    // The current question never leaks into the history it is sent with, and a
    // failed turn (any path) is discarded by commit's Ok-only append.
    let history = history_snapshot();
    let result = ask_once(transcript, roster, &history).await;
    commit(transcript, &result);
    result
}

async fn ask_once(
    transcript: &str,
    roster: &[AgentEntry],
    history: &[(String, String)],
) -> Result<String, String> {
    let (provider, key) = detect()?;
    let req = build_request(provider, transcript, roster, history);

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
        let v = build_request(Provider::Anthropic, "hi", &[], &[]);
        assert_eq!(v["model"], "claude-haiku-4-5");
        assert_eq!(v["max_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
        assert!(!v["system"].as_str().unwrap().is_empty());
    }

    #[test]
    fn t1b_gemini_request_shape() {
        let v = build_request(Provider::Gemini, "hi", &[], &[]);
        assert_eq!(v["contents"][0]["parts"][0]["text"], "hi");
        assert!(!v["system_instruction"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(v["generationConfig"]["maxOutputTokens"], 300);
        assert_eq!(v["generationConfig"]["thinkingConfig"]["thinkingBudget"], 0);
    }

    #[test]
    fn t1c_openai_request_shape() {
        let v = build_request(Provider::OpenAi, "hi", &[], &[]);
        assert_eq!(v["model"], "gpt-4.1-mini");
        assert_eq!(v["max_completion_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "hi");
    }

    // Roster test fixtures
    fn agent(name: &str, status: &str, title: &str, cwd: &str) -> AgentEntry {
        AgentEntry {
            id: format!("id-{name}"),
            name: name.to_string(),
            pane: "%1".to_string(),
            status: status.to_string(),
            title: title.to_string(),
            cwd: cwd.to_string(),
        }
    }

    // RosterPromptBlock contract
    #[test]
    fn rp1_nonempty_row_carries_all_fields() {
        let out = render_roster(&[agent("builder", "working", "設計 1:1:N 架構", "/x/shikigami")]);
        assert!(out.contains("builder"));
        assert!(out.contains("working"));
        assert!(out.contains("設計 1:1:N 架構"));
        assert!(out.contains("/x/shikigami"));
    }

    #[test]
    fn rp2_empty_roster_declares_no_observation() {
        let out = render_roster(&[]);
        assert!(out.contains("沒有觀測到"));
        assert!(!out.contains("- "));
    }

    #[test]
    fn rp3_nonempty_includes_status_vocabulary() {
        let out = render_roster(&[agent("x", "working", "", "")]);
        assert!(out.contains("idle"));
        assert!(out.contains("blocked"));
    }

    #[test]
    fn rp4_empty_fields_omitted_no_dangling() {
        let out = render_roster(&[agent("solo", "idle", "", "")]);
        // the agent row carries only name+status, with no dangling detail
        // separator ("：" / " @ ") left behind by the empty title and cwd.
        let row = out.lines().find(|l| l.starts_with("- ")).unwrap();
        assert_eq!(row, "- solo（idle）");
        assert!(!out.contains(" @ "));
    }

    #[test]
    fn rp5_system_prompt_instructs_stt_fuzzy_match() {
        let out = system_prompt(&[]);
        assert!(out.contains("語音辨識"));
        assert!(out.contains("相近"));
    }

    // BrainRequestCarriesRoster contract
    #[test]
    fn brc1_anthropic_system_carries_roster_user_clean() {
        let v = build_request(
            Provider::Anthropic,
            "hi",
            &[agent("builder", "working", "", "")],
            &[],
        );
        let sys = v["system"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert!(sys.contains("working"));
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn brc2_gemini_system_carries_roster_user_clean() {
        let v = build_request(
            Provider::Gemini,
            "hi",
            &[agent("builder", "working", "", "")],
            &[],
        );
        let sys = v["system_instruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert_eq!(v["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn brc3_openai_system_carries_roster_user_clean() {
        let v = build_request(
            Provider::OpenAi,
            "hi",
            &[agent("builder", "working", "", "")],
            &[],
        );
        assert_eq!(v["messages"][0]["role"], "system");
        let sys = v["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert_eq!(v["messages"][1]["content"], "hi");
    }

    #[test]
    fn brc4_empty_roster_says_no_observation() {
        let v = build_request(Provider::Anthropic, "hi", &[], &[]);
        assert!(v["system"].as_str().unwrap().contains("沒有觀測到"));
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
    fn t2d_extract_gemini_skips_thought_parts() {
        let v: Value = serde_json::from_str(
            r#"{"candidates":[{"content":{"parts":[
                {"text":"讓我想想…","thought":true},
                {"text":"天空是藍色的因為瑞利散射。"}
            ],"role":"model"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_reply(Provider::Gemini, &v).unwrap(),
            "天空是藍色的因為瑞利散射。"
        );
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

    // --- R2c: short multi-turn memory ---
    // These tests mutate the shared HISTORY static; serialize them and reset at
    // the top of each (the lock only serializes, it does not reset state).
    static HISTORY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_history() {
        *HISTORY.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
    }

    fn pair(u: &str, a: &str) -> (String, String) {
        (u.to_string(), a.to_string())
    }

    // ConversationHistoryRetention
    #[test]
    fn chr1_records_pairs_in_order() {
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        commit("q1", &Ok("a1".to_string()));
        commit("q2", &Ok("a2".to_string()));
        assert_eq!(history_snapshot(), vec![pair("q1", "a1"), pair("q2", "a2")]);
    }

    #[test]
    fn chr2_forgets_oldest_beyond_depth() {
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        for i in 1..=7 {
            commit(&format!("q{i}"), &Ok(format!("a{i}")));
        }
        let snap = history_snapshot();
        assert_eq!(snap.len(), 6);
        assert!(!snap.iter().any(|(u, _)| u == "q1"));
        assert_eq!(snap.first().unwrap().0, "q2");
        assert_eq!(snap.last().unwrap().0, "q7");
    }

    // FailedTurnDiscarded
    #[test]
    fn ftd1_err_leaves_history_empty() {
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        commit("q", &Err("boom".to_string()));
        assert!(history_snapshot().is_empty());
    }

    #[test]
    fn ftd2_ok_appends_pair() {
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        commit("q", &Ok("a".to_string()));
        assert_eq!(history_snapshot(), vec![pair("q", "a")]);
    }

    #[test]
    fn ftd3_failed_turn_does_not_break_alternation() {
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        commit("q1", &Ok("a1".to_string()));
        commit("q2", &Err("boom".to_string()));
        commit("q3", &Ok("a3".to_string()));
        assert_eq!(history_snapshot(), vec![pair("q1", "a1"), pair("q3", "a3")]);
    }

    // RequestCarriesHistory
    #[test]
    fn rch1_anthropic_prepends_history() {
        let history = vec![pair("早安", "你好")];
        let v = build_request(Provider::Anthropic, "誰在工作", &[], &history);
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0], json!({ "role": "user", "content": "早安" }));
        assert_eq!(m[1], json!({ "role": "assistant", "content": "你好" }));
        assert_eq!(m[2], json!({ "role": "user", "content": "誰在工作" }));
        assert!(!v["system"].as_str().unwrap().is_empty());
    }

    #[test]
    fn rch2_openai_roles_and_tail() {
        let history = vec![pair("早安", "你好")];
        let v = build_request(Provider::OpenAi, "誰在工作", &[], &history);
        let m = v["messages"].as_array().unwrap();
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert_eq!(m.last().unwrap()["content"], "誰在工作");
    }

    #[test]
    fn rch3_gemini_roles_and_texts() {
        let history = vec![pair("早安", "你好")];
        let v = build_request(Provider::Gemini, "誰在工作", &[], &history);
        let c = v["contents"].as_array().unwrap();
        let roles: Vec<&str> = c.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "model", "user"]);
        let texts: Vec<&str> =
            c.iter().map(|x| x["parts"][0]["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["早安", "你好", "誰在工作"]);
    }

    #[test]
    fn rch4_empty_history_matches_single_turn() {
        // Anthropic/OpenAI single-turn shape unchanged; Gemini gains role:user.
        let a = build_request(Provider::Anthropic, "hi", &[], &[]);
        assert_eq!(a["messages"].as_array().unwrap().len(), 1);
        assert_eq!(a["messages"][0], json!({ "role": "user", "content": "hi" }));

        let o = build_request(Provider::OpenAi, "hi", &[], &[]);
        let oroles: Vec<&str> = o["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["role"].as_str().unwrap())
            .collect();
        assert_eq!(oroles, ["system", "user"]);

        let g = build_request(Provider::Gemini, "hi", &[], &[]);
        assert_eq!(g["contents"].as_array().unwrap().len(), 1);
        assert_eq!(g["contents"][0]["role"], "user");
        assert_eq!(g["contents"][0]["parts"][0]["text"], "hi");
    }

    // RosterFreshNotInHistory
    #[test]
    fn rfh1_roster_in_system_history_user_verbatim() {
        let history = vec![pair("hi", "hello")];
        let v = build_request(
            Provider::Anthropic,
            "誰在工作",
            &[agent("builder", "working", "", "")],
            &history,
        );
        assert!(v["system"].as_str().unwrap().contains("builder"));
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m.len(), 3);
        let first = m[0]["content"].as_str().unwrap();
        assert_eq!(first, "hi");
        assert!(!first.contains("builder"));
        assert!(!first.contains("觀測到"));
    }

    #[test]
    fn rfh2_roster_fully_from_current_param() {
        let history = vec![pair("hi", "hello")];
        let v1 = build_request(
            Provider::Anthropic,
            "誰在工作",
            &[agent("builder", "working", "", "")],
            &history,
        );
        assert!(v1["system"].as_str().unwrap().contains("builder"));

        let v2 = build_request(
            Provider::Anthropic,
            "誰在工作",
            &[agent("reviewer", "idle", "", "")],
            &history,
        );
        let sys2 = v2["system"].as_str().unwrap();
        assert!(sys2.contains("reviewer"));
        assert!(!sys2.contains("builder"));
    }

    // --- R2d: voice summon ---
    // SummonActionParse
    const SUMMON_JSON: &str = r#"{"action":"summon","project":"cyris","task":"跑測試"}"#;

    fn summon(project: &str, task: &str) -> SummonAction {
        SummonAction::Summon {
            project: project.to_string(),
            task: task.to_string(),
        }
    }

    #[test]
    fn sap1_bare_json_parses_as_summon() {
        assert_eq!(parse_action(SUMMON_JSON), summon("cyris", "跑測試"));
    }

    #[test]
    fn sap2_fenced_json_parses_as_summon() {
        let fenced = format!("```json\n{}\n```", SUMMON_JSON);
        assert_eq!(parse_action(&fenced), summon("cyris", "跑測試"));
        let bare_fence = format!("```\n{}\n```", SUMMON_JSON);
        assert_eq!(parse_action(&bare_fence), summon("cyris", "跑測試"));
    }

    #[test]
    fn sap3_surrounding_whitespace_ignored() {
        let padded = format!("\n  {}  \n", SUMMON_JSON);
        assert_eq!(parse_action(&padded), summon("cyris", "跑測試"));
    }

    #[test]
    fn sap4_prose_is_speech() {
        assert_eq!(
            parse_action("好的，我會處理。"),
            SummonAction::Speak("好的，我會處理。".to_string())
        );
    }

    #[test]
    fn sap5_missing_task_falls_back_to_speech() {
        let s = r#"{"action":"summon","project":"cyris"}"#;
        assert_eq!(parse_action(s), SummonAction::Speak(s.to_string()));
    }

    #[test]
    fn sap6_empty_project_falls_back_to_speech() {
        let s = r#"{"action":"summon","project":"","task":"跑測試"}"#;
        assert_eq!(parse_action(s), SummonAction::Speak(s.to_string()));
    }

    #[test]
    fn sap7_broken_json_is_speech_verbatim() {
        assert_eq!(
            parse_action("{壞掉的json"),
            SummonAction::Speak("{壞掉的json".to_string())
        );
    }

    #[test]
    fn sap8_unknown_action_falls_back_to_speech() {
        let s = r#"{"action":"focus","project":"cyris","task":"跑測試"}"#;
        assert_eq!(parse_action(s), SummonAction::Speak(s.to_string()));
    }

    // SummonActionParse fuzzy criterion — live, stays #[ignore] (needs a key).
    #[test]
    #[ignore]
    fn sap9_live_summon_phrasing_parses_and_qa_does_not() {
        // given a real provider: 「幫我開 <專案> 做 <事>」 -> parse_action(reply)
        // is Summon; an ordinary question -> Speak (no false trigger).
        // --nocapture prints both replies as the transcript Review asks for.
        // run: <PROVIDER>_API_KEY=... cargo test sap9_live -- --ignored --nocapture
        let _g = HISTORY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_history();
        let summon = tauri::async_runtime::block_on(ask("幫我開 cyris 跑測試", &[])).unwrap();
        println!("summon turn reply: {summon}");
        assert!(matches!(parse_action(&summon), SummonAction::Summon { .. }));

        reset_history();
        let qa = tauri::async_runtime::block_on(ask("現在誰在工作", &[])).unwrap();
        println!("q&a turn reply: {qa}");
        assert!(matches!(parse_action(&qa), SummonAction::Speak(_)));
    }

    // MultiTurnLiveRecall — live end-to-end, stays #[ignore] (needs herdr + key).
    #[test]
    #[ignore]
    fn mtl1_live_two_turn_recall() {
        // given #[ignore] live: ask("現在誰在工作", roster) names X, then
        // ask("它在做什麼", same roster) -> both Ok non-empty and the 2nd answer
        // contains X's name — manual: 附一次逐字稿為證。
        // run: <PROVIDER>_API_KEY=... cargo test mtl1_live -- --ignored
    }

    #[test]
    #[ignore]
    fn t5_real_api_with_chinese_word_answer() {
        // given #[ignore] real API: transcript "用一個字回答：好" -> expect non-empty reply (manual)
        // run: <PROVIDER>_API_KEY=... cargo test t5_real -- --ignored
    }
}
