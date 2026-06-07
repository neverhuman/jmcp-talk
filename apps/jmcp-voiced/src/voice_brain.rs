use anyhow::{anyhow, Context, Result};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct VoiceBrain {
    ttl: Duration,
    classifier_enabled: bool,
    inner: Arc<Mutex<CachedManifest>>,
}

#[derive(Clone, Debug, Default)]
pub struct VoiceBrainSnapshot {
    pub loaded: bool,
    pub manifest_hash: Option<String>,
    pub cache_age_ms: Option<u128>,
    pub generated_at: Option<String>,
    pub last_error: Option<String>,
    pub manifest: Option<Value>,
}

#[derive(Default)]
struct CachedManifest {
    manifest: Option<Value>,
    fetched_at: Option<Instant>,
    last_error: Option<String>,
}

#[derive(Debug)]
pub struct CommandOutcome {
    pub reply: String,
    pub tool_name: Option<String>,
    pub tool_policy_decision: String,
    pub confirmation_required: bool,
    pub confidence: f32,
}

pub struct BrainRuntime<'a> {
    pub http: &'a Client,
    pub core_base: &'a str,
    pub llm_upstream: &'a str,
    pub llm_model: &'a str,
    pub total_timeout: Duration,
}

struct CoreCommand {
    id: String,
    method: Method,
    route: String,
    body: Option<Value>,
    risk_class: String,
    confirmation_policy: String,
    confidence: f32,
}

impl VoiceBrain {
    pub fn new(ttl: Duration, classifier_enabled: bool) -> Self {
        Self {
            ttl,
            classifier_enabled,
            inner: Arc::new(Mutex::new(CachedManifest::default())),
        }
    }

    pub fn classifier_enabled(&self) -> bool {
        self.classifier_enabled
    }

    pub async fn snapshot(&self, http: &Client, core_base: &str) -> VoiceBrainSnapshot {
        let cached = {
            let guard = self.inner.lock().expect("voice brain cache lock");
            guard.fetched_at.and_then(|fetched_at| {
                if fetched_at.elapsed() <= self.ttl {
                    Some(snapshot_from_cache(&guard, fetched_at.elapsed()))
                } else {
                    None
                }
            })
        };
        if let Some(snapshot) = cached {
            return snapshot;
        }

        let url = format!("{}/voice/brain/manifest", core_base.trim_end_matches('/'));
        let fetched = http.get(url).timeout(Duration::from_secs(2)).send().await;
        let mut guard = self.inner.lock().expect("voice brain cache lock");
        match fetched {
            Ok(response) if response.status().is_success() => match response.json::<Value>().await {
                Ok(manifest) => {
                    guard.manifest = Some(manifest);
                    guard.fetched_at = Some(Instant::now());
                    guard.last_error = None;
                }
                Err(err) => guard.last_error = Some(format!("manifest_json: {err}")),
            },
            Ok(response) => {
                guard.last_error = Some(format!("manifest_http_{}", response.status().as_u16()))
            }
            Err(err) => guard.last_error = Some(format!("manifest_fetch: {err}")),
        }

        let age = guard.fetched_at.map(|fetched_at| fetched_at.elapsed());
        snapshot_from_cache(&guard, option_value(age))
    }
}

fn snapshot_from_cache(cache: &CachedManifest, age: Duration) -> VoiceBrainSnapshot {
    let manifest = cache.manifest.clone();
    VoiceBrainSnapshot {
        loaded: manifest.is_some(),
        manifest_hash: manifest
            .as_ref()
            .and_then(|value| string_field(value, "manifestHash")),
        cache_age_ms: cache.fetched_at.map(|_| age.as_millis()),
        generated_at: manifest
            .as_ref()
            .and_then(|value| string_field(value, "generatedAt")),
        last_error: cache.last_error.clone(),
        manifest,
    }
}

