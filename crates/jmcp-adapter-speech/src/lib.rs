//! Thin HTTP clients for the JMCP speech daemon.
//!
//! The default speech service is `jmcp-speechd`, a Rust HTTP daemon with
//! deterministic offline ASR/TTS endpoints. This crate is the client side:
//! runtime code calls the daemon over localhost HTTP and never embeds live model
//! execution in the approval path.
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

/// Health snapshot of the ASR daemon (`GET /health`).
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

/// Client for the ASR daemon.
pub struct AsrClient {
    http: reqwest::Client,
    base_url: String,
}

/// Shared HTTP client for the speech daemon: bounded connect + request timeouts
/// so a stalled service surfaces a typed error instead of hanging.
fn speech_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("speech http client configuration is valid")
}

impl AsrClient {
    /// Build from `JMCP_ASR_URL` (default `http://127.0.0.1:18878`).
    pub fn from_env() -> Self {
        Self::new(env_url("JMCP_ASR_URL", DEFAULT_ASR_URL))
    }

    /// Build against an explicit base URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: speech_http_client(),
            base_url: base_url.into(),
        }
    }

    /// Read the daemon health (model, device, loaded).
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
        let url = format!("{}/transcribe", self.base_url);
        let mut request = self
            .http
            .post(&url)
            .header("content-type", "audio/wav")
            .body(audio);
        if let Some(language) = language {
            request = request.query(&[("language", language)]);
        }
        let response = request
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

/// Health snapshot of the TTS daemon (`GET /health`).
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

/// Client for the TTS daemon.
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
            http: speech_http_client(),
            base_url: base_url.into(),
        }
    }

    /// Read the daemon health (model, device, voice, loaded).
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
    /// optional overrides of the daemon defaults.
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
        let response = response.error_for_status()?;
        let content_type = match response.headers().get(reqwest::header::CONTENT_TYPE) {
            Some(value) => value
                .to_str()
                .context("parse TTS content-type header")?
                .to_owned(),
            None => anyhow::bail!(
                "TTS response omitted content-type for a {} response",
                format.query()
            ),
        };
        if content_type.trim().is_empty() {
            anyhow::bail!(
                "TTS response contained an empty content-type for a {} response",
                format.query()
            );
        }
        let bytes = response.bytes().await.context("read TTS audio bytes")?;
        if bytes.is_empty() {
            anyhow::bail!(
                "TTS returned an empty body for a {} response",
                format.query()
            );
        }
        if !content_type.starts_with("audio/") {
            anyhow::bail!("TTS returned non-audio content-type {content_type:?}");
        }
        Ok(bytes.to_vec())
    }
}

/// Audio container/codec the TTS daemon emits.
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

/// Speech runtime adapter family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechAdapterKind {
    /// MiniCPM-o 4.5 speech-to-speech spike. Live use is opt-in.
    MinicpmO45,
    /// Existing ASR/TTS sidecar cascade.
    LegacyCascade,
    /// Fully deterministic local fixture adapter.
    Deterministic,
}

impl SpeechAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpeechAdapterKind::MinicpmO45 => "minicpm-o45",
            SpeechAdapterKind::LegacyCascade => "legacy-cascade",
            SpeechAdapterKind::Deterministic => "deterministic",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minicpm-o45" | "minicpm_o45" | "minicpm" => Ok(Self::MinicpmO45),
            "legacy-cascade" | "legacy_cascade" | "legacy" => Ok(Self::LegacyCascade),
            "deterministic" | "fixture" | "offline" => Ok(Self::Deterministic),
            other => anyhow::bail!("unsupported speech adapter {other:?}"),
        }
    }
}

/// Runtime configuration for speech turn execution.
#[derive(Clone, Debug)]
pub struct SpeechRuntimeConfig {
    pub adapter: SpeechAdapterKind,
    pub asr_url: String,
    pub tts_url: String,
    pub minicpm_o45_url: Option<String>,
    pub minicpm_o45_quantization: String,
    pub minicpm_o45_device: String,
    pub deterministic_transcript: String,
    pub minicpm_fixture_transcript: Option<String>,
}

impl SpeechRuntimeConfig {
    pub fn from_env() -> Result<Self> {
        let adapter = match std::env::var("JMCP_SPEECH_ADAPTER") {
            Ok(value) if !value.trim().is_empty() => SpeechAdapterKind::parse(&value)?,
            _ => SpeechAdapterKind::LegacyCascade,
        };
        Ok(Self {
            adapter,
            asr_url: env_url("JMCP_ASR_URL", DEFAULT_ASR_URL),
            tts_url: env_url("JMCP_TTS_URL", DEFAULT_TTS_URL),
            minicpm_o45_url: std::env::var("MINICPM_O45_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            minicpm_o45_quantization: std::env::var("MINICPM_O45_QUANTIZATION")
                .unwrap_or_else(|_| "int4".to_owned()),
            minicpm_o45_device: std::env::var("MINICPM_O45_DEVICE")
                .unwrap_or_else(|_| "cuda:0".to_owned()),
            deterministic_transcript: std::env::var("JMCP_DETERMINISTIC_TRANSCRIPT")
                .unwrap_or_else(|_| "deterministic speech turn".to_owned()),
            minicpm_fixture_transcript: std::env::var("MINICPM_O45_FIXTURE_TRANSCRIPT")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        })
    }
}

/// Non-secret runtime health/configuration summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpeechRuntimeHealth {
    pub adapter: SpeechAdapterKind,
    pub model: &'static str,
    pub quantization: Option<String>,
    pub device: Option<String>,
    pub live_model_configured: bool,
    pub fallback: Option<SpeechAdapterKind>,
}

