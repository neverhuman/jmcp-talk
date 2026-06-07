use anyhow::{anyhow, Context, Result};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::mpsc;
use uuid::Uuid;

mod core_turns;

use core_turns::{
    update_turn_record, CoreVoiceTurnRecorder, SharedVoiceTurnRecord, VoiceTurnRecord,
    VoiceTurnState,
};

const DEFAULT_BIND: &str = "127.0.0.1:8040";
const DEFAULT_LLM_UPSTREAM: &str = "http://127.0.0.1:18902/v1";
const DEFAULT_LLM_MODEL: &str = "local/qwen2.5-7b-instruct-awq";
const DEFAULT_ASR_UPSTREAM: &str = "http://127.0.0.1:18878";
const DEFAULT_TTS_UPSTREAM: &str = "http://127.0.0.1:18901";
const DEFAULT_CORE_UPSTREAM: &str = "http://127.0.0.1:18877";
const DEFAULT_AUDIO_DIR: &str = "/home/ubuntu/jmcp-split/.live/audio";
const DEFAULT_EVENT_LOG: &str = "/home/ubuntu/jmcp-split/.live/logs/voice-events.jsonl";
const DEFAULT_PROFILE_PATH: &str =
    "/home/ubuntu/jmcp-split/jmcp-talk/services/speech/voice_profiles/jmcp_male_v1.json";
const INPUT_SAMPLE_RATE: u32 = 16_000;
const DEFAULT_OUTPUT_SAMPLE_RATE: u32 = 48_000;

#[derive(Clone)]
struct AppState {
    config: Arc<VoiceConfig>,
    http: Client,
    metrics: Arc<Mutex<Metrics>>,
    turn_recorder: CoreVoiceTurnRecorder,
}

#[derive(Clone)]
struct VoiceConfig {
    bind: SocketAddr,
    bind_text: String,
    llm_upstream: String,
    llm_model: String,
    asr_upstream: String,
    tts_upstream: String,
    jmcp_core: String,
    event_log: PathBuf,
    audio_dir: PathBuf,
    capture_raw_audio: bool,
    audio_retention_days: Option<u64>,
    audio_max_mb: Option<u64>,
    voice_profile: String,
    voice_profile_path: PathBuf,
    voice_profile_hash: String,
    min_tts_phrase_chars: usize,
    record_core_turns: bool,
    core_record_timeout: Duration,
    connect_timeout: Duration,
    first_frame_timeout: Duration,
    idle_timeout: Duration,
    total_turn_timeout: Duration,
}

#[derive(Default)]
struct Metrics {
    turns_total: u64,
    failures: Vec<(String, u64)>,
    first_token_latency_ms: Vec<f64>,
    first_audio_latency_ms: Vec<f64>,
    tts_rtf: Vec<f64>,
    dropped_frames: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Arc::new(VoiceConfig::from_env()?);
    prune_audio_retention(&config);
    let http = Client::builder().build()?;
    let turn_recorder = CoreVoiceTurnRecorder::new(
        config.jmcp_core.clone(),
        config.record_core_turns,
        config.core_record_timeout,
        http.clone(),
    );
    let state = AppState {
        config: Arc::clone(&config),
        http,
        metrics: Arc::new(Mutex::new(Metrics::default())),
        turn_recorder,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_response))
        .route("/events", post(events))
        .route("/ws/chat", get(chat_ws))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    println!(
        "[jmcp-voiced] listening on {} (profile={}, asr={}, tts={}, llm={})",
        config.bind_text,
        config.voice_profile,
        config.asr_upstream,
        config.tts_upstream,
        config.llm_upstream
    );
    axum::serve(listener, app).await?;
    Ok(())
}

impl VoiceConfig {
    fn from_env() -> Result<Self> {
        let bind_text = env_or("JMCP_TALK_VOICE_BIND", DEFAULT_BIND);
        let bind = bind_text
            .parse::<SocketAddr>()
            .with_context(|| format!("parse JMCP_TALK_VOICE_BIND={bind_text}"))?;
        let profile_path =
            PathBuf::from(env_or("JMCP_TALK_VOICE_PROFILE_PATH", DEFAULT_PROFILE_PATH));
        let audio_dir = PathBuf::from(env_or("JMCP_TALK_AUDIO_DIR", DEFAULT_AUDIO_DIR));
        Ok(Self {
            bind,
            bind_text,
            llm_upstream: trim_url(env_or("JMCP_TALK_LLM_UPSTREAM", DEFAULT_LLM_UPSTREAM)),
            llm_model: env_or("JMCP_TALK_LLM_MODEL", DEFAULT_LLM_MODEL),
            asr_upstream: trim_url(env_or("JMCP_TALK_ASR_UPSTREAM", DEFAULT_ASR_UPSTREAM)),
            tts_upstream: trim_url(env_or("JMCP_TALK_TTS_UPSTREAM", DEFAULT_TTS_UPSTREAM)),
            jmcp_core: trim_url(env_or("JMCP_API_URL", DEFAULT_CORE_UPSTREAM)),
            event_log: PathBuf::from(env_or("JMCP_TALK_VOICE_EVENT_LOG", DEFAULT_EVENT_LOG)),
            audio_dir,
            capture_raw_audio: env_bool("JMCP_TALK_CAPTURE_RAW_AUDIO", true),
            audio_retention_days: env_u64("JMCP_TALK_AUDIO_RETENTION_DAYS", Some(7)),
            audio_max_mb: env_u64("JMCP_TALK_AUDIO_MAX_MB", Some(2048)),
            voice_profile: env_or("JMCP_TALK_VOICE_PROFILE", "jmcp_male_v1"),
            voice_profile_hash: result_value(load_profile_hash(&profile_path)),
            voice_profile_path: profile_path,
            min_tts_phrase_chars: env_usize("JMCP_TALK_MIN_TTS_PHRASE_CHARS", 32),
            record_core_turns: env_bool("JMCP_TALK_RECORD_CORE_TURNS", true),
            core_record_timeout: env_duration_ms("JMCP_TALK_CORE_RECORD_TIMEOUT_MS", 250),
            connect_timeout: env_duration_ms("JMCP_TALK_VOICE_CONNECT_TIMEOUT_MS", 10_000),
            first_frame_timeout: env_duration_ms("JMCP_TALK_VOICE_FIRST_FRAME_TIMEOUT_MS", 45_000),
            idle_timeout: env_duration_ms("JMCP_TALK_VOICE_IDLE_TIMEOUT_MS", 15_000),
            total_turn_timeout: env_duration_ms("JMCP_TALK_VOICE_TOTAL_TURN_TIMEOUT_MS", 120_000),
        })
    }

    fn local_only(&self) -> bool {
        self.bind.ip().is_loopback()
    }

