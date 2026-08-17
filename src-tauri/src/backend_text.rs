// OpenAI + Anthropic adapter. Neither is given a tool declaration: they reach
// the same two actions through the prose summon convention in the system prompt,
// which is why `tools()` is empty and the loop can never run a tool for them —
// one step, one action, exactly today's behaviour.
//
// Every chat wire key — messages, choices, content blocks, max_tokens vs
// max_completion_tokens, the auth headers — stays inside this module.

use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::{text_action, Backend, StepAction, ToolResult, ToolSpec, TurnStart};

const MAX_TOKENS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatFlavor {
    OpenAi,
    Anthropic,
}

impl ChatFlavor {
    fn name(self) -> &'static str {
        match self {
            ChatFlavor::OpenAi => "openai",
            ChatFlavor::Anthropic => "anthropic",
        }
    }

    fn model(self) -> &'static str {
        match self {
            ChatFlavor::OpenAi => "gpt-4.1-mini",
            ChatFlavor::Anthropic => "claude-haiku-4-5",
        }
    }

    fn url(self) -> &'static str {
        match self {
            ChatFlavor::OpenAi => "https://api.openai.com/v1/chat/completions",
            ChatFlavor::Anthropic => "https://api.anthropic.com/v1/messages",
        }
    }

    fn authorize(self, request: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
        match self {
            ChatFlavor::OpenAi => request.header("authorization", format!("Bearer {}", key)),
            ChatFlavor::Anthropic => request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
        }
    }
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

pub(crate) fn chat_request(flavor: ChatFlavor, turn: &TurnStart) -> Value {
    match flavor {
        ChatFlavor::OpenAi => {
            let mut messages = vec![json!({ "role": "system", "content": turn.system })];
            messages.extend(chat_messages(&turn.history, &turn.transcript));
            json!({
                "model": flavor.model(),
                "max_completion_tokens": MAX_TOKENS,
                "messages": messages,
            })
        }
        ChatFlavor::Anthropic => json!({
            "model": flavor.model(),
            "max_tokens": MAX_TOKENS,
            "system": turn.system,
            "messages": chat_messages(&turn.history, &turn.transcript),
        }),
    }
}

