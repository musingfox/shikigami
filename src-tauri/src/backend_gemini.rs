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
    #[cfg(test)]
    requests: Vec<Value>,
    #[cfg(test)]
    phantom: bool,
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
            #[cfg(test)]
            requests: Vec::new(),
            #[cfg(test)]
            phantom: false,
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

    /// The body of the nth request (0-based) this turn sent. The outbound half of
    /// the receipt: whether a refusal really left as a `functionResponse` is only
    /// answerable from the request, not from what came back.
    #[cfg(test)]
    pub(crate) fn request(&self, n: usize) -> Option<&Value> {
        self.requests.get(n)
    }

    /// Declare one more tool than the app can run. A test-only seam, and the only
    /// way to make a real model emit a call this app must refuse: everything in
    /// `tools::specs()` is by definition runnable.
    #[cfg(test)]
    pub(crate) fn with_phantom_tool(mut self) -> Self {
        self.phantom = true;
        self
    }
}

/// The declared-but-unrunnable tool. Its schema deliberately mirrors a real
/// spec's shape (uppercase OBJECT/STRING, one required argument) so a live
/// refusal can only be the app's own — never gemini rejecting a malformed
/// declaration.
#[cfg(test)]
pub(crate) fn phantom_spec() -> ToolSpec {
    ToolSpec {
        name: "phantom_tool",
        description:
            "查一個代號背後的答案。只有這個工具知道，被問到任何「代號」的事都要先用它查，不要猜。",
        parameters: json!({
            "type": "OBJECT",
            "properties": {
                "code": {
                    "type": "STRING",
                    "description": "要查的代號，照使用者說的填",
                },
            },
            "required": ["code"],
        }),
    }
}