    fn llm_health_url(&self) -> String {
        self.llm_upstream
            .strip_suffix("/v1")
            .unwrap_or(&self.llm_upstream)
            .to_owned()
            + "/health"
    }
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let tts_health = get_json(&state, &format!("{}/health", state.config.tts_upstream)).await;
    let asr_health = get_json(&state, &format!("{}/health", state.config.asr_upstream)).await;
    let llm_health = get_json(&state, &state.config.llm_health_url()).await;
    let tts_ok = endpoint_ok(&tts_health);
    let asr_ok = endpoint_ok(&asr_health);
    let llm_ok = endpoint_ok(&llm_health);
    let ok = tts_ok && asr_ok && llm_ok;
    let metrics = state.metrics.lock().expect("metrics lock");
    let sample_rate = number_field(&tts_health, "sample_rate")
        .unwrap_or(DEFAULT_OUTPUT_SAMPLE_RATE as f64) as u32;
    let payload = json!({
        "ok": ok,
        "adapter": "jmcp-rust-voice",
        "engine": "local-asr-llm-tts",
        "voice_engine": option_or_lazy(
            paired_string_field(&tts_health, "voice_engine", "active_engine"),
            || "voxcpm2".to_owned(),
        ),
        "voice_profile": option_or_lazy(string_field(&tts_health, "voice_profile"), || state.config.voice_profile.clone()),
        "voice_profile_hash": option_or_lazy(
            string_field(&tts_health, "voice_profile_hash").filter(|hash| !hash.is_empty()),
            || state.config.voice_profile_hash.clone(),
        ),
        "voice_profile_path": state.config.voice_profile_path,
        "sample_rate": sample_rate,
        "audio_format": "f32le",
        "streaming_audio": true,
        "tts_rtf_p50": round4(p50(&metrics.tts_rtf)),
        "degraded_active": bool_field(&tts_health, "degraded_active").unwrap_or(false),
        "degraded_engine": value_or_null(&tts_health, "degraded_engine"),
        "local_only": state.config.local_only(),
        "loaded": ok,
        "llm_model": state.config.llm_model,
        "llm_upstream": state.config.llm_upstream,
        "asr_upstream": state.config.asr_upstream,
        "tts_upstream": state.config.tts_upstream,
        "jmcp_core": state.config.jmcp_core,
        "raw_audio_capture": state.config.capture_raw_audio,
        "audio_dir": if state.config.capture_raw_audio { Some(state.config.audio_dir.display().to_string()) } else { None },
        "audio_retention_days": state.config.audio_retention_days,
        "audio_max_mb": state.config.audio_max_mb,
        "first_frame_timeout_ms": state.config.first_frame_timeout.as_millis(),
        "tts_health": tts_health,
        "asr_health": asr_health,
        "llm_health": llm_health,
    });
    (
        if ok {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(payload),
    )
}

async fn metrics_response(State(state): State<AppState>) -> impl IntoResponse {
    let metrics = state.metrics.lock().expect("metrics lock");
    let mut lines = vec![
        format!("jmcp_voice_turns_total {}", metrics.turns_total),
        format!(
            "jmcp_voice_first_audio_latency_ms_p50 {:.1}",
            p50(&metrics.first_audio_latency_ms)
        ),
        format!(
            "jmcp_voice_first_token_latency_ms_p50 {:.1}",
            p50(&metrics.first_token_latency_ms)
        ),
        format!("jmcp_voice_tts_rtf_p50 {:.4}", p50(&metrics.tts_rtf)),
        format!("jmcp_voice_dropped_frames_total {}", metrics.dropped_frames),
    ];
    for (stage, count) in &metrics.failures {
        lines.push(format!(
            "jmcp_voice_failures_total{{stage=\"{}\"}} {}",
            stage, count
        ));
    }
    lines.push(String::new());
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        lines.join("\n"),
    )
}

async fn events(State(state): State<AppState>, Json(payload): Json<Value>) -> Json<Value> {
    match payload {
        Value::Array(items) => {
            for item in items {
                if item.is_object() {
                    write_event(&state, &item);
                }
            }
        }
        Value::Object(_) => write_event(&state, &payload),
        _ => {}
    }
    Json(json!({"ok": true}))
}

async fn chat_ws(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_chat_socket(socket, state))
}

async fn handle_chat_socket(mut socket: WebSocket, state: AppState) {
    let started = Instant::now();
    let mut turn_id = new_turn_id();
    increment_turns(&state);

    let first = match tokio::time::timeout(state.config.connect_timeout, socket.recv()).await {
        Ok(Some(Ok(message))) => message,
        Ok(Some(Err(_))) | Ok(None) => {
            metric_fail(&state, "connect");
            return;
        }
        Err(_) => {
            metric_fail(&state, "connect_timeout");
            let _ = send_socket_json(
                &mut socket,
                json!({"type": "error", "turn_id": turn_id, "error": "connect_timeout"}),
            )
            .await;
            return;
        }
    };

    let Some(first_value) = parse_ws_message(first) else {
        metric_fail(&state, "parse");
        let _ = send_socket_json(
            &mut socket,
            json!({"type": "error", "turn_id": turn_id, "error": "invalid_json_frame"}),
        )
        .await;
        return;
    };

    if let Some(request_turn_id) = string_field(&first_value, "turn_id").filter(|id| !id.is_empty())
    {
        turn_id = request_turn_id;
    }

    let turn_record: SharedVoiceTurnRecord = Arc::new(Mutex::new(VoiceTurnRecord::new(
        turn_id.clone(),
        None,
        "voice",
        "jmcp-voiced",
        Some(state.config.voice_profile_hash.clone()),
    )));
    let started_snapshot = update_turn_record(&turn_record, |turn| {
        turn.state = VoiceTurnState::Started;
    });
    state.turn_recorder.submit_background(&started_snapshot);

    let request = if string_field(&first_value, "type").as_deref() == Some("turn.start") {
        match collect_realtime_turn(
            &mut socket,
            &state,
            turn_record.clone(),
            turn_id.clone(),
            first_value,
        )
        .await
        {
            Ok(Some((collected_turn_id, request))) => {
                turn_id = collected_turn_id;
                request
            }
            Ok(None) => return,
            Err(err) => {
                metric_fail(&state, "realtime_frame");
                let failed_snapshot = update_turn_record(&turn_record, |turn| {
                    turn.state = VoiceTurnState::Failed;
                    turn.error_class = Some(err.to_string());
                    turn.completed_at = Some(Utc::now());
                });
                state.turn_recorder.submit_final(&failed_snapshot).await;
                write_event(
                    &state,
                    &json!({
                        "turn_id": turn_id,
                        "event": "voice.error",
                        "stage": "realtime_frame",
                        "status": "error",
                        "error_class": err.to_string(),
                    }),
                );
                let _ = send_socket_json(
                    &mut socket,
                    json!({"type": "error", "turn_id": turn_id, "error": "realtime_frame_error"}),
                )
                .await;
                return;
            }
        }
    } else {
        first_value
    };

    process_turn(socket, state, turn_record, started, turn_id, request).await;
}

