use super::{AudioFormat, SpeechAdapterKind, SpeechRuntime, SpeechRuntimeConfig};

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