/// Turn-level speech runtime. Core remains responsible for ledgers, approvals,
/// tool policy, and durable side effects.
pub struct SpeechRuntime {
    config: SpeechRuntimeConfig,
    asr: AsrClient,
    tts: TtsClient,
}

impl SpeechRuntime {
    pub fn from_env() -> Result<Self> {
        Self::new(SpeechRuntimeConfig::from_env()?)
    }

    pub fn new(config: SpeechRuntimeConfig) -> Result<Self> {
        Ok(Self {
            asr: AsrClient::new(config.asr_url.clone()),
            tts: TtsClient::new(config.tts_url.clone()),
            config,
        })
    }

    pub fn deterministic(transcript: impl Into<String>) -> Self {
        Self::new(SpeechRuntimeConfig {
            adapter: SpeechAdapterKind::Deterministic,
            asr_url: DEFAULT_ASR_URL.to_owned(),
            tts_url: DEFAULT_TTS_URL.to_owned(),
            minicpm_o45_url: None,
            minicpm_o45_quantization: "int4".to_owned(),
            minicpm_o45_device: "offline".to_owned(),
            deterministic_transcript: transcript.into(),
            minicpm_fixture_transcript: None,
        })
        .expect("deterministic speech runtime config is valid")
    }

    pub fn adapter(&self) -> SpeechAdapterKind {
        self.config.adapter
    }

    pub fn health_summary(&self) -> SpeechRuntimeHealth {
        match self.config.adapter {
            SpeechAdapterKind::MinicpmO45 => SpeechRuntimeHealth {
                adapter: SpeechAdapterKind::MinicpmO45,
                model: "MiniCPM-o-4.5",
                quantization: Some(self.config.minicpm_o45_quantization.clone()),
                device: Some(self.config.minicpm_o45_device.clone()),
                live_model_configured: self.config.minicpm_o45_url.is_some(),
                fallback: Some(SpeechAdapterKind::LegacyCascade),
            },
            SpeechAdapterKind::LegacyCascade => SpeechRuntimeHealth {
                adapter: SpeechAdapterKind::LegacyCascade,
                model: "legacy-asr-tts-cascade",
                quantization: None,
                device: None,
                live_model_configured: true,
                fallback: None,
            },
            SpeechAdapterKind::Deterministic => SpeechRuntimeHealth {
                adapter: SpeechAdapterKind::Deterministic,
                model: "jmcp-deterministic-speech",
                quantization: None,
                device: Some("offline".to_owned()),
                live_model_configured: true,
                fallback: None,
            },
        }
    }

    pub async fn transcribe_turn(
        &self,
        audio: Vec<u8>,
        language: Option<&str>,
    ) -> Result<Transcription> {
        match self.config.adapter {
            SpeechAdapterKind::Deterministic => Ok(deterministic_transcription(
                &self.config.deterministic_transcript,
                language,
            )),
            SpeechAdapterKind::LegacyCascade => self.asr.transcribe(audio, language).await,
            SpeechAdapterKind::MinicpmO45 => {
                if let Some(transcript) = &self.config.minicpm_fixture_transcript {
                    return Ok(deterministic_transcription(transcript, language));
                }
                self.asr.transcribe(audio, language).await.context(
                    "MiniCPM-o 4.5 live adapter is not configured; legacy cascade fallback failed",
                )
            }
        }
    }

    pub async fn synthesize_reply(&self, text: &str, format: AudioFormat) -> Result<Vec<u8>> {
        match self.config.adapter {
            SpeechAdapterKind::Deterministic => Ok(deterministic_audio(format)),
            SpeechAdapterKind::LegacyCascade | SpeechAdapterKind::MinicpmO45 => {
                self.tts.synthesize_as(text, None, None, format).await
            }
        }
    }
}

fn deterministic_transcription(text: &str, language: Option<&str>) -> Transcription {
    Transcription {
        text: text.to_owned(),
        language: language.unwrap_or("en").to_owned(),
        language_probability: 1.0,
        confidence: if text.trim().is_empty() {
            None
        } else {
            Some(1.0)
        },
        duration: 0.0,
        elapsed_ms: Some(0.0),
        rtf: Some(0.0),
        segments: Vec::new(),
    }
}

fn deterministic_audio(format: AudioFormat) -> Vec<u8> {
    match format {
        AudioFormat::Wav => b"RIFF\x24\x00\x00\x00WAVEfmt jmcp-deterministic".to_vec(),
        AudioFormat::OggOpus => b"OggS\0\x02jmcp-deterministic".to_vec(),
    }
}

#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod speech_tests;