async fn collect_realtime_turn(
    socket: &mut WebSocket,
    state: &AppState,
    turn_record: SharedVoiceTurnRecord,
    mut turn_id: String,
    first: Value,
) -> Result<Option<(String, Value)>> {
    if let Some(id) = string_field(&first, "turn_id").filter(|id| !id.is_empty()) {
        turn_id = id;
        let synced = turn_id.clone();
        let _ = update_turn_record(&turn_record, |turn| {
            turn.turn_id = synced;
        });
    }
    write_event(
        state,
        &json!({
            "turn_id": turn_id,
            "event": "voice.turn_started",
            "stage": "gateway",
        }),
    );

    let mut audio_chunks = Vec::<Vec<u8>>::new();
    let mut text_parts = Vec::<String>::new();
    let mut sample_rate = INPUT_SAMPLE_RATE;

    loop {
        let message = match tokio::time::timeout(state.config.idle_timeout, socket.recv()).await {
            Ok(Some(Ok(message))) => message,
            Ok(Some(Err(_))) | Ok(None) => {
                let interrupted_snapshot = update_turn_record(&turn_record, |turn| {
                    turn.state = VoiceTurnState::Interrupted;
                    turn.error_class = Some("client_closed".to_owned());
                    turn.completed_at = Some(Utc::now());
                });
                state
                    .turn_recorder
                    .submit_final(&interrupted_snapshot)
                    .await;
                return Ok(None);
            }
            Err(_) => return Err(anyhow!("realtime idle timeout")),
        };
        let Some(frame) = parse_ws_message(message) else {
            continue;
        };
        let frame_type = option_value(string_field(&frame, "type"));
        if let Some(id) = string_field(&frame, "turn_id").filter(|id| !id.is_empty()) {
            turn_id = id;
            let synced = turn_id.clone();
            let _ = update_turn_record(&turn_record, |turn| {
                turn.turn_id = synced;
            });
        }
        match frame_type.as_str() {
            "input.audio" => {
                if let Some(rate) = number_field(&frame, "sample_rate") {
                    sample_rate = rate as u32;
                }
                let encoded = paired_string_field(&frame, "audio_data", "data");
                let encoded = option_value(encoded);
                if !encoded.is_empty() {
                    let bytes = BASE64
                        .decode(encoded.as_bytes())
                        .context("decode input.audio")?;
                    write_event(
                        state,
                        &json!({
                            "turn_id": turn_id,
                            "event": "voice.input_audio",
                            "stage": "browser",
                            "bytes": bytes.len(),
                            "sample_rate": sample_rate,
                        }),
                    );
                    audio_chunks.push(bytes);
                }
            }
            "turn.end" => {
                if let Some(text) =
                    string_field(&frame, "text").filter(|text| !text.trim().is_empty())
                {
                    text_parts.push(text);
                }
                return Ok(Some((
                    turn_id.clone(),
                    build_realtime_request(&turn_id, &audio_chunks, &text_parts),
                )));
            }
            "barge_in" => {
                write_event(
                    state,
                    &json!({
                        "turn_id": turn_id,
                        "event": "voice.barge_in",
                        "stage": "browser",
                        "barge_in": true,
                    }),
                );
                let interrupted_snapshot = update_turn_record(&turn_record, |turn| {
                    turn.state = VoiceTurnState::Interrupted;
                    turn.error_class = Some("barge_in".to_owned());
                    turn.completed_at = Some(Utc::now());
                });
                state
                    .turn_recorder
                    .submit_final(&interrupted_snapshot)
                    .await;
                send_socket_json(
                    socket,
                    json!({"type": "done", "turn_id": turn_id, "interrupted": true, "text": ""}),
                )
                .await?;
                return Ok(None);
            }
            _ => {}
        }
    }
}

async fn process_turn(
    socket: WebSocket,
    state: AppState,
    turn_record: SharedVoiceTurnRecord,
    started: Instant,
    turn_id: String,
    request: Value,
) {
    let (out_tx, out_rx) = mpsc::channel::<Value>(128);
    let writer = tokio::spawn(write_socket(socket, out_rx));
    let result = run_turn(
        &state,
        &turn_record,
        &turn_id,
        started,
        request,
        out_tx.clone(),
    )
    .await;
    if let Err(err) = result {
        metric_fail(&state, "gateway");
        let failed_snapshot = update_turn_record(&turn_record, |turn| {
            turn.state = VoiceTurnState::Failed;
            turn.error_class = Some(err.to_string());
            turn.completed_at = Some(Utc::now());
        });
        state.turn_recorder.submit_final(&failed_snapshot).await;
        write_event(
            &state,
            &json!({
                "turn_id": turn_id,
                "event": "voice.error",
                "stage": "gateway",
                "status": "error",
                "error_class": err.to_string(),
            }),
        );
        let _ = out_tx
            .send(json!({"type": "error", "turn_id": turn_id, "error": err.to_string()}))
            .await;
    }
    drop(out_tx);
    let _ = writer.await;
}