pub fn normalize_transcript(text: &str) -> String {
    let mut normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (alias, canonical) in [
        ("JPMC", "JMCP"),
        ("J P M C", "JMCP"),
        ("J M C P", "JMCP"),
        ("jay em see pee", "JMCP"),
        ("J P C M", "JPCM"),
        ("J C P", "JCP"),
        ("M C P", "MCP"),
        ("MPCs", "MCPs"),
        ("MPC", "MCP"),
        ("Jerry you", "Jeryu"),
        ("Jeri you", "Jeryu"),
        ("Jank your eye", "Jankurai"),
        ("Janko rye", "Jankurai"),
        ("J echo", "Jekko"),
        ("Gecko", "Jekko"),
        ("Jail gun", "Jailgun"),
        ("Z Y A L", "ZYAL"),
        ("zy all", "ZYAL"),
        ("zile", "ZYAL"),
        ("noccio router", "jnoccio-router"),
    ] {
        normalized = replace_case_insensitive(&normalized, alias, canonical);
    }
    normalized
}

pub fn system_prompt(snapshot: &VoiceBrainSnapshot) -> String {
    let manifest = snapshot
        .manifest
        .as_ref()
        .map_or_else(|| json!({"loaded": false}), compact_manifest_for_prompt);
    format!(
        "You are the JMCP voice command engine. Use only this manifest. \
Never invent commands, routes, adapters, MCP tools, Jeryu, Jekko, or shell actions. \
Never call shell/tools directly; all authority routes through jmcp-core HTTP APIs. \
For unknown or unsafe requests, say you can only use manifest commands. \
Keep replies short and TTS friendly.\nManifest: {}",
        result_or_lazy(serde_json::to_string(&manifest), || "{}".to_owned())
    )
}

pub async fn route_command(
    runtime: &BrainRuntime<'_>,
    snapshot: &VoiceBrainSnapshot,
    transcript: &str,
    classifier_enabled: bool,
) -> Result<Option<CommandOutcome>> {
    let normalized = normalize_transcript(transcript);
    let Some(command) = deterministic_command(snapshot, &normalized) else {
        if direct_tool_request(&normalized) {
            return Ok(Some(CommandOutcome {
                reply: "I cannot call tools, shell, adapters, or MCP directly. Use a JMCP core command.".to_owned(),
                tool_name: Some("voice_brain.refuse_direct_tool".to_owned()),
                tool_policy_decision: "blocked".to_owned(),
                confirmation_required: false,
                confidence: 1.0,
            }));
        }
        if classifier_enabled && looks_like_command_request(&normalized) {
            if let Some(command) = classify_command(runtime, snapshot, &normalized).await? {
                return execute_command(runtime, snapshot, &normalized, command).await.map(Some);
            }
        }
        return Ok(None);
    };
    execute_command(runtime, snapshot, &normalized, command)
        .await
        .map(Some)
}

