use std::sync::{Mutex, OnceLock};

use super::{
    AudioFormat, SpeechAdapterKind, SpeechRuntime, SpeechRuntimeConfig, SpeechTraceEvent,
    SpeechTraceStatus,
};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[tokio::test]
async fn deterministic_runtime_transcribes_and_synthesizes_without_sidecars() {
    let runtime = SpeechRuntime::deterministic("approve local launch");

    let transcript = runtime
        .transcribe_turn(b"fake".to_vec(), Some("en"))
        .await
        .unwrap();
    assert_eq!(transcript.text, "approve local launch");
    assert_eq!(transcript.confidence, Some(1.0));

    let wav = runtime
        .synthesize_reply("approved", AudioFormat::Wav)
        .await
        .unwrap();
    assert!(wav.starts_with(b"RIFF"));
}

#[test]
fn minicpm_runtime_reports_int4_gpu_spike_and_legacy_fallback() {
    let runtime = SpeechRuntime::new(SpeechRuntimeConfig {
        adapter: SpeechAdapterKind::MinicpmO45,
        asr_url: "http://127.0.0.1:18878".to_owned(),
        tts_url: "http://127.0.0.1:18901".to_owned(),
        minicpm_o45_url: None,
        minicpm_o45_quantization: "int4".to_owned(),
        minicpm_o45_device: "cuda:0".to_owned(),
        deterministic_transcript: "unused".to_owned(),
        minicpm_fixture_transcript: Some("fixture MiniCPM transcript".to_owned()),
    })
    .unwrap();

    let health = runtime.health_summary();
    assert_eq!(health.adapter, SpeechAdapterKind::MinicpmO45);
    assert_eq!(health.model, "MiniCPM-o-4.5");
    assert_eq!(health.quantization.as_deref(), Some("int4"));
    assert_eq!(health.device.as_deref(), Some("cuda:0"));
    assert_eq!(health.fallback, Some(SpeechAdapterKind::LegacyCascade));
}

#[tokio::test]
async fn minicpm_fixture_adapter_returns_fixture_transcript() {
    let runtime = SpeechRuntime::new(SpeechRuntimeConfig {
        adapter: SpeechAdapterKind::MinicpmO45,
        asr_url: "http://127.0.0.1:9".to_owned(),
        tts_url: "http://127.0.0.1:9".to_owned(),
        minicpm_o45_url: None,
        minicpm_o45_quantization: "int4".to_owned(),
        minicpm_o45_device: "cuda:0".to_owned(),
        deterministic_transcript: "unused".to_owned(),
        minicpm_fixture_transcript: Some("MiniCPM fixture heard the operator".to_owned()),
    })
    .unwrap();

    let out = runtime
        .transcribe_turn(Vec::new(), Some("en"))
        .await
        .unwrap();
    assert_eq!(out.text, "MiniCPM fixture heard the operator");
    assert_eq!(out.confidence, Some(1.0));
}

#[test]
fn adapter_names_are_stable_for_env_and_receipts() {
    assert_eq!(
        SpeechAdapterKind::parse("minicpm-o45").unwrap().as_str(),
        "minicpm-o45"
    );
    assert_eq!(
        SpeechAdapterKind::parse("legacy").unwrap().as_str(),
        "legacy-cascade"
    );
    assert_eq!(
        SpeechAdapterKind::parse("offline").unwrap().as_str(),
        "deterministic"
    );
    assert!(SpeechAdapterKind::parse("remote-mystery").is_err());
}

#[test]
fn talk_env_names_override_legacy_aliases() {
    let _guard = env_lock();
    for key in [
        "JMCP_TALK_ADAPTER",
        "JMCP_SPEECH_ADAPTER",
        "JMCP_TALK_ASR_URL",
        "JMCP_ASR_URL",
        "JMCP_TALK_TTS_URL",
        "JMCP_TTS_URL",
        "JMCP_TALK_MINICPM_O45_QUANTIZATION",
        "MINICPM_O45_QUANTIZATION",
    ] {
        std::env::remove_var(key);
    }

    std::env::set_var("JMCP_SPEECH_ADAPTER", "legacy");
    std::env::set_var("JMCP_TALK_ADAPTER", "minicpm-o45");
    std::env::set_var("JMCP_ASR_URL", "http://legacy-asr");
    std::env::set_var("JMCP_TALK_ASR_URL", "http://talk-asr");
    std::env::set_var("MINICPM_O45_QUANTIZATION", "int8");
    std::env::set_var("JMCP_TALK_MINICPM_O45_QUANTIZATION", "int4");

    let config = SpeechRuntimeConfig::from_env().unwrap();
    assert_eq!(config.adapter, SpeechAdapterKind::MinicpmO45);
    assert_eq!(config.asr_url, "http://talk-asr");
    assert_eq!(config.minicpm_o45_quantization, "int4");

    for key in [
        "JMCP_TALK_ADAPTER",
        "JMCP_SPEECH_ADAPTER",
        "JMCP_TALK_ASR_URL",
        "JMCP_ASR_URL",
        "MINICPM_O45_QUANTIZATION",
        "JMCP_TALK_MINICPM_O45_QUANTIZATION",
    ] {
        std::env::remove_var(key);
    }
}

#[test]
fn trace_events_are_non_secret_adapter_receipts() {
    let runtime = SpeechRuntime::deterministic("status");
    let health = runtime.health_summary();

    let started = SpeechTraceEvent::new("asr.started", SpeechTraceStatus::Started, &health);
    assert_eq!(started.adapter, SpeechAdapterKind::Deterministic);
    assert_eq!(started.model, "jmcp-deterministic-speech");
    assert_eq!(started.error_class, None);

    let failed = SpeechTraceEvent::failed("tts.done", &health, "timeout");
    assert_eq!(failed.status, SpeechTraceStatus::Failed);
    assert_eq!(failed.error_class.as_deref(), Some("timeout"));
}