async fn write_socket(mut socket: WebSocket, mut out_rx: mpsc::Receiver<Value>) {
    while let Some(payload) = out_rx.recv().await {
        if socket
            .send(Message::Text(payload.to_string()))
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn run_turn(
    state: &AppState,
    turn_record: &SharedVoiceTurnRecord,
    turn_id: &str,
    started: Instant,
    request: Value,
    out_tx: mpsc::Sender<Value>,
) -> Result<()> {
    write_event(
        state,
        &json!({"turn_id": turn_id, "event": "voice.send", "stage": "gateway"}),
    );
    let messages = local_messages(state, turn_id, &request, turn_record).await?;
    let reasoning_snapshot = update_turn_record(turn_record, |turn| {
        turn.state = VoiceTurnState::ReasoningStarted;
    });
    state.turn_recorder.submit_background(&reasoning_snapshot);
    write_event(
        state,
        &json!({"turn_id": turn_id, "event": "voice.reasoning_started", "stage": "llm"}),
    );

    let (tts_tx, tts_rx) = mpsc::channel::<Option<String>>(8);
    let tts_state = state.clone();
    let tts_turn_id = turn_id.to_owned();
    let tts_out_tx = out_tx.clone();
    let tts_turn_record = turn_record.clone();
    let tts_handle = tokio::spawn(async move {
        tts_worker(
            &tts_state,
            &tts_turn_record,
            &tts_turn_id,
            started,
            tts_rx,
            tts_out_tx,
        )
        .await;
    });

    let mut first_token = false;
    let mut full_text = String::new();
    let mut phrase_buffer = String::new();
    let mut done = false;
    let payload = json!({
        "model": state.config.llm_model,
        "messages": messages,
        "stream": true,
        "temperature": 0.6,
        "max_tokens": 220,
    });
    let response = state
        .http
        .post(format!("{}/chat/completions", state.config.llm_upstream))
        .json(&payload)
        .timeout(state.config.total_turn_timeout)
        .send()
        .await
        .context("llm chat/completions request")?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("llm_http_{}", status.as_u16()));
    }

    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    while !done {
        let next = tokio::time::timeout(state.config.idle_timeout, stream.next())
            .await
            .context("llm idle timeout")?;
        let Some(chunk) = next else {
            break;
        };
        let bytes = chunk.context("llm stream chunk")?;
        pending.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(index) = pending.find('\n') {
            let line = pending[..index].trim().to_owned();
            pending = pending[index + 1..].to_owned();
            if handle_llm_line(
                state,
                turn_record,
                turn_id,
                started,
                &line,
                &out_tx,
                &tts_tx,
                &mut first_token,
                &mut full_text,
                &mut phrase_buffer,
            )
            .await?
            {
                done = true;
                break;
            }
        }
    }
    if !pending.trim().is_empty() {
        let _ = handle_llm_line(
            state,
            turn_record,
            turn_id,
            started,
            pending.trim(),
            &out_tx,
            &tts_tx,
            &mut first_token,
            &mut full_text,
            &mut phrase_buffer,
        )
        .await?;
    }

    let tail = phrase_buffer.trim();
    if !tail.is_empty() {
        tts_tx
            .send(Some(tail.to_owned()))
            .await
            .context("queue tts tail")?;
    }
    let _ = tts_tx.send(None).await;
    let _ = tts_handle.await;

    let text = full_text.trim().to_owned();
    out_tx
        .send(json!({"type": "done", "turn_id": turn_id, "text": text}))
        .await
        .context("send done frame")?;
    write_event(
        state,
        &json!({
            "turn_id": turn_id,
            "event": "voice.close",
            "stage": "gateway",
            "latency_ms": elapsed_ms(started),
            "text": text,
        }),
    );
    let completed_snapshot = update_turn_record(turn_record, |turn| {
        turn.reply = text.clone();
        turn.state = VoiceTurnState::Completed;
        turn.completed_at = Some(Utc::now());
    });
    state.turn_recorder.submit_final(&completed_snapshot).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_llm_line(
    state: &AppState,
    turn_record: &SharedVoiceTurnRecord,
    turn_id: &str,
    started: Instant,
    line: &str,
    out_tx: &mpsc::Sender<Value>,
    tts_tx: &mpsc::Sender<Option<String>>,
    first_token: &mut bool,
    full_text: &mut String,
    phrase_buffer: &mut String,
) -> Result<bool> {
    match parse_llm_sse_line(line) {
        LlmLine::Done => Ok(true),
        LlmLine::Ignore => Ok(false),
        LlmLine::Delta(delta) => {
            if delta.is_empty() {
                return Ok(false);
            }
            full_text.push_str(&delta);
            phrase_buffer.push_str(&delta);
            if !*first_token {
                *first_token = true;
                let latency = elapsed_ms(started);
                push_first_token_latency(state, latency);
                let token_snapshot = update_turn_record(turn_record, |turn| {
                    turn.metrics.first_token_latency_ms = Some(latency);
                });
                state.turn_recorder.submit_background(&token_snapshot);
                write_event(
                    state,
                    &json!({
                        "turn_id": turn_id,
                        "event": "voice.first_token",
                        "stage": "llm",
                        "latency_ms": latency,
                        "text": delta,
                    }),
                );
            }
            out_tx
                .send(json!({"type": "chunk", "turn_id": turn_id, "text_delta": delta}))
                .await
                .context("send chunk frame")?;
            let (segments, rest) =
                split_tts_segments(phrase_buffer, state.config.min_tts_phrase_chars);
            *phrase_buffer = rest;
            for segment in segments {
                tts_tx
                    .send(Some(segment))
                    .await
                    .context("queue tts segment")?;
            }
            Ok(false)
        }
    }
}

async fn tts_worker(
    state: &AppState,
    turn_record: &SharedVoiceTurnRecord,
    turn_id: &str,
    started: Instant,
    mut rx: mpsc::Receiver<Option<String>>,
    out_tx: mpsc::Sender<Value>,
) {
    let mut sequence = 0_u64;
    let mut first_audio_seen = false;
    while let Some(item) = rx.recv().await {
        let Some(phrase) = item else {
            return;
        };
        let queue_depth_ms = rx.len() as f64 * 160.0;
        if let Err(err) = stream_tts_phrase(
            state,
            turn_record,
            turn_id,
            started,
            &phrase,
            queue_depth_ms,
            &mut sequence,
            &mut first_audio_seen,
            &out_tx,
        )
        .await
        {
            metric_fail(state, "tts");
            write_event(
                state,
                &json!({
                    "turn_id": turn_id,
                    "event": "voice.error",
                    "stage": "tts",
                    "status": "error",
                    "error_class": err.to_string(),
                }),
            );
            let _ = out_tx
                .send(json!({"type": "error", "turn_id": turn_id, "error": err.to_string()}))
                .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_tts_phrase(
    state: &AppState,
    turn_record: &SharedVoiceTurnRecord,
    turn_id: &str,
    started: Instant,
    phrase: &str,
    queue_depth_ms: f64,
    sequence: &mut u64,
    first_audio_seen: &mut bool,
    out_tx: &mpsc::Sender<Value>,
) -> Result<()> {
    let payload = json!({
        "text": shape_spoken_text(phrase),
        "voice": state.config.voice_profile,
        "sequence_start": *sequence,
        "queue_depth_ms": queue_depth_ms,
    });
    let response = state
        .http
        .post(format!("{}/stream", state.config.tts_upstream))
        .json(&payload)
        .timeout(state.config.total_turn_timeout)
        .send()
        .await
        .context("tts stream request")?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("tts_http_{}", status.as_u16()));
    }
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    while let Some(chunk) = tokio::time::timeout(state.config.idle_timeout, stream.next())
        .await
        .context("tts idle timeout")?
    {
        let bytes = chunk.context("tts stream chunk")?;
        pending.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(index) = pending.find('\n') {
            let line = pending[..index].trim().to_owned();
            pending = pending[index + 1..].to_owned();
            handle_tts_line(
                state,
                turn_record,
                turn_id,
                started,
                &line,
                sequence,
                first_audio_seen,
                out_tx,
            )
            .await?;
        }
    }
    if !pending.trim().is_empty() {
        handle_tts_line(
            state,
            turn_record,
            turn_id,
            started,
            pending.trim(),
            sequence,
            first_audio_seen,
            out_tx,
        )
        .await?;
    }
    Ok(())
}

async fn handle_tts_line(
    state: &AppState,
    turn_record: &SharedVoiceTurnRecord,
    turn_id: &str,
    started: Instant,
    line: &str,
    sequence: &mut u64,
    first_audio_seen: &mut bool,
    out_tx: &mpsc::Sender<Value>,
) -> Result<()> {
    if line.is_empty() {
        return Ok(());
    }
    let parsed: Value = serde_json::from_str(line).context("parse tts ndjson frame")?;
    let frame_type = option_value(string_field(&parsed, "type"));
    if frame_type == "done" {
        if let Some(rtf) = number_field(&parsed, "tts_rtf") {
            push_tts_rtf(state, rtf);
            let rtf_snapshot = update_turn_record(turn_record, |turn| {
                turn.metrics.tts_rtf = Some(rtf);
            });
            state.turn_recorder.submit_background(&rtf_snapshot);
        }
        if bool_field(&parsed, "degraded_active").unwrap_or(false) {
            let degraded_snapshot = update_turn_record(turn_record, |turn| {
                turn.audio.fallback_voice = true;
                if let Some(profile_hash) = string_field(&parsed, "voice_profile_hash") {
                    turn.audio.voice_profile_hash = Some(profile_hash);
                }
            });
            state.turn_recorder.submit_background(&degraded_snapshot);
            write_event(
                state,
                &json!({
                    "turn_id": turn_id,
                    "event": "voice.degraded_tts",
                    "stage": "tts",
                    "degraded_active": true,
                    "voice_profile_hash": option_value(string_field(&parsed, "voice_profile_hash")),
                }),
            );
        }
        return Ok(());
    }
    if frame_type == "error" {
        return Err(anyhow!(
            "{}",
            option_or_lazy(string_field(&parsed, "error"), || "tts_error".to_owned())
        ));
    }
    if !parsed.is_object() {
        return Ok(());
    }

    let mut frame = option_value(parsed.as_object().cloned());
    let frame_sequence = number_field(&Value::Object(frame.clone()), "sequence")
        .map(|value| value as u64)
        .unwrap_or(*sequence);
    frame.insert("type".to_owned(), json!("audio"));
    frame.insert("turn_id".to_owned(), json!(turn_id));
    frame.insert("sequence".to_owned(), json!(frame_sequence));
    let frame_value = Value::Object(frame);

    if !*first_audio_seen {
        *first_audio_seen = true;
        let latency = elapsed_ms(started);
        push_first_audio_latency(state, latency);
        let audio_dir = turn_audio_dir(&state.config.audio_dir, turn_id)
            .display()
            .to_string();
        let audio_snapshot = update_turn_record(turn_record, |turn| {
            turn.state = VoiceTurnState::AudioStarted;
            turn.metrics.first_audio_latency_ms = Some(latency);
            turn.audio.output_ref = Some(audio_dir);
        });
        state.turn_recorder.submit_background(&audio_snapshot);
        write_event(
            state,
            &json!({
                "turn_id": turn_id,
                "event": "voice.first_audio",
                "stage": "tts",
                "latency_ms": latency,
                "bytes": option_value(string_field(&frame_value, "audio_data")).len(),
                "voice_profile_hash": option_value(string_field(&frame_value, "voice_profile_hash")),
            }),
        );
    }
    if let Some((audio_hash, clipping_count)) = capture_tts_frame(state, turn_id, &frame_value) {
        let output_snapshot = update_turn_record(turn_record, |turn| {
            turn.audio.output_hash = Some(audio_hash);
            turn.audio.output_sample_rate =
                number_field(&frame_value, "sample_rate").map(|value| value as u32);
            turn.metrics.clipping_count =
                turn.metrics.clipping_count.saturating_add(clipping_count);
        });
        state.turn_recorder.submit_background(&output_snapshot);
    }
    out_tx.send(frame_value).await.context("send audio frame")?;
    *sequence = frame_sequence + 1;
    Ok(())
}

async fn local_messages(
    state: &AppState,
    turn_id: &str,
    request: &Value,
    turn_record: &SharedVoiceTurnRecord,
) -> Result<Vec<Value>> {
    let mut messages = Vec::<Value>::new();
    if let Some(raw_messages) = request.get("messages").and_then(Value::as_array) {
        for raw in raw_messages {
            let role = option_or_lazy(string_field(raw, "role"), || "user".to_owned());
            let role = if matches!(role.as_str(), "system" | "user" | "assistant") {
                role
            } else {
                "user".to_owned()
            };
            let text =
                content_to_text(state, turn_id, &role, raw.get("content"), turn_record).await?;
            if !text.trim().is_empty() {
                messages.push(json!({"role": role, "content": text.trim()}));
            }
        }
    } else {
        let prompt = paired_string_field(request, "prompt", "text");
        let prompt = option_value(prompt);
        if !prompt.trim().is_empty() {
            messages.push(json!({"role": "user", "content": prompt.trim()}));
        }
    }

    if messages.is_empty()
        || messages
            .last()
            .and_then(|message| string_field(message, "role"))
            .as_deref()
            != Some("user")
    {
        messages.push(json!({
            "role": "user",
            "content": "Give a concise spoken JMCP status update."
        }));
    }
    Ok(messages)
}

async fn content_to_text(
    state: &AppState,
    turn_id: &str,
    role: &str,
    content: Option<&Value>,
    turn_record: &SharedVoiceTurnRecord,
) -> Result<String> {
    let Some(content) = content else {
        return Ok(String::new());
    };
    if let Some(text) = content.as_str() {
        return Ok(text.to_owned());
    }
    let Some(parts) = content.as_array() else {
        return Ok(String::new());
    };
    let mut output = Vec::<String>::new();
    for item in parts {
        let item_type = option_value(string_field(item, "type"));
        if item_type == "text" {
            if let Some(text) = string_field(item, "text").filter(|text| !text.trim().is_empty()) {
                output.push(text);
            }
        } else if item_type == "audio" && role == "user" {
            let encoded = paired_string_field(item, "data", "audio_data");
            let encoded = option_value(encoded);
            if !encoded.is_empty() {
                let transcript = transcribe_audio(state, turn_id, &encoded, turn_record).await?;
                if !transcript.trim().is_empty() {
                    output.push(transcript);
                }
            }
        }
    }
    Ok(output.join("\n"))
}

async fn transcribe_audio(
    state: &AppState,
    turn_id: &str,
    audio_data: &str,
    turn_record: &SharedVoiceTurnRecord,
) -> Result<String> {
    let samples = float32_base64_to_samples(audio_data).context("decode input f32le")?;
    let wav = wav_bytes_from_samples(&samples, INPUT_SAMPLE_RATE);
    let input_hash = sha256_hex(&wav);
    let input_ref = capture_wav_bytes(
        state,
        turn_id,
        "user_input",
        "input_000_16k.wav",
        &wav,
        INPUT_SAMPLE_RATE,
    );
    let started = Instant::now();
    let response = state
        .http
        .post(format!(
            "{}/transcribe?language=en&beam_size=1",
            state.config.asr_upstream
        ))
        .header("content-type", "audio/wav")
        .body(wav)
        .timeout(state.config.total_turn_timeout)
        .send()
        .await
        .context("asr transcribe request")?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("asr_http_{}", status.as_u16()));
    }
    let data: Value = response.json().await.context("parse asr json")?;
    let transcript = option_value(string_field(&data, "text"));
    let confidence = number_field(&data, "confidence").map(|value| value as f32);
    let asr_rtf = number_field(&data, "rtf");
    let transcript_hash = sha256_hex(transcript.as_bytes());
    let transcribed_snapshot = update_turn_record(turn_record, |turn| {
        turn.state = VoiceTurnState::Transcribed;
        turn.transcript = transcript.clone();
        if let Some(confidence) = confidence {
            turn.confidence = confidence;
        }
        turn.audio.input_hash = Some(input_hash.clone());
        turn.audio.input_ref = input_ref.clone();
        turn.audio.input_sample_rate = Some(INPUT_SAMPLE_RATE);
        turn.metrics.asr_rtf = asr_rtf;
        turn.confirmation_evidence = vec![core_turns::VoiceTurnEvidence {
            kind: "voice.transcript".to_owned(),
            uri: format!("sha256:{transcript_hash}"),
            captured_at: Utc::now(),
        }];
    });
    state.turn_recorder.submit_background(&transcribed_snapshot);
    write_event(
        state,
        &json!({
            "turn_id": turn_id,
            "event": "voice.transcript",
            "stage": "asr",
            "latency_ms": elapsed_ms(started),
            "transcript": transcript,
            "asr_confidence": confidence,
            "asr_rtf": asr_rtf,
        }),
    );
    Ok(transcript)
}