fn deterministic_command(snapshot: &VoiceBrainSnapshot, transcript: &str) -> Option<CoreCommand> {
    let lower = transcript.to_ascii_lowercase();
    let text = lower.as_str();
    if text.trim().is_empty() {
        return None;
    }

    if contains_any(text, &["help", "list commands", "what commands"]) {
        return Some(local_manifest_command("voice_brain.help"));
    }
    if contains_any(text, &["direct tool", "call tool", "shell", "terminal command", "run bash"]) {
        return Some(local_refusal_command());
    }
    if contains_any(text, &["approve token", "approve approval token"]) {
        return token_command("approvals.approve_token", Method::POST, "/approvals/approve", text);
    }
    if contains_any(text, &["deny token", "reject token"]) {
        return token_command("approvals.deny_token", Method::POST, "/approvals/deny", text);
    }
    if contains_any(text, &["replay now", "run replay"]) {
        return Some(write_core_command("replay.now", Method::POST, "/replay", None, 0.98));
    }
    if contains_any(text, &["queue autonomous", "break action into microtasks"]) {
        if let Some(id) = catalog_id(snapshot, "autonomousActions", text) {
            return Some(write_core_command(
                "autonomous_actions.queue_microtasks",
                Method::POST,
                &format!("/autonomous-actions/{id}/queue-microtasks"),
                Some(json!({})),
                0.95,
            ));
        }
        return Some(local_clarification_command(
            "autonomous_actions.queue_microtasks",
            "Which autonomous action should I break into microtasks?",
        ));
    }
    if contains_any(text, &["run autonomous action", "start zyal action", "run full auto action"]) {
        if let Some(id) = catalog_id(snapshot, "autonomousActions", text) {
            return Some(write_core_command(
                "autonomous_actions.submit",
                Method::POST,
                &format!("/autonomous-actions/{id}/submit"),
                Some(json!({})),
                0.95,
            ));
        }
        return Some(local_clarification_command(
            "autonomous_actions.submit",
            "Which autonomous action should I run?",
        ));
    }
    if contains_any(text, &["run microtask", "submit microtask", "queue microtask"]) {
        if let Some(id) = catalog_id(snapshot, "microtasks", text) {
            return Some(write_core_command(
                "microtasks.submit",
                Method::POST,
                &format!("/microtasks/{id}/submit"),
                Some(json!({})),
                0.95,
            ));
        }
        return Some(local_clarification_command(
            "microtasks.submit",
            "Which microtask should I queue?",
        ));
    }
    if contains_any(text, &["voice turn", "lookup voice turn"]) {
        if let Some(id) = trailing_identifier(text, &["voice turn", "lookup voice turn"]) {
            return Some(read_core_command(
                "voice_turns.get",
                &format!("/voice/brain/turns/{id}"),
                0.93,
            ));
        }
    }
    if contains_any(text, &["work order status", "lookup work order"]) {
        if let Some(id) = first_identifier(text) {
            return Some(read_core_command(
                "work_orders.get",
                &format!("/work-orders/{id}"),
                0.93,
            ));
        }
    }
    if contains_any(text, &["list approvals", "pending approvals", "approvals"]) {
        return Some(read_core_command("approvals.list", "/approvals", 0.99));
    }
    if contains_any(text, &["list work orders", "work orders", "jobs"]) {
        return Some(read_core_command("work_orders.list", "/work-orders", 0.99));
    }
    if contains_any(text, &["list microtasks", "microtasks", "micro task catalog"]) {
        return Some(read_core_command("microtasks.list", "/microtasks", 0.99));
    }
    if contains_any(text, &["autonomous actions", "full auto actions", "zyal actions"]) {
        return Some(read_core_command(
            "autonomous_actions.list",
            "/autonomous-actions",
            0.99,
        ));
    }
    if contains_any(text, &["evidence", "show evidence"]) {
        return Some(read_core_command("evidence.list", "/evidence", 0.98));
    }
    if contains_any(text, &["replay status", "replay"]) {
        return Some(read_core_command("replay.status", "/replay", 0.98));
    }
    if contains_any(text, &["universe", "show universe"]) {
        return Some(read_core_command("universe.get", "/universe", 0.98));
    }
    if contains_any(text, &["adapters", "adapter health"]) {
        return Some(read_core_command("adapters.list", "/adapters", 0.98));
    }
    if contains_any(text, &["ecosystem", "jeryu ecosystem"]) {
        return Some(read_core_command("ecosystem.get", "/ecosystem", 0.98));
    }
    if contains_any(text, &["memory records", "memory"]) {
        return Some(read_core_command("memory.list", "/memory", 0.96));
    }
    if contains_any(text, &["effects", "effect ledger", "ledgers"]) {
        return Some(read_core_command("effects.list", "/effects", 0.96));
    }
    if contains_any(text, &["leases", "active leases"]) {
        return Some(read_core_command("leases.list", "/leases", 0.96));
    }
    if contains_any(text, &["runtime", "voice state", "control plane state"]) {
        return Some(read_core_command("runtime.summary", "/runtime", 0.96));
    }
    if contains_any(text, &["health", "status", "jmcp status", "core health"]) {
        return Some(read_core_command("status.health", "/health", 0.98));
    }
    None
}

