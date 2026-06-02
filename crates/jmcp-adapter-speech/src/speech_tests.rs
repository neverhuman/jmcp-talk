use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use super::{AsrClient, TtsClient};

/// Spin a one-shot stub HTTP server that answers the next request with a fixed
/// response, and return its base URL. Mirrors the jailgun adapter's test stubs.
fn stub_server(status_line: &'static str, content_type: &'static str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf); // best-effort drain of the request
            let header = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn asr_health_parses() {
    let url = stub_server(
        "200 OK",
        "application/json",
        br#"{"ok":true,"model":"large-v3","device":"cuda","loaded":true,"error":null}"#.to_vec(),
    );
    let health = AsrClient::new(url).health().await.unwrap();
    assert!(health.ok);
    assert!(health.loaded);
    assert_eq!(health.model, "large-v3");
    assert_eq!(health.device, "cuda");
}

#[tokio::test]
async fn asr_transcribe_parses_text_and_segments() {
    let url = stub_server(
        "200 OK",
        "application/json",
        br#"{"text":"hello world","language":"en","language_probability":0.99,"confidence":0.88,"duration":1.5,"rtf":0.05,"segments":[{"start":0.0,"end":1.5,"text":"hello world"}]}"#.to_vec(),
    );
    let out = AsrClient::new(url)
        .transcribe(b"fake-wav".to_vec(), Some("en"))
        .await
        .unwrap();
    assert_eq!(out.text, "hello world");
    assert_eq!(out.language, "en");
    assert_eq!(out.segments.len(), 1);
    assert_eq!(out.rtf, Some(0.05));
    assert_eq!(out.confidence, Some(0.88));
}

#[tokio::test]
async fn asr_confidence_absent_is_none() {
    let url = stub_server(
        "200 OK",
        "application/json",
        br#"{"text":"x","language":"en","segments":[]}"#.to_vec(),
    );
    let out = AsrClient::new(url)
        .transcribe(b"x".to_vec(), None)
        .await
        .unwrap();
    assert_eq!(out.confidence, None);
}

#[tokio::test]
async fn asr_surfaces_server_error() {
    let url = stub_server(
        "503 Service Unavailable",
        "application/json",
        br#"{"error":"model still loading"}"#.to_vec(),
    );
    let result = AsrClient::new(url).transcribe(b"x".to_vec(), None).await;
    assert!(result.is_err(), "5xx must surface as an error");
}

#[tokio::test]
async fn tts_health_parses() {
    let url = stub_server(
        "200 OK",
        "application/json",
        br#"{"ok":true,"model":"kokoro-82M","device":"cuda","loaded":true,"voice":"af_heart","sample_rate":24000,"error":null}"#.to_vec(),
    );
    let health = TtsClient::new(url).health().await.unwrap();
    assert!(health.ok && health.loaded);
    assert_eq!(health.model, "kokoro-82M");
    assert_eq!(health.sample_rate, Some(24000));
}

#[tokio::test]
async fn tts_synthesize_returns_audio_bytes() {
    // A minimal WAV header is enough to assert the bytes round-trip through the client.
    let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    let url = stub_server("200 OK", "audio/wav", wav.clone());
    let bytes = TtsClient::new(url)
        .synthesize("master control plane online", Some("af_heart"), Some(1.0))
        .await
        .unwrap();
    assert_eq!(bytes, wav);
    assert!(bytes.starts_with(b"RIFF"));
}

/// Like `stub_server`, but captures the first request line so a test can assert
/// on the URL the client built (query string, method, path).
fn stub_server_capturing(
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let seen_writer = seen.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or("").to_owned();
            *seen_writer.lock().unwrap() = first_line;
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), seen)
}

#[tokio::test]
async fn tts_synthesize_as_ogg_requests_ogg_and_returns_bytes() {
    use super::AudioFormat;
    let ogg = b"OggS\x00\x02fake-opus".to_vec();
    let (url, seen) = stub_server_capturing("audio/ogg", ogg.clone());
    let bytes = TtsClient::new(url)
        .synthesize_as("approve the deployment", None, None, AudioFormat::OggOpus)
        .await
        .unwrap();
    assert_eq!(bytes, ogg);
    assert!(bytes.starts_with(b"OggS"), "Telegram voice needs Ogg/Opus");
    let request_line = seen.lock().unwrap().clone();
    assert!(
        request_line.contains("format=ogg"),
        "client must request ?format=ogg, got: {request_line}"
    );
}