async fn get_json(state: &AppState, url: &str) -> Value {
    match state
        .http
        .get(url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => match response.text().await {
            Ok(body) if body.trim().is_empty() => json!({"ok": true}),
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(Value::Object(map)) => Value::Object(map),
                Ok(_) => json!({"ok": true}),
                Err(_) => json!({"ok": true, "status": body}),
            },
            Err(err) => json!({"error": err.to_string()}),
        },
        Ok(response) => json!({"error": format!("http_{}", response.status().as_u16())}),
        Err(err) => json!({"error": err.to_string()}),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LlmLine {
    Delta(String),
    Done,
    Ignore,
}

fn parse_llm_sse_line(line: &str) -> LlmLine {
    let line = line.trim();
    if !line.starts_with("data:") {
        return LlmLine::Ignore;
    }
    let data = line[5..].trim();
    if data == "[DONE]" {
        return LlmLine::Done;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return LlmLine::Ignore;
    };
    let Some(choices) = parsed.get("choices").and_then(Value::as_array) else {
        return LlmLine::Ignore;
    };
    let Some(choice) = choices.first() else {
        return LlmLine::Ignore;
    };
    let delta = match choice
        .get("delta")
        .and_then(|delta| string_field(delta, "content"))
    {
        Some(delta) => Some(delta),
        None => string_field(choice, "text"),
    };
    let delta = option_value(delta);
    if delta.is_empty() {
        LlmLine::Ignore
    } else {
        LlmLine::Delta(delta)
    }
}

fn split_tts_segments(buffer: &str, min_chars: usize) -> (Vec<String>, String) {
    let mut segments = Vec::new();
    let mut cursor = 0_usize;
    let mut chars = buffer.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        if !matches!(ch, '.' | '!' | '?' | ';' | ':') {
            continue;
        }
        let after_punctuation = idx + ch.len_utf8();
        let mut end = after_punctuation;
        let mut saw_space = false;
        while let Some((next_idx, next_ch)) = chars.peek().copied() {
            if !next_ch.is_whitespace() {
                break;
            }
            saw_space = true;
            end = next_idx + next_ch.len_utf8();
            chars.next();
        }
        if !saw_space {
            continue;
        }
        let candidate = buffer[cursor..end].trim();
        if candidate.len() >= min_chars {
            segments.push(candidate.to_owned());
            cursor = end;
        }
    }
    if cursor > 0 {
        return (segments, buffer[cursor..].to_owned());
    }
    let hard_limit = 140_usize.max(min_chars * 3);
    if buffer.len() >= hard_limit {
        let limit = buffer.len().min(120);
        if let Some(split_at) = buffer[..limit].rfind(' ') {
            if split_at >= min_chars {
                return (
                    vec![buffer[..split_at].trim().to_owned()],
                    buffer[split_at..].trim_start().to_owned(),
                );
            }
        }
    }
    (segments, buffer.to_owned())
}

