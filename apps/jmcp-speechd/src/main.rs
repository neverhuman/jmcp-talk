use anyhow::Result;
use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};
use uuid::Uuid;

const DEFAULT_BIND: &str = "127.0.0.1:18879";
const DEFAULT_TRANSCRIPT: &str = "deterministic speech turn approved";

#[derive(Clone)]
struct AppState {
    config: Arc<SpeechConfig>,
    metrics: Arc<Mutex<Metrics>>,
}

#[derive(Clone)]
struct SpeechConfig {
    adapter: String,
    model: String,
    device: String,
    quantization: String,
    trace_path: PathBuf,
    raw_audio_capture: bool,
    deterministic_transcript: String,
}

#[derive(Default)]
struct Metrics {
    health: u64,
    transcribe: u64,
    synthesize: u64,
}

#[derive(Debug, Deserialize)]
struct SynthesizeRequest {
    text: String,
    voice: Option<String>,
    speed: Option<f32>,
}

#[derive(Debug, Serialize)]
struct TraceEvent {
    turn_id: String,
    session_id: String,
    event: String,
    status: String,
    adapter: String,
    model: String,
    device: String,
    quantization: String,
    latency_ms: u128,
    error_class: Option<String>,
    redacted_transcript: Option<String>,
    audio_hash: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let bind = std::env::var("JMCP_TALK_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let addr: SocketAddr = bind.parse()?;
    let state = AppState {
        config: Arc::new(SpeechConfig::from_env()),
        metrics: Arc::new(Mutex::new(Metrics::default())),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/transcribe", post(transcribe))
        .route("/synthesize", post(synthesize))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

impl SpeechConfig {
    fn from_env() -> Self {
        Self {
            adapter: env_or("JMCP_TALK_ADAPTER", "deterministic"),
            model: env_or("JMCP_TALK_MODEL", "deterministic-speech-fixture"),
            device: env_or("JMCP_TALK_DEVICE", "cpu"),
            quantization: env_or("JMCP_TALK_QUANTIZATION", "none"),
            trace_path: PathBuf::from(env_or(
                "JMCP_TALK_TRACE_PATH",
                "target/jankurai/talk/speech-trace.jsonl",
            )),
            raw_audio_capture: env_or("JMCP_TALK_RAW_AUDIO_CAPTURE", "false") == "true",
            deterministic_transcript: env_or(
                "JMCP_TALK_DETERMINISTIC_TRANSCRIPT",
                DEFAULT_TRANSCRIPT,
            ),
        }
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    state.metrics.lock().expect("metrics lock").health += 1;
    Json(json!({
        "ok": true,
        "adapter": state.config.adapter,
        "model": state.config.model,
        "device": state.config.device,
        "quantization": state.config.quantization,
        "loaded": true,
        "raw_audio_capture": state.config.raw_audio_capture
    }))
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let metrics = state.metrics.lock().expect("metrics lock");
    format!(
        concat!(
            "jmcp_talk_requests_total{{route=\"/health\"}} {}\n",
            "jmcp_talk_requests_total{{route=\"/transcribe\"}} {}\n",
            "jmcp_talk_requests_total{{route=\"/synthesize\"}} {}\n"
        ),
        metrics.health, metrics.transcribe, metrics.synthesize
    )
}

async fn transcribe(State(state): State<AppState>, audio: Bytes) -> Json<serde_json::Value> {
    let started = Instant::now();
    state.metrics.lock().expect("metrics lock").transcribe += 1;
    let transcript = state.config.deterministic_transcript.clone();
    let audio_hash = Some(sha256_hex(&audio));
    let elapsed = started.elapsed().as_millis();
    let event = TraceEvent {
        turn_id: Uuid::new_v4().to_string(),
        session_id: Uuid::new_v4().to_string(),
        event: "speech.transcribed".to_owned(),
        status: "ok".to_owned(),
        adapter: state.config.adapter.clone(),
        model: state.config.model.clone(),
        device: state.config.device.clone(),
        quantization: state.config.quantization.clone(),
        latency_ms: elapsed,
        error_class: None,
        redacted_transcript: Some(redact(&transcript)),
        audio_hash,
    };
    write_trace(&state.config.trace_path, &event);
    Json(json!({
        "text": transcript,
        "language": "en",
        "language_probability": 1.0,
        "confidence": 1.0,
        "duration": 0.0,
        "elapsed_ms": elapsed as f64,
        "rtf": 0.0,
        "segments": [{
            "start": 0.0,
            "end": 0.0,
            "text": transcript
        }]
    }))
}

async fn synthesize(
    State(state): State<AppState>,
    Json(request): Json<SynthesizeRequest>,
) -> impl IntoResponse {
    let started = Instant::now();
    state.metrics.lock().expect("metrics lock").synthesize += 1;
    let bytes = deterministic_wav(&request.text, request.voice.as_deref(), request.speed);
    let elapsed = started.elapsed().as_millis();
    let event = TraceEvent {
        turn_id: Uuid::new_v4().to_string(),
        session_id: Uuid::new_v4().to_string(),
        event: "speech.synthesized".to_owned(),
        status: "ok".to_owned(),
        adapter: state.config.adapter.clone(),
        model: state.config.model.clone(),
        device: state.config.device.clone(),
        quantization: state.config.quantization.clone(),
        latency_ms: elapsed,
        error_class: None,
        redacted_transcript: Some(redact(&request.text)),
        audio_hash: Some(sha256_hex(&bytes)),
    };
    write_trace(&state.config.trace_path, &event);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    (headers, bytes)
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_trace(path: &PathBuf, event: &TraceEvent) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn redact(text: &str) -> String {
    text.split_whitespace()
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            if lower.contains("token") || lower.contains("secret") || lower.contains("password") {
                "[redacted]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn deterministic_wav(text: &str, voice: Option<&str>, speed: Option<f32>) -> Vec<u8> {
    let mut body = format!(
        "JMCP_TALK:{}:{}:{}",
        text,
        voice.unwrap_or("default"),
        speed.unwrap_or(1.0)
    )
    .into_bytes();
    let mut wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    wav.append(&mut body);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_like_words() {
        assert_eq!(
            redact("approve token=abc password hunter2"),
            "approve [redacted] [redacted] hunter2"
        );
    }

    #[test]
    fn trace_event_contains_required_schema_fields() {
        let event = TraceEvent {
            turn_id: "turn-1".to_owned(),
            session_id: "session-1".to_owned(),
            event: "speech.transcribed".to_owned(),
            status: "ok".to_owned(),
            adapter: "deterministic".to_owned(),
            model: "fixture".to_owned(),
            device: "cpu".to_owned(),
            quantization: "none".to_owned(),
            latency_ms: 1,
            error_class: None,
            redacted_transcript: Some("hello".to_owned()),
            audio_hash: Some("abc".to_owned()),
        };
        let json = serde_json::to_value(event).unwrap();
        for key in [
            "turn_id",
            "session_id",
            "event",
            "status",
            "adapter",
            "model",
            "device",
            "quantization",
            "latency_ms",
            "error_class",
            "redacted_transcript",
            "audio_hash",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
    }
}