async fn execute_command(
    runtime: &BrainRuntime<'_>,
    snapshot: &VoiceBrainSnapshot,
    transcript: &str,
    command: CoreCommand,
) -> Result<CommandOutcome> {
    if command.id == "voice_brain.help" {
        return Ok(CommandOutcome {
            reply: manifest_help_reply(snapshot),
            tool_name: Some(command.id),
            tool_policy_decision: "allowed".to_owned(),
            confirmation_required: false,
            confidence: command.confidence,
        });
    }
    if command.id == "voice_brain.refuse_direct_tool" {
        return Ok(CommandOutcome {
            reply: "I cannot call tools, shell, adapters, or MCP directly. Use a JMCP core command.".to_owned(),
            tool_name: Some(command.id),
            tool_policy_decision: "blocked".to_owned(),
            confirmation_required: false,
            confidence: command.confidence,
        });
    }
    if command.id.ends_with(".clarify") {
        return Ok(CommandOutcome {
            reply: command.route,
            tool_name: Some(command.id.trim_end_matches(".clarify").to_owned()),
            tool_policy_decision: "clarification_required".to_owned(),
            confirmation_required: false,
            confidence: command.confidence,
        });
    }
    if command.method != Method::GET && requires_confirmation(&command) && !explicitly_confirmed(transcript) {
        return Ok(CommandOutcome {
            reply: format!(
                "{} requires confirmation. Say confirm, then repeat the command.",
                spoken_command_name(&command.id)
            ),
            tool_name: Some(command.id),
            tool_policy_decision: "requires_confirmation".to_owned(),
            confirmation_required: true,
            confidence: command.confidence,
        });
    }

    let data = core_request(runtime, command.method.clone(), &command.route, command.body).await?;
    let reply = summarize_core_response(&command, &data);
    Ok(CommandOutcome {
        reply,
        tool_name: Some(command.id),
        tool_policy_decision: if command.method == Method::GET {
            "allowed_read_only".to_owned()
        } else {
            "allowed_after_confirmation".to_owned()
        },
        confirmation_required: false,
        confidence: command.confidence,
    })
}

async fn core_request(
    runtime: &BrainRuntime<'_>,
    method: Method,
    route: &str,
    body: Option<Value>,
) -> Result<Value> {
    let url = format!("{}{}", runtime.core_base.trim_end_matches('/'), route);
    let mut request = runtime.http.request(method, url).timeout(runtime.total_timeout);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.context("jmcp core request")?;
    let status = response.status();
    let text = response.text().await.context("jmcp core response body")?;
    if !status.is_success() {
        return Err(anyhow!("jmcp_core_http_{}: {}", status.as_u16(), text));
    }
    if text.trim().is_empty() {
        return Ok(json!({"ok": true}));
    }
    serde_json::from_str(&text).context("jmcp core json")
}

async fn classify_command(
    runtime: &BrainRuntime<'_>,
    snapshot: &VoiceBrainSnapshot,
    transcript: &str,
) -> Result<Option<CoreCommand>> {
    let Some(manifest) = snapshot.manifest.as_ref() else {
        return Ok(None);
    };
    let payload = json!({
        "model": runtime.llm_model,
        "messages": [
            {
                "role": "system",
                "content": format!(
                    "Classify a JMCP voice command. Return only JSON with command_id, method, route, body, confidence, or {{\"command_id\":\"none\"}}. Manifest: {}",
                    result_value(serde_json::to_string(&compact_manifest_for_prompt(manifest)))
                )
            },
            {"role": "user", "content": transcript}
        ],
        "stream": false,
        "temperature": 0,
        "max_tokens": 96
    });
    let response = runtime
        .http
        .post(format!("{}/chat/completions", runtime.llm_upstream.trim_end_matches('/')))
        .json(&payload)
        .timeout(runtime.total_timeout)
        .send()
        .await
        .context("voice brain classifier request")?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let data: Value = response.json().await.context("voice brain classifier json")?;
    let content = data["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .and_then(|choice| match choice["message"]["content"].as_str() {
            Some(content) => Some(content),
            None => choice["text"].as_str(),
        });
    let content = option_value(content);
    let parsed: Value = match serde_json::from_str(content.trim()) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    command_from_classifier(snapshot, &parsed, transcript)
}