fn shape_spoken_text(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = clean.trim();
    if trimmed.len() > 80 && (trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return "I have structured data ready, but I will not read the raw JSON unless you ask."
            .to_owned();
    }
    clean
        .split_whitespace()
        .map(shape_spoken_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shape_spoken_token(token: &str) -> String {
    let bare = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
    if looks_like_uuid_or_hash(bare) {
        let suffix = bare
            .chars()
            .rev()
            .filter(|ch| ch.is_ascii_hexdigit())
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        return format!("identifier ending {suffix}");
    }
    match token {
        "OK" | "ok" => "okay".to_owned(),
        "JSON" => "J S O N".to_owned(),
        "UUID" => "U U I D".to_owned(),
        "GPU" => "G P U".to_owned(),
        "CPU" => "C P U".to_owned(),
        _ => token.replace('_', " "),
    }
}

fn looks_like_uuid_or_hash(value: &str) -> bool {
    let hex_count = value.chars().filter(|ch| ch.is_ascii_hexdigit()).count();
    let dash_count = value.chars().filter(|ch| *ch == '-').count();
    value.len() >= 24 && hex_count * 100 / value.len().max(1) >= 80 && dash_count <= 5
}

fn build_realtime_request(turn_id: &str, audio_chunks: &[Vec<u8>], text_parts: &[String]) -> Value {
    let mut content = Vec::<Value>::new();
    let mut combined_audio = Vec::<u8>::new();
    for chunk in audio_chunks {
        combined_audio.extend_from_slice(chunk);
    }
    if !combined_audio.is_empty() {
        content.push(json!({"type": "audio", "data": BASE64.encode(combined_audio)}));
    }
    let text = text_parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        content.push(json!({"type": "text", "text": text}));
    }
    if content.is_empty() {
        json!({"turn_id": turn_id, "prompt": "Give a concise spoken JMCP status update."})
    } else {
        json!({"turn_id": turn_id, "messages": [{"role": "user", "content": content}]})
    }
}

fn parse_ws_message(message: Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str(&text).ok(),
        Message::Binary(bytes) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    }
}

async fn send_socket_json(socket: &mut WebSocket, payload: Value) -> Result<()> {
    socket
        .send(Message::Text(payload.to_string()))
        .await
        .map_err(|err| anyhow!(err.to_string()))
}

fn write_event(state: &AppState, event: &Value) {
    let sanitized = sanitize_event(event);
    if let Some(parent) = state.config.event_log.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.config.event_log)
    {
        let _ = writeln!(file, "{}", result_value(serde_json::to_string(&sanitized)));
    }
    if state.config.capture_raw_audio {
        let turn_id = option_or_lazy(string_field(&sanitized, "turn_id"), || {
            "turn_unknown".to_owned()
        });
        let turn_dir = turn_audio_dir(&state.config.audio_dir, &turn_id);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(turn_dir.join("events.jsonl"))
        {
            let _ = writeln!(file, "{}", result_value(serde_json::to_string(&sanitized)));
        }
    }
}

fn sanitize_event(event: &Value) -> Value {
    let mut out = Map::<String, Value>::new();
    out.insert(
        "ts".to_owned(),
        json!(option_or_lazy(string_field(event, "ts"), now_iso)),
    );
    out.insert(
        "turn_id".to_owned(),
        json!(option_or_lazy(string_field(event, "turn_id"), new_turn_id)),
    );
    out.insert(
        "event".to_owned(),
        json!(option_or_lazy(string_field(event, "event"), || {
            "voice.event".to_owned()
        })),
    );
    out.insert(
        "status".to_owned(),
        json!(option_or_lazy(string_field(event, "status"), || "ok".to_owned())),
    );
    copy_string(event, &mut out, "stage");
    copy_string(event, &mut out, "error_class");
    copy_string(event, &mut out, "audio_format");
    copy_string(event, &mut out, "audio_hash");
    copy_string(event, &mut out, "voice_profile_hash");
    copy_string(event, &mut out, "tool_policy_decision");
    copy_string(event, &mut out, "tool_name");
    for key in [
        "latency_ms",
        "bytes",
        "sample_rate",
        "sequence",
        "duration_ms",
        "tts_elapsed_ms",
        "queue_depth_ms",
        "asr_confidence",
        "asr_rtf",
        "tts_rtf",
        "clipping_count",
        "rms",
        "loudness_range_db",
        "leading_silence_ms",
        "trailing_silence_ms",
        "chunk_boundary_gap_ms",
        "crossfade_ms",
        "output_sample_rate",
    ] {
        copy_number(event, &mut out, key);
    }
    for key in [
        "crossfade_applied",
        "degraded_active",
        "echo_rejected",
        "barge_in",
        "underrun",
    ] {
        copy_bool(event, &mut out, key);
    }
    if let Some(transcript) = string_field(event, "transcript").filter(|value| !value.is_empty()) {
        out.insert(
            "transcript_preview".to_owned(),
            json!(preview(&transcript, 80)),
        );
        out.insert(
            "transcript_hash".to_owned(),
            json!(sha256_hex(transcript.as_bytes())),
        );
    }
    if let Some(text) = string_field(event, "text").filter(|value| !value.is_empty()) {
        out.insert("text_preview".to_owned(), json!(preview(&text, 80)));
        out.insert("text_hash".to_owned(), json!(sha256_hex(text.as_bytes())));
    }
    Value::Object(out)
}

