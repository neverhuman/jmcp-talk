//! Thin HTTP clients for the JMCP speech sidecars.
//!
//! The heavy CUDA/ML speech stack runs out-of-process in two Python sidecars
//! (`services/speech/`): faster-whisper distil-small.en for realtime ASR and
//! Kokoro-82M for TTS.
//! This crate is the Rust side — exactly like [`jmcp_adapter_jekko`] shells out
//! to a separate engine, these clients call the sidecars over localhost HTTP and
//! never embed PyTorch/CUDA in the runtime.
//!
//! - [`AsrClient`] — `POST /transcribe` (raw audio bytes) → [`Transcription`].
//! - [`TtsClient`] — `POST /synthesize` (text) → WAV bytes (24 kHz, PCM_16).
//!
//! Both default to JMCP-safe localhost ports and are overridable via
//! `JMCP_TALK_ASR_URL` / `JMCP_TALK_TTS_URL`. Older `JMCP_ASR_URL` and
//! `JMCP_TTS_URL` aliases remain secondary degradeds only.

use anyhow::{Context, Result};
use serde::Deserialize;

const DEFAULT_ASR_URL: &str = "http://127.0.0.1:18878";
const DEFAULT_TTS_URL: &str = "http://127.0.0.1:18901";

fn env_url(primary: &str, secondary: &str, default: &str) -> String {
    match std::env::var(primary) {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) | Err(std::env::VarError::NotUnicode(_)) => default.to_owned(),
        Err(std::env::VarError::NotPresent) => match std::env::var(secondary) {
            Ok(value) if !value.trim().is_empty() => value,
            _ => default.to_owned(),
        },
    }
}

/// An AGENT-FRIENDLY typed error for the speech-sidecar HTTP clients.
///
/// When a sidecar call fails at the transport layer (the process is down, the
/// port is wrong, or nothing is listening), a bare `reqwest` error tells the
/// next agent almost nothing actionable. This type instead carries the explicit,
/// machine- and agent-readable context needed to recover without guessing: *what*
/// was attempted ([`purpose`](Self::purpose)), *why* it failed
/// ([`reason`](Self::reason)), a short list of [`common_fixes`](Self::common_fixes),
/// a [`docs_url`](Self::docs_url) runbook, and a concrete
/// [`repair_hint`](Self::repair_hint) naming where to rerun once fixed.
///
/// It implements [`std::error::Error`]/[`std::fmt::Display`] by hand (no extra
/// dependency) and is attached as `anyhow` context at each call site, so the
/// existing `anyhow::Result` signatures keep compiling unchanged. This mirrors
/// the `jmcp_adapter_sdk::AdapterError` pattern used by the core adapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechError {
    /// What the client was trying to accomplish when it failed.
    pub purpose: String,
    /// Why it failed, in human-readable terms.
    pub reason: String,
    /// Ordered list of concrete things an operator/agent can try.
    pub common_fixes: Vec<&'static str>,
    /// URL of the runbook / docs for this failure class.
    pub docs_url: &'static str,
    /// Concrete next step naming exactly where to rerun once fixed.
    pub repair_hint: String,
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "speech sidecar error: purpose={}; reason={}",
            self.purpose, self.reason
        )?;
        if !self.common_fixes.is_empty() {
            write!(f, "; common_fixes=[{}]", self.common_fixes.join(", "))?;
        }
        write!(f, "; docs_url={}", self.docs_url)?;
        write!(f, "; repair_hint={}", self.repair_hint)
    }
}

impl std::error::Error for SpeechError {}

impl SpeechError {
    /// Build the agent-friendly error for an unreachable `service` sidecar at
    /// `url` (e.g. `service = "ASR"`, `url = "http://127.0.0.1:18878"`).
    pub fn sidecar_unreachable(service: &str, url: &str) -> Self {
        Self {
            purpose: format!("reach the {service} speech sidecar at {url}"),
            reason: format!(
                "the {service} sidecar did not answer at {url}; the process is likely not running or the URL is wrong"
            ),
            common_fixes: vec![
                "start the speech sidecars (services/speech/run-*.sh)",
                "confirm JMCP_TALK_ASR_URL / JMCP_TALK_TTS_URL point at the live ports",
                "check the sidecar logs for a load/CUDA error before the port opened",
            ],
            docs_url: "https://docs.jmcp.dev/talk/speech-sidecars",
            repair_hint: format!(
                "bring the {service} sidecar up at {url}, then re-run the speech turn"
            ),
        }
    }
}