fn command_from_classifier(
    snapshot: &VoiceBrainSnapshot,
    parsed: &Value,
    transcript: &str,
) -> Result<Option<CoreCommand>> {
    let command_id = paired_string_field(parsed, "command_id", "commandId");
    let command_id = option_value(command_id);
    if command_id.is_empty() || command_id == "none" {
        return Ok(None);
    }
    let confidence = number_field(parsed, "confidence")
        .map(|value| value as f32)
        .unwrap_or(0.75);
    let Some(command) = manifest_command(snapshot, &command_id) else {
        return Ok(None);
    };
    let method = option_or_lazy(string_field(&command, "method"), || "GET".to_owned());
    let route = option_value(string_field(&command, "route"));
    if route.contains("{id}") {
        if command_id.contains("microtasks") {
            if let Some(id) = catalog_id(snapshot, "microtasks", transcript) {
                return Ok(Some(write_core_command(
                    &command_id,
                    Method::POST,
                    &route.replace("{id}", &id),
                    Some(json!({})),
                    confidence,
                )));
            }
        }
        if command_id.contains("autonomous_actions") {
            if let Some(id) = catalog_id(snapshot, "autonomousActions", transcript) {
                return Ok(Some(write_core_command(
                    &command_id,
                    Method::POST,
                    &route.replace("{id}", &id),
                    Some(json!({})),
                    confidence,
                )));
            }
        }
        return Ok(None);
    }
    let method = if method == "POST" {
        Method::POST
    } else {
        Method::GET
    };
    let body = parsed.get("body").cloned().filter(|value| !value.is_null());
    Ok(Some(CoreCommand {
        id: command_id,
        method,
        route,
        body,
        risk_class: option_or_lazy(string_field(&command, "riskClass"), || "read_only".to_owned()),
        confirmation_policy: option_or_lazy(string_field(&command, "confirmationPolicy"), || {
            "none".to_owned()
        }),
        confidence,
    }))
}

fn read_core_command(id: &str, route: &str, confidence: f32) -> CoreCommand {
    CoreCommand {
        id: id.to_owned(),
        method: Method::GET,
        route: route.to_owned(),
        body: None,
        risk_class: "read_only".to_owned(),
        confirmation_policy: "none".to_owned(),
        confidence,
    }
}

fn write_core_command(
    id: &str,
    method: Method,
    route: &str,
    body: Option<Value>,
    confidence: f32,
) -> CoreCommand {
    CoreCommand {
        id: id.to_owned(),
        method,
        route: route.to_owned(),
        body,
        risk_class: "durable_state_mutation".to_owned(),
        confirmation_policy: "explicit confirmation required".to_owned(),
        confidence,
    }
}

fn token_command(id: &str, method: Method, route: &str, text: &str) -> Option<CoreCommand> {
    let token = trailing_identifier(text, &["approve token", "approve approval token", "deny token", "reject token"])?;
    Some(CoreCommand {
        id: id.to_owned(),
        method,
        route: route.to_owned(),
        body: Some(json!({"token": token, "approver": "voice"})),
        risk_class: "approval_token".to_owned(),
        confirmation_policy: "spoken approval token required".to_owned(),
        confidence: 0.98,
    })
}

fn local_manifest_command(id: &str) -> CoreCommand {
    CoreCommand {
        id: id.to_owned(),
        method: Method::GET,
        route: String::new(),
        body: None,
        risk_class: "read_only".to_owned(),
        confirmation_policy: "none".to_owned(),
        confidence: 1.0,
    }
}

fn local_refusal_command() -> CoreCommand {
    CoreCommand {
        id: "voice_brain.refuse_direct_tool".to_owned(),
        method: Method::GET,
        route: String::new(),
        body: None,
        risk_class: "blocked".to_owned(),
        confirmation_policy: "blocked".to_owned(),
        confidence: 1.0,
    }
}

fn local_clarification_command(id: &str, reply: &str) -> CoreCommand {
    CoreCommand {
        id: format!("{id}.clarify"),
        method: Method::GET,
        route: reply.to_owned(),
        body: None,
        risk_class: "clarification".to_owned(),
        confirmation_policy: "clarification required".to_owned(),
        confidence: 0.75,
    }
}