/// Exactly one action per step: whatever the model said, classified. A model
/// answering with the prose summon object becomes a Summon here, so machine
/// payload is never read out loud.
pub(crate) fn chat_step(flavor: ChatFlavor, response: &Value) -> Result<Vec<StepAction>, String> {
    let text: String = match flavor {
        ChatFlavor::OpenAi => response["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        ChatFlavor::Anthropic => response["content"]
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
        return Err(format!("empty reply from brain ({})", flavor.name()));
    }
    Ok(vec![text_action(&text)])
}

pub(crate) struct TextBackend {
    flavor: ChatFlavor,
    key: String,
}

impl TextBackend {
    pub(crate) fn new(flavor: ChatFlavor, key: String) -> Self {
        Self { flavor, key }
    }
}

impl Backend for TextBackend {
    fn name(&self) -> &'static str {
        self.flavor.name()
    }

    /// No declared tools, so `system_prompt` grows no tool clause for these
    /// providers and the loop provably never runs a tool on their behalf.
    fn tools(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    async fn step(
        &mut self,
        turn: &TurnStart,
        _results: &[ToolResult],
        budget: Duration,
    ) -> Result<Vec<StepAction>, String> {
        let request = chat_request(self.flavor, turn);
        let raw = crate::backend::send_json(
            self.flavor.authorize(
                crate::backend::http_client(budget).post(self.flavor.url()),
                &self.key,
            ),
            &request,
            self.flavor.name(),
        )
        .await?;
        chat_step(self.flavor, &crate::backend::parse_body(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::{AgentDepth, PreciseDepth};
    use crate::events::AgentEntry;

    struct Tmp(std::path::PathBuf);

    impl Tmp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "shikigami-text-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn agent(name: &str, status: &str) -> AgentEntry {
        AgentEntry {
            id: format!("id-{name}"),
            name: name.to_string(),
            pane: "%1".to_string(),
            status: status.to_string(),
            title: String::new(),
            cwd: String::new(),
        }
    }

    fn turn_for(
        roster: &[AgentEntry],
        depth: &[AgentDepth],
        history: &[(String, String)],
        transcript: &str,
    ) -> TurnStart {
        TurnStart {
            system: crate::brain::system_prompt(roster, depth, None, &[]),
            history: history.to_vec(),
            transcript: transcript.to_string(),
        }
    }

    fn turn(transcript: &str) -> TurnStart {
        turn_for(&[], &[], &[], transcript)
    }

    fn pair(u: &str, a: &str) -> (String, String) {
        (u.to_string(), a.to_string())
    }

    // --- request shape (t1 / t1c / rch1 / rch2 / rch4 preserved) ---
    #[test]
    fn t1_anthropic_request_shape() {
        let v = chat_request(ChatFlavor::Anthropic, &turn("hi"));
        assert_eq!(v["model"], "claude-haiku-4-5");
        assert_eq!(v["max_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["messages"][0]["content"], "hi");
        assert!(!v["system"].as_str().unwrap().is_empty());
    }

    #[test]
    fn t1c_openai_request_shape() {
        let v = chat_request(ChatFlavor::OpenAi, &turn("hi"));
        assert_eq!(v["model"], "gpt-4.1-mini");
        assert_eq!(v["max_completion_tokens"], 300);
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "hi");
    }

    #[test]
    fn rch1_anthropic_prepends_history() {
        let history = vec![pair("早安", "你好")];
        let v = chat_request(ChatFlavor::Anthropic, &turn_for(&[], &[], &history, "誰在工作"));
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
        let v = chat_request(ChatFlavor::OpenAi, &turn_for(&[], &[], &history, "誰在工作"));
        let m = v["messages"].as_array().unwrap();
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert_eq!(m.last().unwrap()["content"], "誰在工作");
    }

    #[test]
    fn rch4_empty_history_matches_single_turn() {
        let a = chat_request(ChatFlavor::Anthropic, &turn("hi"));
        assert_eq!(a["messages"].as_array().unwrap().len(), 1);
        assert_eq!(a["messages"][0], json!({ "role": "user", "content": "hi" }));

        let o = chat_request(ChatFlavor::OpenAi, &turn("hi"));
        let oroles: Vec<&str> = o["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["role"].as_str().unwrap())
            .collect();
        assert_eq!(oroles, ["system", "user"]);
    }

    // --- roster / depth reach the system position only ---
    #[test]
    fn brc1_anthropic_system_carries_roster_user_clean() {
        let v = chat_request(
            ChatFlavor::Anthropic,
            &turn_for(&[agent("builder", "working")], &[], &[], "hi"),
        );
        let sys = v["system"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert!(sys.contains("working"));
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn brc3_openai_system_carries_roster_user_clean() {
        let v = chat_request(
            ChatFlavor::OpenAi,
            &turn_for(&[agent("builder", "working")], &[], &[], "hi"),
        );
        assert_eq!(v["messages"][0]["role"], "system");
        let sys = v["messages"][0]["content"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert_eq!(v["messages"][1]["content"], "hi");
    }

    #[test]
    fn brc4_empty_roster_says_no_observation() {
        let v = chat_request(ChatFlavor::Anthropic, &turn("hi"));
        assert!(v["system"].as_str().unwrap().contains("沒有觀測到"));
    }

    #[test]
    fn adpb_t5_request_carries_depth_in_system_only() {
        let v = chat_request(
            ChatFlavor::Anthropic,
            &turn_for(
                &[agent("builder", "blocked")],
                &[AgentDepth {
                    pane: "%1".into(),
                    precise: Some(PreciseDepth {
                        label: "permission_prompt".into(),
                        detail: "Claude needs your permission".into(),
                    }),
                    screen: None,
                }],
                &[],
                "hi",
            ),
        );
        assert!(v["system"].as_str().unwrap().contains("Claude needs your permission"));
        assert_eq!(v["messages"][0]["content"], "hi");
    }

    #[test]
    fn rfh1_roster_in_system_history_user_verbatim() {
        let history = vec![pair("hi", "hello")];
        let v = chat_request(
            ChatFlavor::Anthropic,
            &turn_for(&[agent("builder", "working")], &[], &history, "誰在工作"),
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
        let v1 = chat_request(
            ChatFlavor::Anthropic,
            &turn_for(&[agent("builder", "working")], &[], &history, "誰在工作"),
        );
        assert!(v1["system"].as_str().unwrap().contains("builder"));

        let v2 = chat_request(
            ChatFlavor::Anthropic,
            &turn_for(&[agent("reviewer", "idle")], &[], &history, "誰在工作"),
        );
        let sys2 = v2["system"].as_str().unwrap();
        assert!(sys2.contains("reviewer"));
        assert!(!sys2.contains("builder"));
    }

    #[test]
    fn fact_rows_never_enter_anthropic_requests() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"verb\":\"inject\",\"text\":\"祕密指令ZZZ\"}\n{\"verb\":\"turn\",\"user\":\"q\",\"assistant\":\"a\"}\n",
        )
        .unwrap();
        let history = crate::memory::history_snapshot_in(&tmp.0);
        let request =
            chat_request(ChatFlavor::Anthropic, &turn_for(&[], &[], &history, "now")).to_string();
        assert!(request.contains("\"a\""));
        assert!(!request.contains("祕密指令ZZZ"));
        assert!(!request.contains("\"verb\""));
    }

    // --- response shape (t2 / t2c / t3 / t4 preserved) ---
    #[test]
    fn t2_extract_anthropic_text() {
        let v: Value = serde_json::from_str(
            r#"{"content":[{"type":"text","text":"你好"}],"model":"m","usage":{"input_tokens":1,"output_tokens":1}}"#,
        )
        .unwrap();
        assert_eq!(
            chat_step(ChatFlavor::Anthropic, &v).unwrap(),
            vec![StepAction::Speak("你好".into())]
        );
    }

    #[test]
    fn t2c_extract_openai_text() {
        let v: Value = serde_json::from_str(
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"你好"},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(
            chat_step(ChatFlavor::OpenAi, &v).unwrap(),
            vec![StepAction::Speak("你好".into())]
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
        assert_eq!(
            chat_step(ChatFlavor::Anthropic, &v).unwrap(),
            vec![StepAction::Speak("ab".into())]
        );
    }

    #[test]
    fn t4_extract_empty_content_err_names_provider() {
        let v: Value = serde_json::from_str(r#"{"content":[]}"#).unwrap();
        let err = chat_step(ChatFlavor::Anthropic, &v).unwrap_err();
        assert!(err.contains("empty"));
        assert!(err.contains("anthropic"));
        let v: Value = serde_json::from_str(r#"{"choices":[]}"#).unwrap();
        assert!(chat_step(ChatFlavor::OpenAi, &v).unwrap_err().contains("openai"));
    }

    // --- SpeakTotality on the text path ---
    #[test]
    fn a_summon_object_arriving_as_text_is_never_spoken() {
        let bare = r#"{"action":"summon","project":"cyris","task":"跑測試"}"#;
        let v = json!({ "content": [{ "type": "text", "text": bare }] });
        assert_eq!(
            chat_step(ChatFlavor::Anthropic, &v).unwrap(),
            vec![StepAction::Summon { project: "cyris".into(), task: "跑測試".into() }]
        );
    }

    // --- the loop cannot tell it is not talking to a tool-capable model ---
    #[test]
    fn a_text_backend_declares_no_tools() {
        assert!(TextBackend::new(ChatFlavor::OpenAi, "sk-x".into()).tools().is_empty());
        assert!(TextBackend::new(ChatFlavor::Anthropic, "sk-x".into()).tools().is_empty());
    }

    #[test]
    fn a_step_yields_exactly_one_action() {
        let v = json!({ "content": [{ "type": "text", "text": "誰都沒在工作" }] });
        assert_eq!(chat_step(ChatFlavor::Anthropic, &v).unwrap().len(), 1);
    }
}