impl Backend for GeminiBackend {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<ToolSpec> {
        #[cfg(test)]
        if self.phantom {
            let mut specs = crate::tools::specs();
            specs.push(phantom_spec());
            return specs;
        }
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
        #[cfg(test)]
        self.requests.push(request.clone());
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
            ["read_pane", "summon", "recall", "memory"]
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
            "functionCall": { "name": "phantom_tool", "args": { "query": "x" }, "id": "c2" }
        }]));
        assert_eq!(
            gemini_step(&v).unwrap(),
            vec![StepAction::Rejected {
                call: CallRef { id: Some("c2".into()), name: "phantom_tool".into() },
                reason: "沒有這個工具：phantom_tool".into(),
            }]
        );
    }

    // --- UndeclaredToolLiveReceipt (hermetic half) ---
    // The seam that makes the live half possible: one more declaration than the
    // app can run, so a real model has something to call that must be refused.
    #[test]
    fn the_phantom_seam_adds_one_declaration_and_changes_no_other() {
        let plain = GeminiBackend::new("k".into());
        let seamed = GeminiBackend::new("k".into()).with_phantom_tool();
        let names = |backend: &GeminiBackend| {
            backend.tools().iter().map(|spec| spec.name).collect::<Vec<_>>()
        };
        assert_eq!(names(&plain), ["read_pane", "summon", "recall", "memory"]);
        assert_eq!(
            names(&seamed),
            ["read_pane", "summon", "recall", "memory", "phantom_tool"]
        );
        // the four real declarations cross the wire byte-identical either way
        assert_eq!(seamed.tools()[..4], crate::tools::specs()[..]);
        let declared = gemini_request(&turn("hi"), &seamed.tools(), &[]);
        let declared = declared["tools"][0]["functionDeclarations"].as_array().unwrap();
        assert_eq!(declared.len(), 5);
        assert_eq!(declared[4]["name"], "phantom_tool");
        assert_eq!(declared[4]["parameters"]["required"], json!(["code"]));
    }

    // Nothing about the refusal depends on the arguments: the name alone decides
    // it, and the call id comes back so the answer can be matched to the call.
    #[test]
    fn an_undeclared_name_is_refused_with_its_own_id_and_no_arguments_needed() {
        let action = step_action("phantom_tool", &json!({}), Some("id-1"));
        let StepAction::Rejected { call, reason } = &action else {
            panic!("a tool the app cannot run must not become an action: {action:?}");
        };
        assert_eq!(reason, "沒有這個工具：phantom_tool");
        assert_eq!(call.id.as_deref(), Some("id-1"));
        assert_eq!(call.name, "phantom_tool");
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

    // --- GeminiLiveToolLoopReceipt ---
    // `cargo test` structurally cannot prove that a real gemini call returns a
    // function call, nor that a two-step turn's answer is grounded in the pane
    // text the first step fetched. These two stay #[ignore]; a human runs them
    // and reads the transcript. That is the receipt.

    /// A backend that prints what actually crossed the wire while delegating to
    /// the real one — the transcript is the deliverable, so it is built here
    /// rather than smuggled into production logging.
    struct Receipt {
        inner: GeminiBackend,
        step: usize,
        first_actions: Vec<StepAction>,
    }

    impl Receipt {
        fn new(inner: GeminiBackend) -> Self {
            Self { inner, step: 0, first_actions: Vec::new() }
        }

        fn steps(&self) -> usize {
            self.inner.steps()
        }

        fn asked_to_read_a_pane_first(&self) -> bool {
            self.first_actions
                .iter()
                .any(|action| matches!(action, StepAction::ReadPane { .. }))
        }
    }

    impl Backend for Receipt {
        fn name(&self) -> &'static str {
            self.inner.name()
        }

        fn tools(&self) -> Vec<ToolSpec> {
            self.inner.tools()
        }

        async fn step(
            &mut self,
            turn: &TurnStart,
            results: &[ToolResult],
            budget: Duration,
        ) -> Result<Vec<StepAction>, String> {
            self.step += 1;
            let n = self.step;
            println!("\n── step {n} (budget {budget:?}, {} tool result(s) fed back)", results.len());
            for fed in results {
                println!("   ↳ result for {}:\n{}", fed.call.name, fed.text);
            }
            let actions = self.inner.step(turn, results, budget).await;
            if n == 1 {
                // printed exactly once, so mixed text+functionCall parts and
                // parallel functionCall parts are observable in the transcript
                println!(
                    "── raw step-1 response body:\n{}",
                    self.inner.first_body().unwrap_or("<no body: the request failed>")
                );
                self.first_actions = actions.clone().unwrap_or_default();
            }
            println!("── step {n} parsed actions: {actions:?}");
            actions
        }
    }

    /// The isolation order matters: the override relocates the WHOLE config root,
    /// so a key file under the real one would become invisible. Resolve the real
    /// dir FIRST, carry the key files (only those, at 0600) into a throwaway
    /// root, and only then install the override.
    fn live_key_in(tmp: &std::path::Path) -> String {
        crate::backend::copy_provider_keys(&crate::config::config_dir(), tmp);
        crate::backend::load_key(crate::backend::Provider::Gemini).expect(
            "live receipt needs GEMINI_API_KEY in the env or a gemini_api_key file under ~/.config/shikigami/",
        )
    }

    fn live_roster() -> Vec<crate::events::AgentEntry> {
        // NOT get_roster(): that reads the cache the polling thread fills, and a
        // test binary never runs it.
        let roster = crate::herdr::fetch_roster_now()
            .expect("live receipt needs herdr running (agent.list over its socket)");
        assert!(
            roster.iter().any(|entry| !entry.pane.is_empty()),
            "live receipt needs at least one live herdr agent to look at"
        );
        println!(
            "== roster: {:?}",
            roster
                .iter()
                .map(|entry| (entry.name.as_str(), entry.status.as_str(), entry.pane.as_str()))
                .collect::<Vec<_>>()
        );
        roster
    }

    fn run_live(
        key: String,
        roster: &[crate::events::AgentEntry],
        transcript: &str,
    ) -> (Receipt, Result<crate::brain::SummonAction, String>) {
        // The prefetched depth is deliberately EMPTY: the model has to reach for
        // read_pane instead of being handed the excerpt, which is the point.
        let turn = TurnStart {
            system: crate::brain::system_prompt(roster, &[], None, &crate::tools::specs()),
            history: Vec::new(),
            transcript: transcript.to_string(),
        };
        println!("== transcript: {transcript}");
        let mut receipt = Receipt::new(GeminiBackend::new(key));
        // The caller has already relocated the config root, so a live `recall`
        // searches the throwaway memory rather than the real one.
        let tools = crate::tools::LiveTools {
            roster: roster.to_vec(),
            dir: crate::config::config_dir(),
        };
        let started = std::time::Instant::now();
        let outcome = tauri::async_runtime::block_on(crate::brain::run_turn_with(
            &mut receipt,
            &turn,
            &tools,
            move || crate::brain::TURN_BUDGET.saturating_sub(started.elapsed()),
            crate::brain::MAX_STEPS,
        ));
        println!("\n== final outcome: {outcome:?}");
        (receipt, outcome)
    }

    #[test]
    #[ignore]
    fn gemini_live_looks_at_a_pane_then_answers_from_it() {
        // run: GEMINI_API_KEY=… cargo test gemini_live_looks -- --ignored --nocapture
        let tmp = Tmp::new();
        let key = live_key_in(&tmp.0);
        let _dir = crate::config::test_override::ConfigDirOverride::set(&tmp.0);
        let roster = live_roster();
        // the agent named is a real one, so a human can judge whether the second
        // step's answer is grounded in the pane text the first step fetched
        let target = roster
            .iter()
            .find(|entry| !entry.pane.is_empty())
            .expect("checked by live_roster")
            .name
            .clone();

        let (receipt, outcome) =
            run_live(key, &roster, &format!("去看一下 {target} 的畫面，它現在在做什麼？"));

        let Ok(crate::brain::SummonAction::Speak(answer)) = &outcome else {
            panic!("a look-and-tell-me question must end in speech: {outcome:?}");
        };
        assert!(!answer.trim().is_empty(), "the second step must actually say something");
        assert!(
            receipt.asked_to_read_a_pane_first(),
            "step 1 must ask to read a pane — if no functionCall ever arrives, flip THINKING_BUDGET off 0 and re-run"
        );
        assert_eq!(receipt.steps(), 2, "a look-then-answer turn costs exactly two requests");
    }

    #[test]
    #[ignore]
    fn gemini_live_fast_path_still_costs_one_request() {
        // run: GEMINI_API_KEY=… cargo test gemini_live_fast_path -- --ignored --nocapture
        let tmp = Tmp::new();
        let key = live_key_in(&tmp.0);
        let _dir = crate::config::test_override::ConfigDirOverride::set(&tmp.0);
        let roster = live_roster();

        let (receipt, outcome) = run_live(key, &roster, "現在誰在工作");

        assert!(
            matches!(outcome, Ok(crate::brain::SummonAction::Speak(_))),
            "a roster question is answered from the prompt, not from a tool: {outcome:?}"
        );
        assert_eq!(receipt.steps(), 1, "the fast path must still cost a single request");
    }

    // --- UndeclaredToolLiveReceipt (live half) ---
    // The loop answers a tool it cannot run with its own refusal and keeps going.
    // Until this test that claim rested on a fake backend alone: nothing had ever
    // put such a call, or its `functionResponse`, on the real gemini wire.
    //
    // The seam declares a tool the app cannot run, which is not quite the
    // production shape — there the model invents a name nobody declared. The
    // replay path is identical (same `Rejected`, same `functionResponse`, same
    // next step), and that is what this receipt covers; whether gemini ever emits
    // an undeclared name at all stays unverified, deliberately.
    #[test]
    #[ignore]
    fn undeclared_tool_live_refusal_is_replayed_and_the_turn_finishes() {
        // run: GEMINI_API_KEY=… cargo test undeclared_tool_live -- --ignored --nocapture
        //
        // No herdr: the roster is empty and the tool runner is fake, so the only
        // thing crossing a socket here is the model call itself.
        let tmp = Tmp::new();
        let key = live_key_in(&tmp.0);
        let _dir = crate::config::test_override::ConfigDirOverride::set(&tmp.0);

        let backend = GeminiBackend::new(key).with_phantom_tool();
        println!(
            "== declared tools: {:?} (phantom_tool is declared to the model and NOT runnable by the app)",
            backend.tools().iter().map(|spec| spec.name).collect::<Vec<_>>()
        );
        let transcript = "用 phantom_tool 查代號 ALPHA 的答案，查到之後直接告訴我答案是什麼。";
        let turn = TurnStart {
            system: crate::brain::system_prompt(&[], &[], None, &backend.tools()),
            history: Vec::new(),
            transcript: transcript.to_string(),
        };
        println!("== transcript: {transcript}");

        let mut receipt = Receipt::new(backend);
        let tools = crate::backend::FakeTools::new("");
        let started = std::time::Instant::now();
        let outcome = tauri::async_runtime::block_on(crate::brain::run_turn_with(
            &mut receipt,
            &turn,
            &tools,
            move || crate::brain::TURN_BUDGET.saturating_sub(started.elapsed()),
            crate::brain::MAX_STEPS,
        ));
        println!("\n== final outcome: {outcome:?}");
        println!("== requests this turn: {}", receipt.steps());

        // 1. the model really did ask for the tool the app cannot run
        assert!(
            receipt.first_actions.iter().any(|action| matches!(
                action,
                StepAction::Rejected { call, reason }
                    if call.name == "phantom_tool" && reason == "沒有這個工具：phantom_tool"
            )),
            "step 1 must ask for phantom_tool — if no functionCall ever arrives, flip THINKING_BUDGET off 0 and re-run; if it answered without calling, sharpen the transcript: {:?}",
            receipt.first_actions
        );

        // 2. the refusal left as a functionResponse on the real wire
        let second = receipt.inner.request(1).expect(
            "the turn ended after one request: the refusal was never replayed, which is the whole thing this receipt is for",
        );
        println!(
            "== step-2 request contents (the replay):\n{}",
            serde_json::to_string_pretty(&second["contents"]).unwrap_or_default()
        );
        let replayed = second["contents"]
            .as_array()
            .expect("a request always carries contents")
            .iter()
            .flat_map(|entry| entry["parts"].as_array().cloned().unwrap_or_default())
            .filter_map(|part| part.get("functionResponse").cloned())
            .find(|response| response["name"] == "phantom_tool")
            .expect("step 2 must answer the phantom call with a functionResponse of its own");
        assert_eq!(replayed["response"]["result"], "沒有這個工具：phantom_tool");

        // 3. and the turn ended in speech, not in an error
        assert!(
            matches!(outcome, Ok(crate::brain::SummonAction::Speak(_))),
            "a refused tool must not end the turn as a failure: {outcome:?}"
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
