// Gemini adapter: native function calling on the endpoint, auth header and
// model the app already speaks (`v1beta/…:generateContent`). Every gemini wire
// key — contents, parts, functionCall, functionResponse, candidates,
// generationConfig, x-goog-api-key — stays inside this module.
//
// ponytail: this is Google's "Legacy REST API" shape, not /v1beta/interactions.
// Deliberate: one request key plus one response branch, and the durable rolling
// layer stays the sole authority on conversation history.

use std::time::Duration;

use serde_json::{json, Value};

use crate::backend::{
    step_action, text_action, Backend, StepAction, ToolResult, ToolSpec, TurnStart,
};

const NAME: &str = "gemini";
const MODEL: &str = "gemini-2.5-flash";
const MAX_TOKENS: u32 = 300;

/// 2.5-flash thinks by default AND thinking tokens eat maxOutputTokens — a
/// spoken reply needs neither. Named rather than inlined because whether 0 also
/// suppresses *function* calling on this model is only observable live: if no
/// functionCall ever arrives, this is the one number to flip.
pub(crate) const THINKING_BUDGET: i64 = 0;

pub(crate) fn gemini_url(model: &str) -> String {
    format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent")
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

/// One request for one step. `tail` is this turn's accumulated model/tool
/// exchange, appended after the history so the model continues from where it
/// stopped. An empty tool set produces exactly the request shipped before tools
/// existed — no `tools` key at all.
pub(crate) fn gemini_request(turn: &TurnStart, tools: &[ToolSpec], tail: &[Value]) -> Value {
    let mut contents = gemini_contents(&turn.history, &turn.transcript);
    contents.extend(tail.iter().cloned());
    let mut request = json!({
        "system_instruction": { "parts": [{ "text": turn.system }] },
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": MAX_TOKENS,
            "thinkingConfig": { "thinkingBudget": THINKING_BUDGET },
        },
    });
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.parameters,
                })
            })
            .collect();
        request["tools"] = json!([{ "functionDeclarations": declarations }]);
    }
    request
}

/// Every part of the reply, in wire order. Consecutive text parts are one
/// utterance (gemini splits prose across parts freely); thought summaries are
/// skipped; each functionCall becomes its own action, so a candidate carrying
/// text plus one or several calls loses nothing.
pub(crate) fn gemini_step(response: &Value) -> Result<Vec<StepAction>, String> {
    let mut actions = Vec::new();
    let mut said = String::new();
    let flush = |said: &mut String, actions: &mut Vec<StepAction>| {
        if !said.trim().is_empty() {
            actions.push(text_action(said));
        }
        said.clear();
    };
    if let Some(parts) = response["candidates"][0]["content"]["parts"].as_array() {
        for part in parts {
            if part["thought"] == true {
                continue; // thought-summary part, never spoken
            }
            if let Some(call) = part.get("functionCall") {
                flush(&mut said, &mut actions);
                actions.push(step_action(
                    call["name"].as_str().unwrap_or_default(),
                    &call["args"],
                    call["id"].as_str(),
                ));
                continue;
            }
            if let Some(text) = part["text"].as_str() {
                said.push_str(text);
            }
        }
    }
    flush(&mut said, &mut actions);
    if actions.is_empty() {
        return Err(format!("empty reply from brain ({NAME})"));
    }
    Ok(actions)
}

/// The two `contents` entries one round of tool use adds: the model's own turn
/// replayed verbatim — gemini rejects a follow-up whose prior content carries
/// unanswered calls — then one user entry answering every call. There is no
/// role:"tool" in this wire shape.
pub(crate) fn result_contents(model_content: &Value, results: &[ToolResult]) -> Vec<Value> {
    let parts: Vec<Value> = results
        .iter()
        .map(|result| {
            let mut response = json!({
                "name": result.call.name,
                "response": { "result": result.text },
            });
            if let Some(id) = &result.call.id {
                response["id"] = json!(id);
            }
            json!({ "functionResponse": response })
        })
        .collect();
    vec![model_content.clone(), json!({ "role": "user", "parts": parts })]
}

/// Stateful for one turn: it owns the provider-shaped tail so no raw gemini
/// `Value` ever crosses into the loop.
pub(crate) struct GeminiBackend {
    key: String,
    tail: Vec<Value>,
    last_content: Option<Value>,
    #[cfg(test)]
    steps: usize,
    #[cfg(test)]
    first_body: Option<String>,
}