fn summarize_core_response(command: &CoreCommand, data: &Value) -> String {
    match command.id.as_str() {
        "status.health" => {
            let ok = bool_field(data, "ok").unwrap_or(false);
            let systems = data["systems"].as_array().map_or(0, Vec::len);
            if ok {
                format!("JMCP core is healthy. {systems} systems are visible.")
            } else {
                "JMCP core responded, but it is not healthy.".to_owned()
            }
        }
        "runtime.summary" => {
            let work_orders = data["workOrders"].as_array().map_or(0, Vec::len);
            let approvals = data["approvals"].as_array().map_or(0, Vec::len);
            format!("Runtime is visible. {work_orders} work orders and {approvals} approvals are listed.")
        }
        "work_orders.list" => summarize_array(data, "work orders"),
        "approvals.list" => summarize_array(data, "approvals"),
        "microtasks.list" => summarize_named_array(data, "microtasks"),
        "autonomous_actions.list" => summarize_named_array(data, "autonomous actions"),
        "evidence.list" => summarize_array(data, "evidence records"),
        "memory.list" => summarize_array(data, "memory records"),
        "leases.list" => summarize_array(data, "leases"),
        "replay.status" => {
            let events = data["events"].as_u64().unwrap_or(0);
            let checkpoints = data["checkpoints"].as_array().map_or(0, Vec::len);
            format!("Replay has {events} events and {checkpoints} checkpoints.")
        }
        "replay.now" => "Replay was requested through JMCP core.".to_owned(),
        "universe.get" => "The JMCP universe state is available.".to_owned(),
        "adapters.list" => {
            let health = data["health"].as_array().map_or(0, Vec::len);
            format!("{health} adapters are visible.")
        }
        "ecosystem.get" => {
            let tools = data["tools"].as_array().map_or(0, Vec::len);
            format!("The ecosystem lists {tools} tools.")
        }
        id if id.starts_with("microtasks.submit") => summarize_work_order(data, "Microtask queued"),
        id if id.starts_with("autonomous_actions.submit") => {
            summarize_work_order(data, "Autonomous action queued")
        }
        id if id.starts_with("autonomous_actions.queue_microtasks") => summarize_array(data, "child work orders"),
        id if id.starts_with("approvals.approve_token") => "Approval token was approved through JMCP core.".to_owned(),
        id if id.starts_with("approvals.deny_token") => "Approval token was denied through JMCP core.".to_owned(),
        _ => "JMCP core completed the command.".to_owned(),
    }
}

fn summarize_array(data: &Value, label: &str) -> String {
    let count = data.as_array().map_or(0, Vec::len);
    format!("There are {count} {label}.")
}

fn summarize_named_array(data: &Value, label: &str) -> String {
    let Some(items) = data.as_array() else {
        return format!("The {label} catalog is available.");
    };
    let names = items
        .iter()
        .filter_map(|item| paired_string_field(item, "id", "name"))
        .take(3)
        .collect::<Vec<_>>();
    if names.is_empty() {
        format!("There are {} {label}.", items.len())
    } else {
        format!("There are {} {label}. First: {}.", items.len(), names.join(", "))
    }
}

fn summarize_work_order(data: &Value, prefix: &str) -> String {
    let id = string_field(data, "id")
        .map(|value| short_identifier(&value));
    let id = option_or_lazy(id, || "unknown".to_owned());
    format!("{prefix}. Work order ending {id}.")
}

fn manifest_help_reply(snapshot: &VoiceBrainSnapshot) -> String {
    let Some(manifest) = snapshot.manifest.as_ref() else {
        return "Voice brain manifest is not loaded yet.".to_owned();
    };
    let commands = manifest["commands"].as_array().map_or(0, Vec::len);
    let microtasks = manifest["microtasks"].as_array().map_or(0, Vec::len);
    let actions = manifest["autonomousActions"].as_array().map_or(0, Vec::len);
    format!("I know {commands} JMCP commands, {microtasks} microtasks, and {actions} autonomous actions.")
}

fn requires_confirmation(command: &CoreCommand) -> bool {
    command.risk_class != "read_only"
        && command.risk_class != "approval_token"
        && command.confirmation_policy != "none"
}

fn explicitly_confirmed(text: &str) -> bool {
    contains_any(
        &text.to_ascii_lowercase(),
        &["confirm", "confirmed", "with confirmation"],
    )
}

fn direct_tool_request(text: &str) -> bool {
    contains_any(
        &text.to_ascii_lowercase(),
        &[
            "call jeryu directly",
            "call jekko directly",
            "call mcp",
            "run shell",
            "shell command",
            "execute bash",
        ],
    )
}

fn looks_like_command_request(text: &str) -> bool {
    contains_any(
        &text.to_ascii_lowercase(),
        &[
            "show", "list", "status", "approve", "deny", "run", "queue", "lookup", "check",
            "replay", "evidence", "memory", "lease", "adapter", "ecosystem", "universe",
        ],
    )
}

fn catalog_id(snapshot: &VoiceBrainSnapshot, field: &str, text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    snapshot
        .manifest
        .as_ref()?
        .get(field)?
        .as_array()?
        .iter()
        .find_map(|item| {
            let id = string_field(item, "id")?;
            let title = option_value(string_field(item, "title"));
            let id_words = id.replace(['.', '-', '_'], " ");
            if lower.contains(&id.to_ascii_lowercase())
                || lower.contains(&title.to_ascii_lowercase())
                || lower.contains(&id_words.to_ascii_lowercase())
            {
                Some(id)
            } else {
                None
            }
        })
}