fn capture_wav_bytes(
    state: &AppState,
    turn_id: &str,
    kind: &str,
    filename: &str,
    wav: &[u8],
    sample_rate: u32,
) -> Option<String> {
    if !state.config.capture_raw_audio {
        return None;
    }
    let path = turn_audio_dir(&state.config.audio_dir, turn_id).join(filename);
    if fs::write(&path, wav).is_ok() {
        write_event(
            state,
            &json!({
            "turn_id": turn_id,
            "event": "voice.audio_snippet",
            "stage": kind,
            "bytes": wav.len(),
            "sample_rate": sample_rate,
            "duration_ms": wav_duration_ms(wav, sample_rate),
            "audio_hash": sha256_hex(wav),
            }),
        );
        return Some(path.display().to_string());
    }
    None
}

fn capture_tts_frame(state: &AppState, turn_id: &str, frame: &Value) -> Option<(String, u64)> {
    let mut event = Map::<String, Value>::new();
    event.insert("turn_id".to_owned(), json!(turn_id));
    event.insert("event".to_owned(), json!("voice.tts_chunk"));
    event.insert("stage".to_owned(), json!("tts"));
    copy_string(frame, &mut event, "audio_format");
    copy_string(frame, &mut event, "voice_profile_hash");
    for key in [
        "sample_rate",
        "sequence",
        "duration_ms",
        "tts_elapsed_ms",
        "queue_depth_ms",
    ] {
        copy_number(frame, &mut event, key);
    }
    let audio_data = option_value(string_field(frame, "audio_data"));
    let sample_rate =
        number_field(frame, "sample_rate").unwrap_or(DEFAULT_OUTPUT_SAMPLE_RATE as f64) as u32;
    if !audio_data.is_empty() {
        if let Ok(raw) = BASE64.decode(audio_data.as_bytes()) {
            let audio_hash = sha256_hex(&raw);
            event.insert("bytes".to_owned(), json!(raw.len()));
            event.insert("audio_hash".to_owned(), json!(audio_hash.clone()));
            let samples = samples_from_f32le_bytes(&raw);
            let metrics = audio_quality_metrics(&samples, sample_rate);
            let clipping_count = metrics
                .get("clipping_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            for (key, value) in metrics {
                event.insert(key, value);
            }
            if state.config.capture_raw_audio {
                let sequence = number_field(frame, "sequence").unwrap_or(0.0) as u64;
                let wav = wav_bytes_from_samples(&samples, sample_rate);
                let path = turn_audio_dir(&state.config.audio_dir, turn_id).join(format!(
                    "output_chunk_{sequence:04}_{}k.wav",
                    sample_rate / 1000
                ));
                let _ = fs::write(path, wav);
            }
            write_event(state, &Value::Object(event));
            return Some((audio_hash, clipping_count));
        }
    }
    write_event(state, &Value::Object(event));
    None
}

fn audio_quality_metrics(samples: &[f32], sample_rate: u32) -> Map<String, Value> {
    let mut out = Map::<String, Value>::new();
    if samples.is_empty() || sample_rate == 0 {
        return out;
    }
    let clipping_count = samples
        .iter()
        .filter(|sample| sample.abs() >= 0.999)
        .count();
    let mean_square = samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64;
    let rms = mean_square.sqrt();
    let silence_threshold = 0.001_f32;
    let leading = samples
        .iter()
        .take_while(|sample| sample.abs() < silence_threshold)
        .count();
    let trailing = samples
        .iter()
        .rev()
        .take_while(|sample| sample.abs() < silence_threshold)
        .count();
    out.insert("clipping_count".to_owned(), json!(clipping_count));
    out.insert("rms".to_owned(), json!(round4(rms)));
    out.insert(
        "leading_silence_ms".to_owned(),
        json!(round1(leading as f64 / sample_rate as f64 * 1000.0)),
    );
    out.insert(
        "trailing_silence_ms".to_owned(),
        json!(round1(trailing as f64 / sample_rate as f64 * 1000.0)),
    );
    if let Some(range) = loudness_range_db(samples, sample_rate) {
        out.insert("loudness_range_db".to_owned(), json!(round1(range)));
    }
    out
}

fn loudness_range_db(samples: &[f32], sample_rate: u32) -> Option<f64> {
    let window = (sample_rate as usize / 10).max(1);
    let mut levels = Vec::<f64>::new();
    for chunk in samples.chunks(window) {
        if chunk.is_empty() {
            continue;
        }
        let rms = (chunk
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
            / chunk.len() as f64)
            .sqrt();
        if rms > 0.000_001 {
            levels.push(20.0 * rms.log10());
        }
    }
    if levels.len() < 2 {
        return Some(0.0);
    }
    let min = levels.iter().copied().fold(f64::INFINITY, f64::min);
    let max = levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some(max - min)
}

fn float32_base64_to_samples(data: &str) -> Result<Vec<f32>> {
    let raw = BASE64.decode(data.as_bytes())?;
    Ok(samples_from_f32le_bytes(&raw))
}

fn samples_from_f32le_bytes(raw: &[u8]) -> Vec<f32> {
    raw.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn wav_bytes_from_samples(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut frames = Vec::<u8>::with_capacity(samples.len() * 2);
    for sample in samples {
        let clipped = sample.clamp(-1.0, 1.0);
        let value = if clipped >= 0.0 {
            (clipped * 32767.0) as i16
        } else {
            (clipped * 32768.0) as i16
        };
        frames.extend_from_slice(&value.to_le_bytes());
    }
    let data_size = frames.len() as u32;
    let mut wav = Vec::<u8>::with_capacity(44 + frames.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(&frames);
    wav
}

fn wav_duration_ms(wav: &[u8], sample_rate: u32) -> f64 {
    if wav.len() <= 44 || sample_rate == 0 {
        return 0.0;
    }
    round1(((wav.len() - 44) as f64 / 2.0) / sample_rate as f64 * 1000.0)
}

fn turn_audio_dir(base: &Path, turn_id: &str) -> PathBuf {
    let safe = turn_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .take(96)
        .collect::<String>();
    let dir = base.join(if safe.is_empty() {
        "turn_unknown".to_owned()
    } else {
        safe
    });
    let _ = fs::create_dir_all(&dir);
    dir
}

fn prune_audio_retention(config: &VoiceConfig) {
    if !config.capture_raw_audio || !config.audio_dir.exists() {
        return;
    }
    let Ok(entries) = fs::read_dir(&config.audio_dir) else {
        return;
    };
    let mut retained = Vec::<AudioEntry>::new();
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Some(days) = config.audio_retention_days {
            let max_age = Duration::from_secs(days.saturating_mul(86_400));
            if result_value(now.duration_since(modified)) > max_age {
                remove_path(&path);
                continue;
            }
        }
        let size = path_size(&path);
        retained.push(AudioEntry {
            path,
            modified,
            size,
        });
    }
    let Some(max_mb) = config.audio_max_mb else {
        return;
    };
    let max_bytes = max_mb.saturating_mul(1024 * 1024);
    let mut total = retained.iter().map(|entry| entry.size).sum::<u64>();
    retained.sort_by_key(|entry| entry.modified);
    for entry in retained {
        if total <= max_bytes {
            break;
        }
        remove_path(&entry.path);
        total = total.saturating_sub(entry.size);
    }
}

struct AudioEntry {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
}

fn path_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| path_size(&entry.path()))
        .sum()
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn increment_turns(state: &AppState) {
    state.metrics.lock().expect("metrics lock").turns_total += 1;
}