impl GeminiBackend {
    pub(crate) fn new(key: String) -> Self {
        Self {
            key,
            tail: Vec::new(),
            last_content: None,
            #[cfg(test)]
            steps: 0,
            #[cfg(test)]
            first_body: None,
        }
    }

    /// How many requests this turn has cost — the live receipt's step count.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> usize {
        self.steps
    }

    /// The raw body of the FIRST response of this turn, kept so a live run can
    /// print it once: mixed text+functionCall parts and parallel calls are only
    /// observable in the real thing.
    #[cfg(test)]
    pub(crate) fn first_body(&self) -> Option<&str> {
        self.first_body.as_deref()
    }
}

impl Backend for GeminiBackend {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<ToolSpec> {
        crate::tools::specs()
    }

    async fn step(
        &mut self,
        turn: &TurnStart,
        results: &[ToolResult],
        budget: Duration,
    ) -> Result<Vec<StepAction>, String> {
        if !results.is_empty() {
            if let Some(content) = self.last_content.take() {
                self.tail.extend(result_contents(&content, results));
            }
        }
        let request = gemini_request(turn, &self.tools(), &self.tail);
        let raw = crate::backend::send_json(
            crate::backend::http_client(budget)
                .post(gemini_url(MODEL))
                .header("x-goog-api-key", &self.key),
            &request,
            NAME,
        )
        .await?;
        #[cfg(test)]
        {
            self.steps += 1;
            if self.first_body.is_none() {
                self.first_body = Some(raw.clone());
            }
        }
        let parsed = crate::backend::parse_body(&raw)?;
        let content = &parsed["candidates"][0]["content"];
        self.last_content = (!content.is_null()).then(|| content.clone());
        gemini_step(&parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::CallRef;
    use crate::events::AgentEntry;

    struct Tmp(std::path::PathBuf);

    impl Tmp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "shikigami-gemini-{}-{}",
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

    fn turn_for(roster: &[AgentEntry], history: &[(String, String)], transcript: &str) -> TurnStart {
        TurnStart {
            system: crate::brain::system_prompt(roster, &[], None, &[]),
            history: history.to_vec(),
            transcript: transcript.to_string(),
        }
    }

    fn turn(transcript: &str) -> TurnStart {
        turn_for(&[], &[], transcript)
    }

    fn pair(u: &str, a: &str) -> (String, String) {
        (u.to_string(), a.to_string())
    }

    fn parts(json_parts: Value) -> Value {
        json!({ "candidates": [{ "content": { "role": "model", "parts": json_parts } }] })
    }

    fn read_pane(agent: &str) -> StepAction {
        StepAction::ReadPane {
            call: CallRef { id: None, name: "read_pane".into() },
            agent: agent.into(),
        }
    }

    // --- GeminiToolDeclaration ---
    #[test]
    fn declared_tools_reach_the_request_as_function_declarations() {
        let v = gemini_request(&turn("hi"), &crate::tools::specs(), &[]);
        let declared = v["tools"][0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(
            declared.iter().map(|d| d["name"].as_str().unwrap()).collect::<Vec<_>>(),
            ["read_pane", "summon"]
        );
        let read_pane = &declared[0]["parameters"];
        assert_eq!(read_pane["type"], "OBJECT");
        assert_eq!(read_pane["properties"]["agent"]["type"], "STRING");
        assert_eq!(read_pane["required"], json!(["agent"]));
    }

    #[test]
    fn t1b_gemini_request_shape() {
        let v = gemini_request(&turn("hi"), &crate::tools::specs(), &[]);
        assert_eq!(v["contents"][0]["parts"][0]["text"], "hi");
        assert!(!v["system_instruction"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(v["generationConfig"]["maxOutputTokens"], 300);
        assert_eq!(
            v["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            THINKING_BUDGET
        );
    }

    #[test]
    fn rch3_gemini_roles_and_texts() {
        let history = vec![pair("早安", "你好")];
        let v = gemini_request(&turn_for(&[], &history, "誰在工作"), &[], &[]);
        let c = v["contents"].as_array().unwrap();
        let roles: Vec<&str> = c.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "model", "user"]);
        let texts: Vec<&str> =
            c.iter().map(|x| x["parts"][0]["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["早安", "你好", "誰在工作"]);
        // an empty tool set produces exactly today's request
        assert!(v.get("tools").is_none());
    }

    #[test]
    fn rch4_empty_history_matches_single_turn() {
        let g = gemini_request(&turn("hi"), &[], &[]);
        assert_eq!(g["contents"].as_array().unwrap().len(), 1);
        assert_eq!(g["contents"][0]["role"], "user");
        assert_eq!(g["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn brc2_gemini_system_carries_roster_user_clean() {
        let v = gemini_request(&turn_for(&[agent("builder", "working")], &[], "hi"), &[], &[]);
        let sys = v["system_instruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(sys.contains("builder"));
        assert_eq!(v["contents"][0]["parts"][0]["text"], "hi");
    }

    #[test]
    fn fact_rows_never_enter_gemini_requests() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"verb\":\"inject\",\"text\":\"祕密指令ZZZ\"}\n{\"verb\":\"turn\",\"user\":\"q\",\"assistant\":\"a\"}\n",
        )
        .unwrap();
        let history = crate::memory::history_snapshot_in(&tmp.0);
        let request = gemini_request(&turn_for(&[], &history, "now"), &[], &[]).to_string();
        assert!(request.contains("\"a\""));
        assert!(!request.contains("祕密指令ZZZ"));
        assert!(!request.contains("\"verb\""));
    }

    #[test]
    fn the_endpoint_is_the_one_the_app_already_speaks() {
        assert_eq!(
            gemini_url("gemini-2.5-flash"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
        );
    }

    // --- GeminiStepParse ---
    #[test]
    fn a_lone_function_call_becomes_a_read_pane_action() {
        let v = parts(json!([{ "functionCall": { "name": "read_pane", "args": { "agent": "builder" } } }]));
        assert_eq!(gemini_step(&v).unwrap(), vec![read_pane("builder")]);
    }

    #[test]
    fn text_and_a_call_in_one_candidate_are_both_kept_in_order() {
        let v = parts(json!([
            { "text": "我看一下" },
            { "functionCall": { "name": "read_pane", "args": { "agent": "builder" } } },
        ]));
        assert_eq!(
            gemini_step(&v).unwrap(),
            vec![StepAction::Speak("我看一下".into()), read_pane("builder")]
        );
    }

    #[test]
    fn parallel_calls_are_all_kept_in_wire_order() {
        let v = parts(json!([
            { "functionCall": { "name": "read_pane", "args": { "agent": "builder" } } },
            { "functionCall": { "name": "read_pane", "args": { "agent": "reviewer" } } },
        ]));
        assert_eq!(
            gemini_step(&v).unwrap(),
            vec![read_pane("builder"), read_pane("reviewer")]
        );
    }

    #[test]
    fn t2d_extract_gemini_skips_thought_parts() {
        let v = parts(json!([
            { "text": "讓我想想…", "thought": true },
            { "text": "天空是藍色的" },
        ]));
        assert_eq!(gemini_step(&v).unwrap(), vec![StepAction::Speak("天空是藍色的".into())]);
    }

    #[test]
    fn t2b_extract_gemini_text() {
        let v = parts(json!([{ "text": "你" }, { "text": "好" }]));
        assert_eq!(gemini_step(&v).unwrap(), vec![StepAction::Speak("你好".into())]);
    }

    #[test]
    fn a_native_summon_call_becomes_a_summon_action() {
        let v = parts(json!([{
            "functionCall": { "name": "summon", "args": { "project": "cyris", "task": "跑測試" }, "id": "c1" }
        }]));
        assert_eq!(
            gemini_step(&v).unwrap(),
            vec![StepAction::Summon { project: "cyris".into(), task: "跑測試".into() }]
        );
    }

    #[test]
    fn a_summon_call_missing_its_task_is_rejected_by_argument_name() {
        let v = parts(json!([{
            "functionCall": { "name": "summon", "args": { "project": "cyris" }, "id": "c1" }
        }]));
        let actions = gemini_step(&v).unwrap();
        let [StepAction::Rejected { call, reason }] = actions.as_slice() else {
            panic!("a half-filled summon must not become an action: {actions:?}");
        };
        assert_eq!(call, &CallRef { id: Some("c1".into()), name: "summon".into() });
        assert!(reason.contains("task"));
    }

    #[test]
    fn a_call_for_a_tool_we_do_not_have_is_rejected_by_name() {
        let v = parts(json!([{
            "functionCall": { "name": "recall", "args": { "query": "x" }, "id": "c2" }
        }]));
        assert_eq!(
            gemini_step(&v).unwrap(),
            vec![StepAction::Rejected {
                call: CallRef { id: Some("c2".into()), name: "recall".into() },
                reason: "沒有這個工具：recall".into(),
            }]
        );
    }

    #[test]
    fn t4_extract_empty_content_err_names_provider() {
        let v: Value = serde_json::from_str(r#"{"candidates":[]}"#).unwrap();
        assert!(gemini_step(&v).unwrap_err().contains("gemini"));
    }

    // --- SpeakTotality on the native path ---
    #[test]
    fn a_summon_object_arriving_as_text_is_never_spoken() {
        let bare = r#"{"action":"summon","project":"cyris","task":"跑測試"}"#;
        let summon = StepAction::Summon { project: "cyris".into(), task: "跑測試".into() };
        assert_eq!(gemini_step(&parts(json!([{ "text": bare }]))).unwrap(), vec![summon.clone()]);
        let fenced = format!("```json\n{bare}\n```");
        assert_eq!(gemini_step(&parts(json!([{ "text": fenced }]))).unwrap(), vec![summon]);
    }

    #[test]
    fn prose_and_broken_json_stay_speech_and_arrive_trimmed() {
        assert_eq!(
            gemini_step(&parts(json!([{ "text": "好的，我會處理。\n\n" }]))).unwrap(),
            vec![StepAction::Speak("好的，我會處理。".into())]
        );
        assert_eq!(
            gemini_step(&parts(json!([{ "text": "{壞掉的json" }]))).unwrap(),
            vec![StepAction::Speak("{壞掉的json".into())]
        );
        let half = r#"{"action":"summon","project":"","task":"跑測試"}"#;
        assert_eq!(
            gemini_step(&parts(json!([{ "text": half }]))).unwrap(),
            vec![StepAction::Speak(half.into())]
        );
    }

    // --- GeminiToolResultRoundTrip ---
    fn result(name: &str, id: Option<&str>, text: &str) -> ToolResult {
        ToolResult {
            call: CallRef { id: id.map(str::to_string), name: name.to_string() },
            text: text.to_string(),
        }
    }

    fn model_content(agent: &str) -> Value {
        json!({ "role": "model", "parts": [{ "functionCall": { "name": "read_pane", "args": { "agent": agent } } }] })
    }

    #[test]
    fn a_round_replays_the_model_turn_then_answers_its_call() {
        let content = model_content("builder");
        let round = result_contents(&content, &[result("read_pane", None, "畫面…")]);
        assert_eq!(round.len(), 2);
        assert_eq!(round[0], content);
        assert_eq!(
            round[1],
            json!({
                "role": "user",
                "parts": [{ "functionResponse": { "name": "read_pane", "response": { "result": "畫面…" } } }],
            })
        );
    }

    #[test]
    fn two_results_share_one_user_entry_in_result_order() {
        let round = result_contents(
            &model_content("builder"),
            &[
                result("read_pane", None, "builder 的畫面"),
                result("read_pane", None, "reviewer 的畫面"),
            ],
        );
        assert_eq!(round.len(), 2);
        let parts = round[1]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["response"]["result"], "builder 的畫面");
        assert_eq!(parts[1]["functionResponse"]["response"]["result"], "reviewer 的畫面");
    }

    #[test]
    fn a_call_id_is_echoed_only_when_the_provider_gave_one() {
        let with_id = result_contents(&model_content("builder"), &[result("read_pane", Some("c1"), "x")]);
        assert_eq!(with_id[1]["parts"][0]["functionResponse"]["id"], "c1");
        let without = result_contents(&model_content("builder"), &[result("read_pane", None, "x")]);
        assert!(without[1]["parts"][0]["functionResponse"]
            .as_object()
            .unwrap()
            .get("id")
            .is_none());
    }

    #[test]
    fn a_second_round_appends_after_the_first_and_leaves_the_prefix_alone() {
        let mut tail = result_contents(&model_content("builder"), &[result("read_pane", None, "第一輪")]);
        tail.extend(result_contents(&model_content("reviewer"), &[result("read_pane", None, "第二輪")]));
        let history = vec![pair("早安", "你好")];
        let v = gemini_request(&turn_for(&[], &history, "誰在工作"), &[], &tail);
        let contents = v["contents"].as_array().unwrap();
        let roles: Vec<&str> = contents.iter().map(|c| c["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "model", "user", "model", "user", "model", "user"]);
        // the history + transcript prefix is untouched by the appended rounds
        assert_eq!(contents[0]["parts"][0]["text"], "早安");
        assert_eq!(contents[1]["parts"][0]["text"], "你好");
        assert_eq!(contents[2]["parts"][0]["text"], "誰在工作");
        assert_eq!(contents[4]["parts"][0]["functionResponse"]["response"]["result"], "第一輪");
        assert_eq!(contents[6]["parts"][0]["functionResponse"]["response"]["result"], "第二輪");
    }
}
