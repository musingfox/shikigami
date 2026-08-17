// The port between the brain's decision loop and whatever model answers it: one
// "advance the turn by one step" call, provider-neutral actions coming back, and
// no wire format anywhere in sight — that lives in backend_gemini.rs /
// backend_text.rs, which are the only modules allowed to name a provider key.
//
// Provider picked by SHIKIGAMI_BRAIN env override, else the first provider with
// a key available, cheapest first (gemini -> openai -> anthropic). That order is
// selection-time only, no failover: once a turn starts, a request failure ends
// the turn instead of silently retrying or handing off to a second provider.
//
// ponytail: requests/responses are serde_json::Value, no typed structs per vendor.

use std::fs;
use std::future::Future;
use std::time::Duration;

use serde_json::Value;

use crate::brain::SummonAction;

/// One tool a backend declares to its model. `parameters` is a JSON Schema
/// object; the adapters pass it through to whatever key their wire calls it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// Which call a result answers. `id` is present only for providers that hand
/// out call ids; the name alone is enough for the ones that do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRef {
    pub id: Option<String>,
    pub name: String,
}

/// What the app hands back for one call the model made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call: CallRef,
    pub text: String,
}

/// Everything the model produced in one step, in wire order. `Summon` carries no
/// `CallRef` on purpose: it is loop-terminal, so no result can ever be fed back
/// for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    Speak(String),
    ReadPane { call: CallRef, agent: String },
    Summon { project: String, task: String },
    Rejected { call: CallRef, reason: String },
}

/// The turn-invariant half of a request: what the loop assembles once and the
/// backend re-sends on every step of the same utterance.
#[derive(Debug, Clone, Default)]
pub struct TurnStart {
    pub system: String,
    pub history: Vec<(String, String)>,
    pub transcript: String,
}

/// One step of one turn. The backend is stateful for the turn's duration — a
/// provider that requires its own returned content replayed verbatim keeps that
/// raw value inside its adapter, never in the core.
pub trait Backend {
    fn name(&self) -> &'static str;

    fn tools(&self) -> Vec<ToolSpec>;

    /// Declared as `impl Future + Send` rather than `async fn` because the
    /// future the whole loop returns has to be provably `Send` to live inside a
    /// `#[tauri::command]`.
    fn step(
        &mut self,
        turn: &TurnStart,
        results: &[ToolResult],
        budget: Duration,
    ) -> impl Future<Output = Result<Vec<StepAction>, String>> + Send;
}

/// A tool call named on the wire -> the action the loop can act on. Total: an
/// unknown tool, or a known one missing a required argument, becomes `Rejected`
/// so its reason can go back to the model as that call's result instead of
/// ending the turn.
pub fn step_action(name: &str, args: &Value, id: Option<&str>) -> StepAction {
    let call = CallRef {
        id: id.map(str::to_string),
        name: name.to_string(),
    };
    let field = |key: &str| args[key].as_str().unwrap_or_default().trim().to_string();
    match name {
        "read_pane" => {
            let agent = field("agent");
            if agent.is_empty() {
                return StepAction::Rejected {
                    call,
                    reason: "read_pane 需要 agent 這個參數，填名冊上的 agent 名稱".to_string(),
                };
            }
            StepAction::ReadPane { call, agent }
        }
        "summon" => {
            let (project, task) = (field("project"), field("task"));
            if project.is_empty() || task.is_empty() {
                return StepAction::Rejected {
                    call,
                    reason: "summon 需要 project 和 task 兩個參數都填好".to_string(),
                };
            }
            StepAction::Summon { project, task }
        }
        other => StepAction::Rejected {
            call,
            reason: format!("沒有這個工具：{other}"),
        },
    }
}