fn metric_fail(state: &AppState, stage: &str) {
    let mut metrics = state.metrics.lock().expect("metrics lock");
    if let Some((_, count)) = metrics.failures.iter_mut().find(|(name, _)| name == stage) {
        *count += 1;
    } else {
        metrics.failures.push((stage.to_owned(), 1));
    }
}

fn push_first_token_latency(state: &AppState, latency_ms: f64) {
    state
        .metrics
        .lock()
        .expect("metrics lock")
        .first_token_latency_ms
        .push(latency_ms);
}

fn push_first_audio_latency(state: &AppState, latency_ms: f64) {
    state
        .metrics
        .lock()
        .expect("metrics lock")
        .first_audio_latency_ms
        .push(latency_ms);
}

fn push_tts_rtf(state: &AppState, rtf: f64) {
    state
        .metrics
        .lock()
        .expect("metrics lock")
        .tts_rtf
        .push(rtf);
}

fn endpoint_ok(value: &Value) -> bool {
    if string_field(value, "error").is_some_and(|error| !error.is_empty()) {
        return false;
    }
    if let Some(ok) = bool_field(value, "ok") {
        return ok;
    }
    value.as_object().is_some_and(|object| !object.is_empty())
}

fn value_or_null(value: &Value, key: &str) -> Value {
    value.get(key).cloned().unwrap_or(Value::Null)
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

fn option_or_lazy<T, F>(value: Option<T>, replacement: F) -> T
where
    F: FnOnce() -> T,
{
    match value {
        Some(value) => value,
        None => replacement(),
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn paired_string_field(value: &Value, primary: &str, secondary: &str) -> Option<String> {
    match string_field(value, primary) {
        Some(value) => Some(value),
        None => string_field(value, secondary),
    }
}

fn number_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn copy_string(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = string_field(input, key).filter(|value| !value.is_empty()) {
        output.insert(key.to_owned(), json!(value));
    }
}

fn copy_number(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = number_field(input, key).filter(|value| value.is_finite()) {
        output.insert(key.to_owned(), json!(value));
    }
}

fn copy_bool(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = bool_field(input, key) {
        output.insert(key.to_owned(), json!(value));
    }
}

fn load_profile_hash(path: &Path) -> Result<String> {
    let data = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&data)?;
    Ok(sha256_hex(canonical_json(&value).as_bytes()))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let fields = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        result_value(serde_json::to_string(key)),
                        canonical_json(&map[key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{fields}}}")
        }
        Value::Array(items) => {
            let fields = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{fields}]")
        }
        _ => result_value(serde_json::to_string(value)),
    }
}

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default.to_owned(),
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
        .unwrap_or(default)
}

fn env_u64(key: &str, default: Option<u64>) -> Option<u64> {
    match std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        Some(value) => value.parse::<u64>().ok(),
        None => default,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_duration_ms(key: &str, default_ms: u64) -> Duration {
    Duration::from_millis(
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(default_ms),
    )
}

fn trim_url(value: String) -> String {
    value.trim_end_matches('/').to_owned()
}

fn new_turn_id() -> String {
    format!("turn_{}", Uuid::new_v4().simple())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex::encode(digest.finalize())
}

fn preview(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn elapsed_ms(started: Instant) -> f64 {
    round1(started.elapsed().as_secs_f64() * 1000.0)
}

fn p50(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    ordered[ordered.len() / 2]
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_tts_segments_waits_for_useful_phrase() {
        let (segments, rest) = split_tts_segments("Short. Still building", 12);
        assert!(segments.is_empty());
        assert_eq!(rest, "Short. Still building");

        let (segments, rest) = split_tts_segments("This is a complete spoken phrase. Next bit", 12);
        assert_eq!(segments, vec!["This is a complete spoken phrase."]);
        assert_eq!(rest, "Next bit");
    }

    #[test]
    fn float32_base64_audio_becomes_wav() {
        let raw = [0.0_f32, 0.5, -0.5]
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        let samples = float32_base64_to_samples(&BASE64.encode(raw)).unwrap();
        let wav = wav_bytes_from_samples(&samples, 16_000);

        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(wav.len(), 44 + 3 * 2);
    }

    #[test]
    fn sanitize_event_redacts_text_to_preview_and_hash() {
        let event = json!({
            "turn_id": "turn_test",
            "event": "voice.transcript",
            "stage": "asr",
            "transcript": "approve token alpha for the deployment",
        });
        let sanitized = sanitize_event(&event);

        assert!(sanitized.get("transcript").is_none());
        assert_eq!(
            sanitized["transcript_preview"],
            "approve token alpha for the deployment"
        );
        assert!(sanitized["transcript_hash"].as_str().unwrap().len() >= 32);
    }

    #[test]
    fn quality_metrics_detect_clipping_and_silence() {
        let samples = vec![0.0, 0.0, 1.0, 0.5, 0.0, 0.0];
        let metrics = audio_quality_metrics(&samples, 1000);

        assert_eq!(metrics["clipping_count"], 1);
        assert_eq!(metrics["leading_silence_ms"], 2.0);
        assert_eq!(metrics["trailing_silence_ms"], 2.0);
    }

    #[test]
    fn realtime_request_combines_audio_chunks() {
        let first = 0.25_f32.to_le_bytes().to_vec();
        let second = (-0.25_f32).to_le_bytes().to_vec();
        let request = build_realtime_request("turn_a", &[first, second], &[String::from("status")]);
        let content = request["messages"][0]["content"].as_array().unwrap();

        assert_eq!(request["turn_id"], "turn_a");
        assert_eq!(content[0]["type"], "audio");
        assert_eq!(content[1]["text"], "status");
        let decoded = BASE64.decode(content[0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(decoded.len(), 8);
    }

    #[test]
    fn llm_sse_parser_reads_openai_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(parse_llm_sse_line(line), LlmLine::Delta("hello".to_owned()));
        assert_eq!(parse_llm_sse_line("data: [DONE]"), LlmLine::Done);
    }

    #[test]
    fn spoken_text_shapes_raw_json_and_identifiers() {
        let shaped = shape_spoken_text("{\"id\":\"11111111-1111-4111-8111-111111111111\",\"items\":[1,2,3],\"status\":\"ok\",\"payload\":\"long enough to avoid reading raw structured data aloud\"}");
        assert!(shaped.contains("structured data"));

        let shaped = shape_spoken_text("job 11111111-1111-4111-8111-111111111111 OK");
        assert!(shaped.contains("identifier ending 1111"));
        assert!(shaped.contains("okay"));
    }
}
