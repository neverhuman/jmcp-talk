use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Parser)]
#[command(author, version, about = "JMCP deterministic local speech daemon")]
struct Args {
    #[arg(long, env = "JMCP_SPEECHD_BIND", default_value = "127.0.0.1:18878")]
    bind: SocketAddr,
    #[arg(long, env = "JMCP_SPEECHD_ROLE", default_value = "both")]
    role: String,
    #[arg(long, env = "JMCP_SPEECHD_TRANSCRIPT", default_value = "")]
    transcript: String,
    #[arg(long, env = "JMCP_SPEECHD_FAIL_CLOSED", default_value_t = false)]
    fail_closed: bool,
}

#[derive(Clone)]
struct SpeechState {
    role: String,
    transcript: String,
    fail_closed: bool,
}

#[derive(Deserialize)]
struct TranscribeQuery {
    language: Option<String>,
    beam_size: Option<u32>,
}

#[derive(Deserialize)]
struct SynthesizeQuery {
    format: Option<String>,
}

#[derive(Deserialize)]
struct SynthesizeRequest {
    text: String,
    voice: Option<String>,
    speed: Option<f32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let state = Arc::new(SpeechState {
        role: args.role,
        transcript: args.transcript,
        fail_closed: args.fail_closed,
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/transcribe", post(transcribe))
        .route("/synthesize", post(synthesize))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<Arc<SpeechState>>) -> Json<serde_json::Value> {
    let loaded = !state.fail_closed;
    Json(json!({
        "ok": loaded,
        "provider": "jmcp-speechd-deterministic",
        "role": state.role,
        "model": if state.role == "tts" { "jmcp-speechd-deterministic-tts" } else { "jmcp-speechd-deterministic-asr" },
        "device": "offline",
        "compute_type": "deterministic",
        "beam_size": 1,
        "loaded": loaded,
        "warmed": loaded,
        "voice": "jmcp_neutral",
        "sample_rate": 24000,
        "last_elapsed_ms": 0.0,
        "last_warmup_ms": 0.0,
        "error": if loaded { serde_json::Value::Null } else { json!("fail_closed") },
        "warm_error": serde_json::Value::Null,
    }))
}

async fn transcribe(
    State(state): State<Arc<SpeechState>>,
    Query(query): Query<TranscribeQuery>,
    audio: Bytes,
) -> Response {
    if state.fail_closed || state.role == "tts" {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "speech transcription is disabled",
        );
    }
    let language = match query.language {
        Some(language) => language,
        None => "en".to_owned(),
    };
    let text = state.transcript.clone();
    let confidence = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        json!(1.0)
    };
    Json(json!({
        "text": text,
        "language": language,
        "language_probability": 1.0,
        "confidence": confidence,
        "duration": 0.0,
        "elapsed_ms": 0.0,
        "rtf": 0.0,
        "audio_bytes": audio.len(),
        "beam_size": query.beam_size.unwrap_or(1),
        "segments": [],
    }))
    .into_response()
}

async fn synthesize(
    State(state): State<Arc<SpeechState>>,
    Query(query): Query<SynthesizeQuery>,
    Json(request): Json<SynthesizeRequest>,
) -> Response {
    if state.fail_closed || state.role == "asr" {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "speech synthesis is disabled",
        );
    }
    if request.text.trim().is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "text is required");
    }
    let speed = request.speed.unwrap_or(1.0);
    if !(0.25..=4.0).contains(&speed) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "speed must be between 0.25 and 4.0",
        );
    }
    match query.format.as_deref().unwrap_or("wav") {
        "wav" => audio_response("audio/wav", deterministic_wav()),
        "ogg" => audio_response("audio/ogg", deterministic_ogg(request.voice.as_deref())),
        _ => json_error(StatusCode::BAD_REQUEST, "format must be wav or ogg"),
    }
}

fn audio_response(content_type: &'static str, bytes: Vec<u8>) -> Response {
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn json_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

fn deterministic_ogg(voice: Option<&str>) -> Vec<u8> {
    let mut bytes = b"OggS\0\x02jmcp-speechd-deterministic:".to_vec();
    bytes.extend_from_slice(voice.unwrap_or("jmcp_neutral").as_bytes());
    bytes
}

fn deterministic_wav() -> Vec<u8> {
    let sample_rate = 24_000_u32;
    let samples = 240_u32;
    let channels = 1_u16;
    let bits_per_sample = 16_u16;
    let data_len = samples * channels as u32 * (bits_per_sample as u32 / 8);
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(44 + data_len as usize, 0);
    bytes
}