fn manifest_command(snapshot: &VoiceBrainSnapshot, command_id: &str) -> Option<Value> {
    snapshot
        .manifest
        .as_ref()?
        .get("commands")?
        .as_array()?
        .iter()
        .find(|command| string_field(command, "id").as_deref() == Some(command_id))
        .cloned()
}

fn compact_manifest_for_prompt(manifest: &Value) -> Value {
    json!({
        "manifestHash": manifest["manifestHash"].clone(),
        "commands": compact_id_route_array(&manifest["commands"]),
        "microtasks": compact_id_array(&manifest["microtasks"]),
        "autonomousActions": compact_id_array(&manifest["autonomousActions"]),
        "terms": manifest["terms"].clone(),
        "spokenAliases": manifest["spokenAliases"].clone(),
        "toolPolicy": manifest["toolPolicy"].clone()
    })
}

fn compact_id_route_array(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|item| {
                json!({
                    "id": item["id"].clone(),
                    "method": item["method"].clone(),
                    "route": item["route"].clone(),
                    "riskClass": item["riskClass"].clone(),
                    "confirmationPolicy": item["confirmationPolicy"].clone(),
                    "aliases": item["aliases"].clone()
                })
            })
            .collect(),
    )
}

fn compact_id_array(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|item| {
                json!({
                    "id": item["id"].clone(),
                    "title": item["title"].clone()
                })
            })
            .collect(),
    )
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn trailing_identifier(text: &str, phrases: &[&str]) -> Option<String> {
    for phrase in phrases {
        if let Some(index) = text.find(phrase) {
            let after = text[index + phrase.len()..].trim();
            if let Some(id) = after
                .split_whitespace()
                .map(clean_identifier)
                .find(|part| !part.is_empty())
            {
                return Some(id);
            }
        }
    }
    first_identifier(text)
}

fn first_identifier(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(clean_identifier)
        .find(|part| {
            part.len() >= 6
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        })
}

fn clean_identifier(value: &str) -> String {
    value
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && !matches!(ch, '-' | '_' | '.'))
        .to_owned()
}

