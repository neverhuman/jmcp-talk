use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use jmcp_speechd::{router, SpeechConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn request(
    config: SpeechConfig,
    method: Method,
    uri: &str,
    body: Body,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router(config)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, headers, bytes.to_vec())
}

async fn json_request(
    config: SpeechConfig,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, _, bytes) = request(config, method, uri, Body::from(body.to_string())).await;
    let value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

#[tokio::test]
async fn health_reports_loaded_role_and_offline_provider() {
    let (status, value) = json_request(
        SpeechConfig::new("both", "hello", false),
        Method::GET,
        "/health",
        Value::Null,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(value["loaded"], true);
    assert_eq!(value["provider"], "jmcp-speechd-deterministic");
    assert_eq!(value["role"], "both");
}

#[tokio::test]
async fn transcribe_returns_configured_transcript_and_audio_size() {
    let (status, _, bytes) = request(
        SpeechConfig::new("asr", "approve local proof", false),
        Method::POST,
        "/transcribe?language=es&beam_size=4",
        Body::from(vec![1_u8, 2, 3, 4, 5]),
    )
    .await;
    let value: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["text"], "approve local proof");
    assert_eq!(value["language"], "es");
    assert_eq!(value["beam_size"], 4);
    assert_eq!(value["audio_bytes"], 5);
    assert_eq!(value["confidence"], 1.0);
}

#[tokio::test]
async fn transcribe_empty_text_returns_null_confidence() {
    let (status, _, bytes) = request(
        SpeechConfig::new("asr", "", false),
        Method::POST,
        "/transcribe",
        Body::from(Vec::<u8>::new()),
    )
    .await;
    let value: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["text"], "");
    assert!(value["confidence"].is_null());
}

#[tokio::test]
async fn synthesize_returns_deterministic_wav_and_ogg() {
    let (wav_status, wav_headers, wav_bytes) = request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize?format=wav",
        Body::from(json!({ "text": "speak", "speed": 1.0 }).to_string()),
    )
    .await;
    let (ogg_status, ogg_headers, ogg_bytes) = request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize?format=ogg",
        Body::from(json!({ "text": "speak", "voice": "proof" }).to_string()),
    )
    .await;

    assert_eq!(wav_status, StatusCode::OK);
    assert_eq!(wav_headers[header::CONTENT_TYPE], "audio/wav");
    assert!(wav_bytes.starts_with(b"RIFF"));
    assert_eq!(ogg_status, StatusCode::OK);
    assert_eq!(ogg_headers[header::CONTENT_TYPE], "audio/ogg");
    assert!(ogg_bytes.starts_with(b"OggS"));
    assert!(String::from_utf8_lossy(&ogg_bytes).contains("proof"));
}

#[tokio::test]
async fn role_disabled_and_fail_closed_requests_are_unavailable() {
    let (health_status, health_error) = json_request(
        SpeechConfig::new("both", "", true),
        Method::GET,
        "/health",
        Value::Null,
    )
    .await;
    let (asr_status, asr_error) = json_request(
        SpeechConfig::new("asr", "", false),
        Method::POST,
        "/synthesize",
        json!({ "text": "blocked" }),
    )
    .await;
    let (tts_status, _, tts_error) = request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/transcribe",
        Body::from(vec![9_u8]),
    )
    .await;
    let tts_error: Value = serde_json::from_slice(&tts_error).unwrap();
    let (closed_status, closed_error) = json_request(
        SpeechConfig::new("both", "closed", true),
        Method::POST,
        "/synthesize",
        json!({ "text": "blocked" }),
    )
    .await;

    assert_eq!(health_status, StatusCode::OK);
    assert_eq!(health_error["ok"], false);
    assert_eq!(health_error["loaded"], false);
    assert_eq!(health_error["error"], "fail_closed");
    assert_eq!(asr_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(asr_error["error"], "speech synthesis is disabled");
    assert_eq!(tts_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(tts_error["error"], "speech transcription is disabled");
    assert_eq!(closed_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(closed_error["error"], "speech synthesis is disabled");
}

#[tokio::test]
async fn synthesize_rejects_empty_text_invalid_format_and_speed_bounds() {
    let (empty_status, empty_error) = json_request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize",
        json!({ "text": " " }),
    )
    .await;
    let (format_status, format_error) = json_request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize?format=mp3",
        json!({ "text": "speak" }),
    )
    .await;
    let (slow_status, slow_error) = json_request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize",
        json!({ "text": "speak", "speed": 0.24 }),
    )
    .await;
    let (fast_status, fast_error) = json_request(
        SpeechConfig::new("tts", "", false),
        Method::POST,
        "/synthesize",
        json!({ "text": "speak", "speed": 4.01 }),
    )
    .await;

    assert_eq!(empty_status, StatusCode::BAD_REQUEST);
    assert_eq!(empty_error["error"], "text is required");
    assert_eq!(format_status, StatusCode::BAD_REQUEST);
    assert_eq!(format_error["error"], "format must be wav or ogg");
    assert_eq!(slow_status, StatusCode::BAD_REQUEST);
    assert_eq!(slow_error["error"], "speed must be between 0.25 and 4.0");
    assert_eq!(fast_status, StatusCode::BAD_REQUEST);
    assert_eq!(fast_error["error"], "speed must be between 0.25 and 4.0");
}