/// A bare text reply -> an action. Routed through the brain's total classifier
/// so a model answering with the prose summon convention is never read out loud,
/// on the native path as much as on the text-simulated one.
pub fn text_action(text: &str) -> StepAction {
    match crate::brain::parse_action(text) {
        SummonAction::Speak(text) => StepAction::Speak(text),
        SummonAction::Summon { project, task } => StepAction::Summon { project, task },
    }
}

/// A single request never outlives the turn that started it: the client's
/// timeout is the smaller of the remaining turn budget and this ceiling.
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Client shape (no_proxy + timeout) copied from SumVox — macOS CoreFoundation
/// workaround.
pub(crate) fn http_client(budget: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(budget.min(MAX_REQUEST_TIMEOUT))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Send one already-shaped request and hand back the raw response body. Shared
/// transport only — which keys go in the body is each adapter's own business.
pub(crate) async fn send_json(
    request: reqwest::RequestBuilder,
    body: &Value,
    provider: &str,
) -> Result<String, String> {
    let response = request
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("brain request failed: {}", e))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("brain read body: {}", e))?;
    if !status.is_success() {
        return Err(format!("{} API {}: {}", provider, status, text));
    }
    Ok(text)
}

pub(crate) fn parse_body(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("brain parse: {} body={}", e, raw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    OpenAi,
    Anthropic,
}

// cheap-first order for auto-detection
pub(crate) const PROVIDERS: [Provider; 3] =
    [Provider::Gemini, Provider::OpenAi, Provider::Anthropic];

impl Provider {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Provider::Gemini => "gemini",
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
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

pub(crate) fn load_key(provider: Provider) -> Result<String, String> {
    let (env_var, file_name) = provider.key_sources();
    let env = std::env::var(env_var).ok();
    let file = fs::read_to_string(crate::config::config_dir().join(file_name)).ok();
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

fn detect_provider() -> Result<(Provider, String), String> {
    let explicit = std::env::var("SHIKIGAMI_BRAIN").ok();
    let available = PROVIDERS.map(|p| load_key(p).is_ok());
    let provider = pick_provider(explicit.as_deref(), available)?;
    let key = load_key(provider)?;
    Ok((provider, key))
}

/// Whichever backend answers this utterance. A two-arm enum rather than a
/// `Box<dyn Backend>`: the trait's future is an RPITIT, so this delegating impl
/// is the only place in the app that matches on which provider was chosen.
pub(crate) enum SelectedBackend {
    Gemini(crate::backend_gemini::GeminiBackend),
    Chat(crate::backend_text::TextBackend),
}

impl Backend for SelectedBackend {
    fn name(&self) -> &'static str {
        match self {
            SelectedBackend::Gemini(backend) => backend.name(),
            SelectedBackend::Chat(backend) => backend.name(),
        }
    }

    fn tools(&self) -> Vec<ToolSpec> {
        match self {
            SelectedBackend::Gemini(backend) => backend.tools(),
            SelectedBackend::Chat(backend) => backend.tools(),
        }
    }

    async fn step(
        &mut self,
        turn: &TurnStart,
        results: &[ToolResult],
        budget: Duration,
    ) -> Result<Vec<StepAction>, String> {
        match self {
            SelectedBackend::Gemini(backend) => backend.step(turn, results, budget).await,
            SelectedBackend::Chat(backend) => backend.step(turn, results, budget).await,
        }
    }
}

/// Build the backend one utterance will use. Cheapest-first is a *selection-time*
/// order only: once a turn has started there is no failover — a request failure
/// ends the turn rather than silently retrying or handing off.
pub(crate) fn detect() -> Result<SelectedBackend, String> {
    let (provider, key) = detect_provider()?;
    Ok(match provider {
        Provider::Gemini => {
            SelectedBackend::Gemini(crate::backend_gemini::GeminiBackend::new(key))
        }
        Provider::OpenAi => SelectedBackend::Chat(crate::backend_text::TextBackend::new(
            crate::backend_text::ChatFlavor::OpenAi,
            key,
        )),
        Provider::Anthropic => SelectedBackend::Chat(crate::backend_text::TextBackend::new(
            crate::backend_text::ChatFlavor::Anthropic,
            key,
        )),
    })
}

/// Carry provider key files into a throwaway config root — and nothing else,
/// so memory stays isolated. A missing key file is silent: the env-var form
/// is the other half of `load_key` and must keep working on its own.
///
/// The copy is chmod 0600 because `temp_dir()` is only private when TMPDIR
/// is set (macOS gives a per-user 0700 dir); with it unset Rust falls back
/// to a world-readable /tmp, and a real API key would land there at 0644.
#[cfg(test)]
pub(crate) fn copy_provider_keys(from: &std::path::Path, to: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    for provider in PROVIDERS {
        let (_, file_name) = provider.key_sources();
        if let Ok(key) = std::fs::read_to_string(from.join(file_name)) {
            let dest = to.join(file_name);
            if std::fs::write(&dest, key).is_ok() {
                let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

/// A backend that answers from a script and records what the loop handed it.
/// Lives beside the port rather than inside one test module so both the loop's
/// tests and the port's own can drive it.
#[cfg(test)]
pub(crate) struct FakeBackend {
    script: Vec<Result<Vec<StepAction>, String>>,
    calls: usize,
    results: Vec<Vec<ToolResult>>,
    budgets: Vec<Duration>,
}

#[cfg(test)]
impl FakeBackend {
    pub(crate) fn new(script: Vec<Result<Vec<StepAction>, String>>) -> Self {
        assert!(!script.is_empty(), "a scripted backend needs at least one answer");
        Self {
            script,
            calls: 0,
            results: Vec::new(),
            budgets: Vec::new(),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls
    }

    /// The tool results the loop sent on step `step` (1-based).
    pub(crate) fn results_on(&self, step: usize) -> &[ToolResult] {
        &self.results[step - 1]
    }

    /// The per-request budget the loop passed on step `step` (1-based).
    pub(crate) fn budget_on(&self, step: usize) -> Duration {
        self.budgets[step - 1]
    }
}

#[cfg(test)]
impl Backend for FakeBackend {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn tools(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn step(
        &mut self,
        _turn: &TurnStart,
        results: &[ToolResult],
        budget: Duration,
    ) -> impl Future<Output = Result<Vec<StepAction>, String>> + Send {
        self.results.push(results.to_vec());
        self.budgets.push(budget);
        // the last scripted answer repeats, so "keeps asking for another look" is
        // a one-entry script
        let index = self.calls.min(self.script.len() - 1);
        self.calls += 1;
        std::future::ready(self.script[index].clone())
    }
}

/// A tool runner with no socket behind it: it builds the same body the live
/// runner would from an injected pane text, and records who was read.
#[cfg(test)]
pub(crate) struct FakeTools {
    roster: Vec<crate::events::AgentEntry>,
    screen: String,
    reads: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeTools {
    pub(crate) fn new(screen: &str) -> Self {
        let entry = |name: &str, pane: &str| crate::events::AgentEntry {
            id: format!("id-{name}"),
            name: name.to_string(),
            pane: pane.to_string(),
            status: "working".to_string(),
            title: String::new(),
            cwd: String::new(),
        };
        Self {
            roster: vec![entry("builder", "%1"), entry("reviewer", "%2")],
            screen: screen.to_string(),
            reads: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl crate::tools::Tools for FakeTools {
    fn read_pane(&self, agent: &str) -> impl Future<Output = String> + Send {
        self.reads.lock().unwrap().push(agent.to_string());
        let screen = self.screen.clone();
        std::future::ready(crate::tools::read_pane_body(&self.roster, agent, move |_| {
            Ok(screen)
        }))
    }
}

/// A clock with room to spare on every check.
#[cfg(test)]
pub(crate) fn always(secs: u64) -> impl Fn() -> Duration {
    move || Duration::from_secs(secs)
}

/// A clock that answers once and then reports nothing left — a deadline lapsing
/// mid-step, which is when partial tool results would be the tempting mistake.
#[cfg(test)]
pub(crate) fn then_exhausted(first: u64) -> impl Fn() -> Duration {
    let checks = std::sync::atomic::AtomicUsize::new(0);
    move || {
        if checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
            Duration::from_secs(first)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(std::path::PathBuf);

    impl Tmp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "shikigami-backend-{}-{}",
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

    #[test]
    fn provider_keys_travel_but_memory_does_not() {
        let real = Tmp::new();
        let throwaway = Tmp::new();
        std::fs::write(real.0.join("anthropic_api_key"), "sk-file\n").unwrap();
        std::fs::write(real.0.join("memory.jsonl"), "{\"role\":\"user\"}\n").unwrap();

        copy_provider_keys(&real.0, &throwaway.0);

        assert_eq!(
            std::fs::read_to_string(throwaway.0.join("anthropic_api_key")).unwrap(),
            "sk-file\n"
        );
        // the providers without a key file are skipped, not created empty
        assert!(!throwaway.0.join("gemini_api_key").exists());
        assert!(!throwaway.0.join("openai_api_key").exists());
        // isolation intact: memory is never carried over
        assert!(!throwaway.0.join("memory.jsonl").exists());
    }

    // A tool call the app cannot run is a correctable mistake, not a failure.
    #[test]
    fn unknown_tool_is_rejected_by_name() {
        let action = step_action("recall", &serde_json::json!({ "query": "x" }), Some("c2"));
        assert_eq!(
            action,
            StepAction::Rejected {
                call: CallRef { id: Some("c2".into()), name: "recall".into() },
                reason: "沒有這個工具：recall".into(),
            }
        );
    }

    #[test]
    fn read_pane_without_an_agent_is_rejected_naming_the_argument() {
        let StepAction::Rejected { reason, .. } =
            step_action("read_pane", &serde_json::json!({}), None)
        else {
            panic!("a read_pane without an agent must not become a call");
        };
        assert!(reason.contains("agent"));
    }

    #[test]
    fn well_formed_calls_become_neutral_actions() {
        assert_eq!(
            step_action("read_pane", &serde_json::json!({ "agent": "builder" }), None),
            StepAction::ReadPane {
                call: CallRef { id: None, name: "read_pane".into() },
                agent: "builder".into(),
            }
        );
        assert_eq!(
            step_action(
                "summon",
                &serde_json::json!({ "project": "cyris", "task": "跑測試" }),
                Some("c1"),
            ),
            StepAction::Summon { project: "cyris".into(), task: "跑測試".into() }
        );
    }

    // ProviderErrorEndsTurn: the failing string is provider-shaped, so this half
    // of the contract is pinned here rather than in the provider-free loop module.
    #[test]
    fn a_provider_failure_ends_the_turn_without_a_retry_or_a_hand_off() {
        let mut backend = FakeBackend::new(vec![Err("gemini API 503: overloaded".to_string())]);
        let tools = FakeTools::new("");
        let got = tauri::async_runtime::block_on(crate::brain::run_turn_with(
            &mut backend,
            &TurnStart::default(),
            &tools,
            always(45),
            crate::brain::MAX_STEPS,
        ));
        assert_eq!(got, Err("gemini API 503: overloaded".to_string()));
        assert_eq!(backend.calls(), 1);
        assert!(tools.reads().is_empty());
    }

    // The prose summon convention must not reach the speaker on any path.
    #[test]
    fn text_carrying_a_summon_object_becomes_a_summon() {
        assert_eq!(
            text_action(r#"{"action":"summon","project":"cyris","task":"跑測試"}"#),
            StepAction::Summon { project: "cyris".into(), task: "跑測試".into() }
        );
        assert_eq!(text_action("好的，我會處理。\n\n"), StepAction::Speak("好的，我會處理。".into()));
    }
}