/// Health snapshot of the ASR sidecar (`GET /health`).
#[derive(Clone, Debug, Deserialize)]
pub struct AsrHealth {
    pub ok: bool,
    pub model: String,
    pub device: String,
    #[serde(default)]
    pub compute_type: Option<String>,
    #[serde(default)]
    pub beam_size: Option<u32>,
    pub loaded: bool,
    #[serde(default)]
    pub warmed: bool,
    #[serde(default)]
    pub last_elapsed_ms: Option<f64>,
    #[serde(default)]
    pub last_warmup_ms: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub warm_error: Option<String>,
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
    pub elapsed_ms: Option<f64>,
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
    /// Build from `JMCP_TALK_ASR_URL`, then the secondary `JMCP_ASR_URL`
    /// and then `http://127.0.0.1:18878`.
    pub fn from_env() -> Self {
        Self::new(env_url(
            "JMCP_TALK_ASR_URL",
            "JMCP_ASR_URL",
            DEFAULT_ASR_URL,
        ))
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
            .with_context(|| SpeechError::sidecar_unreachable("ASR", &url))?;
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
            .with_context(|| SpeechError::sidecar_unreachable("ASR", &url))?;
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
    pub warmed: bool,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub last_elapsed_ms: Option<f64>,
    #[serde(default)]
    pub last_warmup_ms: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub warm_error: Option<String>,
}

/// Client for the Kokoro TTS sidecar.
pub struct TtsClient {
    http: reqwest::Client,
    base_url: String,
}

impl TtsClient {
    /// Build from `JMCP_TALK_TTS_URL`, then the secondary `JMCP_TTS_URL`
    /// and then `http://127.0.0.1:18901`.
    pub fn from_env() -> Self {
        Self::new(env_url(
            "JMCP_TALK_TTS_URL",
            "JMCP_TTS_URL",
            DEFAULT_TTS_URL,
        ))
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
            .with_context(|| SpeechError::sidecar_unreachable("TTS", &url))?;
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
            .with_context(|| SpeechError::sidecar_unreachable("TTS", &url))?;
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
mod error_tests {
    use super::SpeechError;

    #[test]
    fn sidecar_unreachable_display_carries_agent_context() {
        let err = SpeechError::sidecar_unreachable("ASR", "http://127.0.0.1:18878");
        let rendered = err.to_string();
        // The Display must surface every recovery field so the next agent can
        // act without reading the source.
        assert!(
            rendered.contains(&err.purpose),
            "purpose missing: {rendered}"
        );
        assert!(rendered.contains(&err.reason), "reason missing: {rendered}");
        assert!(
            rendered.contains(&err.repair_hint),
            "repair_hint missing: {rendered}"
        );
        assert!(
            rendered.contains(err.docs_url),
            "docs_url missing: {rendered}"
        );
        for fix in &err.common_fixes {
            assert!(
                rendered.contains(fix),
                "common_fix `{fix}` missing: {rendered}"
            );
        }
        assert!(rendered.contains("ASR") && rendered.contains("18878"));
    }

    #[test]
    fn sidecar_unreachable_survives_anyhow_context() {
        let url = "http://127.0.0.1:18901";
        // Attaching the typed error as anyhow context (exactly how the clients use
        // it) keeps the recovery fields in the rendered chain.
        let chained: anyhow::Error = anyhow::anyhow!("connection refused")
            .context(SpeechError::sidecar_unreachable("TTS", url));
        let rendered = format!("{chained:#}");
        assert!(rendered.contains("docs_url="));
        assert!(rendered.contains("repair_hint="));
        assert!(rendered.contains("connection refused"));
    }
}

#[cfg(test)]
mod speech_tests;
