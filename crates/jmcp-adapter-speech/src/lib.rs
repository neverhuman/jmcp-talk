//! Thin HTTP clients for the JMCP speech sidecars.
//!
//! The heavy CUDA/ML speech stack runs out-of-process in two Python sidecars
//! (`services/speech/`): faster-whisper large-v3 for ASR and Kokoro-82M for TTS.
//! This crate is the Rust side — exactly like [`jmcp_adapter_jekko`] shells out
//! to a separate engine, these clients call the sidecars over localhost HTTP and
//! never embed PyTorch/CUDA in the runtime.
//!
//! - [`AsrClient`] — `POST /transcribe` (raw audio bytes) → [`Transcription`].
//! - [`TtsClient`] — `POST /synthesize` (text) → WAV bytes (24 kHz, PCM_16).
//!
//! Both default to JMCP-safe localhost ports and are overridable via
//! `JMCP_ASR_URL` / `JMCP_TTS_URL`.

use anyhow::{Context, Result};
use serde::Deserialize;

const DEFAULT_ASR_URL: &str = "http://127.0.0.1:18878";
const DEFAULT_TTS_URL: &str = "http://127.0.0.1:18901";

fn env_url(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) => default.to_owned(),
        Err(std::env::VarError::NotPresent) => default.to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => default.to_owned(),
    }
}

/// Health snapshot of the ASR sidecar (`GET /health`).
#[derive(Clone, Debug, Deserialize)]
pub struct AsrHealth {
    pub ok: bool,
    pub model: String,
    pub device: String,
    pub loaded: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// One transcribed segment with timestamps.
#[derive(Clone, Debug, Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Result of `POST /transcribe`.
#[derive(Clone, Debug, Deserialize)]
pub struct Transcription {
    pub text: String,
    pub language: String,
    #[serde(default)]
    pub language_probability: f64,
    /// Overall recognizer confidence in `0.0..=1.0` (mean per-segment), or
    /// `None` when no speech segments were produced. Drives voice-approval gating.
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub rtf: Option<f64>,
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,
}

/// Client for the faster-whisper ASR sidecar.
pub struct AsrClient {
    http: reqwest::Client,
    base_url: String,
}

impl AsrClient {
    /// Build from `JMCP_ASR_URL` (default `http://127.0.0.1:18878`).
    pub fn from_env() -> Self {
        Self::new(env_url("JMCP_ASR_URL", DEFAULT_ASR_URL))
    }

    /// Build against an explicit base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Read the sidecar health (model, device, loaded).
    pub async fn health(&self) -> Result<AsrHealth> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        response
            .error_for_status()?
            .json::<AsrHealth>()
            .await
            .context("parse ASR /health")
    }

    /// Transcribe raw audio bytes (wav/mp3/flac/ogg). `language` pins the
    /// language (e.g. `"en"`); `None` auto-detects.
    pub async fn transcribe(
        &self,
        audio: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Transcription> {
        let mut url = format!("{}/transcribe", self.base_url);
        if let Some(language) = language {
            url.push_str(&format!("?language={language}"));
        }
        let response = self
            .http
            .post(&url)
            .header("content-type", "audio/wav")
            .body(audio)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        response
            .error_for_status()?
            .json::<Transcription>()
            .await
            .context("parse ASR /transcribe")
    }
}

/// Health snapshot of the TTS sidecar (`GET /health`).
#[derive(Clone, Debug, Deserialize)]
pub struct TtsHealth {
    pub ok: bool,
    pub model: String,
    #[serde(default)]
    pub device: Option<String>,
    pub loaded: bool,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Client for the Kokoro TTS sidecar.
pub struct TtsClient {
    http: reqwest::Client,
    base_url: String,
}

impl TtsClient {
    /// Build from `JMCP_TTS_URL` (default `http://127.0.0.1:18901`).
    pub fn from_env() -> Self {
        Self::new(env_url("JMCP_TTS_URL", DEFAULT_TTS_URL))
    }

    /// Build against an explicit base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Read the sidecar health (model, device, voice, loaded).
    pub async fn health(&self) -> Result<TtsHealth> {
        let url = format!("{}/health", self.base_url);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        response
            .error_for_status()?
            .json::<TtsHealth>()
            .await
            .context("parse TTS /health")
    }

    /// Synthesize `text` to WAV bytes (24 kHz, PCM_16). `voice`/`speed` are
    /// optional overrides of the sidecar defaults.
    pub async fn synthesize(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
    ) -> Result<Vec<u8>> {
        self.synthesize_as(text, voice, speed, AudioFormat::Wav)
            .await
    }

    /// Synthesize `text` in the requested [`AudioFormat`]. Use
    /// [`AudioFormat::OggOpus`] for Telegram voice notes (`sendVoice`).
    pub async fn synthesize_as(
        &self,
        text: &str,
        voice: Option<&str>,
        speed: Option<f32>,
        format: AudioFormat,
    ) -> Result<Vec<u8>> {
        let url = format!("{}/synthesize?format={}", self.base_url, format.query());
        let mut body = serde_json::json!({ "text": text });
        if let Some(voice) = voice {
            body["voice"] = serde_json::json!(voice);
        }
        if let Some(speed) = speed {
            body["speed"] = serde_json::json!(speed);
        }
        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let bytes = response
            .error_for_status()?
            .bytes()
            .await
            .context("read TTS audio bytes")?;
        Ok(bytes.to_vec())
    }
}

/// Audio container/codec the TTS sidecar emits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioFormat {
    /// WAV (24 kHz, PCM_16) — the default.
    Wav,
    /// OGG/Opus — required by Telegram `sendVoice`.
    OggOpus,
}

impl AudioFormat {
    fn query(self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::OggOpus => "ogg",
        }
    }
}

#[cfg(test)]
mod speech_tests;