fn short_identifier(value: &str) -> String {
    value
        .chars()
        .rev()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn spoken_command_name(id: &str) -> String {
    id.replace(['_', '.'], " ")
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn paired_string_field(value: &Value, primary: &str, secondary: &str) -> Option<String> {
    match string_field(value, primary) {
        Some(value) => Some(value),
        None => string_field(value, secondary),
    }
}

fn option_value<T: Default>(value: Option<T>) -> T {
    match value {
        Some(value) => value,
        None => T::default(),
    }
}

fn result_value<T: Default, E>(value: std::result::Result<T, E>) -> T {
    match value {
        Ok(value) => value,
        Err(_) => T::default(),
    }
}

fn result_or_lazy<T, E, F>(value: std::result::Result<T, E>, replacement: F) -> T
where
    F: FnOnce() -> T,
{
    match value {
        Ok(value) => value,
        Err(_) => replacement(),
    }
}

fn option_or_lazy<T, F>(value: Option<T>, replacement: F) -> T
where
    F: FnOnce() -> T,
{
    match value {
        Some(value) => value,
        None => replacement(),
    }
}

fn number_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn replace_case_insensitive(text: &str, needle: &str, replacement: &str) -> String {
    let mut output = String::new();
    let mut rest = text;
    let needle_lower = needle.to_ascii_lowercase();
    while let Some(index) = rest.to_ascii_lowercase().find(&needle_lower) {
        output.push_str(&rest[..index]);
        output.push_str(replacement);
        rest = &rest[index + needle.len()..];
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::get, routing::post, Json, Router};
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    fn manifest() -> Value {
        json!({
            "manifestHash": "sha256:test",
            "commands": [
                {"id": "status.health", "method": "GET", "route": "/health", "riskClass": "read_only", "confirmationPolicy": "none", "aliases": ["health"]},
                {"id": "approvals.approve_token", "method": "POST", "route": "/approvals/approve", "riskClass": "approval_token", "confirmationPolicy": "spoken approval token required", "aliases": ["approve token"]},
                {"id": "microtasks.submit", "method": "POST", "route": "/microtasks/{id}/submit", "riskClass": "durable_state_mutation", "confirmationPolicy": "explicit confirmation required", "aliases": ["run microtask"]}
            ],
            "microtasks": [
                {"id": "router.tool-build-probe", "title": "Router Tool Build Probe"}
            ],
            "autonomousActions": [],
            "terms": [],
            "spokenAliases": {},
            "toolPolicy": {}
        })
    }

    fn snapshot() -> VoiceBrainSnapshot {
        VoiceBrainSnapshot {
            loaded: true,
            manifest_hash: Some("sha256:test".to_owned()),
            cache_age_ms: Some(0),
            generated_at: None,
            last_error: None,
            manifest: Some(manifest()),
        }
    }

    #[test]
    fn normalizes_jmcp_specific_mishearings() {
        let normalized = normalize_transcript(
            "JPMC should ask Jerry you and J echo about Z Y A L and MPC tools",
        );
        assert!(normalized.contains("JMCP"));
        assert!(normalized.contains("Jeryu"));
        assert!(normalized.contains("Jekko"));
        assert!(normalized.contains("ZYAL"));
        assert!(normalized.contains("MCP tools"));
    }

    #[tokio::test]
    async fn read_only_command_calls_core_route() {
        let app = Router::new()
            .route(
                "/health",
                get(|| async { Json(json!({"ok": true, "systems": [{}, {}]})) }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = Client::builder().build().unwrap();
        let runtime = BrainRuntime {
            http: &http,
            core_base: &format!("http://{addr}"),
            llm_upstream: "http://127.0.0.1:1/v1",
            llm_model: "local/test",
            total_timeout: Duration::from_secs(2),
        };

        let outcome = route_command(&runtime, &snapshot(), "what is JMCP status", false)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(outcome.tool_name.as_deref(), Some("status.health"));
        assert_eq!(outcome.tool_policy_decision, "allowed_read_only");
        assert!(outcome.reply.contains("2 systems"));
        server.abort();
    }

    #[tokio::test]
    async fn mutating_microtask_requires_confirmation_before_core_call() {
        let seen = Arc::new(Mutex::new(0_u32));
        let app = Router::new()
            .route(
                "/microtasks/router.tool-build-probe/submit",
                post(
                    |State(seen): State<Arc<Mutex<u32>>>| async move {
                        *seen.lock().expect("seen lock") += 1;
                        Json(json!({"id": "wo-test"}))
                    },
                ),
            )
            .with_state(Arc::clone(&seen));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = Client::builder().build().unwrap();
        let runtime = BrainRuntime {
            http: &http,
            core_base: &format!("http://{addr}"),
            llm_upstream: "http://127.0.0.1:1/v1",
            llm_model: "local/test",
            total_timeout: Duration::from_secs(2),
        };

        let outcome = route_command(
            &runtime,
            &snapshot(),
            "run microtask router tool build probe",
            false,
        )
        .await
        .unwrap()
        .unwrap();

        assert!(outcome.confirmation_required);
        assert_eq!(outcome.tool_policy_decision, "requires_confirmation");
        assert_eq!(*seen.lock().expect("seen lock"), 0);
        server.abort();
    }

    #[tokio::test]
    async fn confirmed_microtask_routes_through_core() {
        let app = Router::new().route(
            "/microtasks/router.tool-build-probe/submit",
            post(|| async { Json(json!({"id": "11111111-1111-4111-8111-111111111111"})) }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let http = Client::builder().build().unwrap();
        let runtime = BrainRuntime {
            http: &http,
            core_base: &format!("http://{addr}"),
            llm_upstream: "http://127.0.0.1:1/v1",
            llm_model: "local/test",
            total_timeout: Duration::from_secs(2),
        };

        let outcome = route_command(
            &runtime,
            &snapshot(),
            "confirm run microtask router tool build probe",
            false,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(outcome.tool_name.as_deref(), Some("microtasks.submit"));
        assert_eq!(outcome.tool_policy_decision, "allowed_after_confirmation");
        assert!(outcome.reply.contains("Work order ending 111111"));
        server.abort();
    }
}
