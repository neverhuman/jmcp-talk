use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTurnState {
    Started,
    Transcribed,
    ReasoningStarted,
    ToolCandidate,
    ConfirmationRequested,
    AudioStarted,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurnAudio {
    pub input_hash: Option<String>,
    pub input_ref: Option<String>,
    pub input_sample_rate: Option<u32>,
    pub output_hash: Option<String>,
    pub output_ref: Option<String>,
    pub output_sample_rate: Option<u32>,
    pub degraded_voice: bool,
    pub voice_profile_hash: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurnMetrics {
    pub first_token_latency_ms: Option<f64>,
    pub first_audio_latency_ms: Option<f64>,
    pub asr_rtf: Option<f64>,
    pub tts_rtf: Option<f64>,
    pub underruns: u64,
    pub clipping_count: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct VoiceTurnEvidence {
    pub kind: String,
    pub uri: String,
    pub captured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceTurnRecord {
    pub id: Uuid,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub channel: String,
    pub speaker_id: String,
    pub state: VoiceTurnState,
    pub transcript: String,
    pub reply: String,
    pub confidence: f32,
    pub tool_policy_decision: Option<String>,
    pub tool_name: Option<String>,
    pub confirmation_required: bool,
    pub confirmation_evidence: Vec<VoiceTurnEvidence>,
    pub audio: VoiceTurnAudio,
    pub metrics: VoiceTurnMetrics,
    pub error_class: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl VoiceTurnRecord {
    pub fn new(
        turn_id: impl Into<String>,
        session_id: Option<String>,
        channel: impl Into<String>,
        speaker_id: impl Into<String>,
        voice_profile_hash: Option<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            turn_id: turn_id.into(),
            session_id,
            channel: channel.into(),
            speaker_id: speaker_id.into(),
            state: VoiceTurnState::Started,
            transcript: String::new(),
            reply: String::new(),
            confidence: 0.0,
            tool_policy_decision: None,
            tool_name: None,
            confirmation_required: false,
            confirmation_evidence: Vec::new(),
            audio: VoiceTurnAudio {
                voice_profile_hash,
                ..VoiceTurnAudio::default()
            },
            metrics: VoiceTurnMetrics::default(),
            error_class: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }
}

#[derive(Clone)]
pub struct CoreVoiceTurnRecorder {
    enabled: bool,
    timeout: Duration,
    tx: Option<mpsc::UnboundedSender<QueuedVoiceTurn>>,
}

struct QueuedVoiceTurn {
    payload: Value,
    final_ack: Option<oneshot::Sender<()>>,
}

impl CoreVoiceTurnRecorder {
    pub fn new(base_url: String, enabled: bool, timeout: Duration, client: Client) -> Self {
        if !enabled {
            return Self {
                enabled,
                timeout,
                tx: None,
            };
        }

        let (tx, mut rx) = mpsc::unbounded_channel::<QueuedVoiceTurn>();
        let post_url = format!("{}/voice/turns", base_url.trim_end_matches('/'));
        tokio::spawn(async move {
            while let Some(item) = rx.recv().await {
                let result = client
                    .post(&post_url)
                    .json(&item.payload)
                    .timeout(timeout)
                    .send()
                    .await;
                match result {
                    Ok(response) if response.status().is_success() => {}
                    Ok(response) => {
                        let status = response.status();
                        let body = match response.text().await {
                            Ok(body) => body,
                            Err(_) => String::new(),
                        };
                        eprintln!(
                            "[jmcp-voiced] core turn record failed: non-2xx status={status} body={body}"
                        );
                    }
                    Err(err) => {
                        eprintln!("[jmcp-voiced] core turn record failed: {err}");
                    }
                }
                if let Some(ack) = item.final_ack {
                    let _ = ack.send(());
                }
            }
        });

        Self {
            enabled,
            timeout,
            tx: Some(tx),
        }
    }

    pub fn submit_background(&self, snapshot: &VoiceTurnRecord) {
        if !self.enabled {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        let payload = match serde_json::to_value(snapshot) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[jmcp-voiced] core turn encode failed: {err}");
                return;
            }
        };
        let _ = tx.send(QueuedVoiceTurn {
            payload,
            final_ack: None,
        });
    }

    pub async fn submit_final(&self, snapshot: &VoiceTurnRecord) {
        if !self.enabled {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        let payload = match serde_json::to_value(snapshot) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[jmcp-voiced] core turn encode failed: {err}");
                return;
            }
        };
        let (ack_tx, ack_rx) = oneshot::channel();
        if tx
            .send(QueuedVoiceTurn {
                payload,
                final_ack: Some(ack_tx),
            })
            .is_err()
        {
            eprintln!("[jmcp-voiced] core turn queue unavailable");
            return;
        }
        let _ = tokio::time::timeout(self.timeout, ack_rx).await;
    }
}

pub type SharedVoiceTurnRecord = Arc<Mutex<VoiceTurnRecord>>;

pub fn update_turn_record<F>(turn: &SharedVoiceTurnRecord, update: F) -> VoiceTurnRecord
where
    F: FnOnce(&mut VoiceTurnRecord),
{
    let mut guard = turn.lock().expect("voice turn lock");
    update(&mut guard);
    guard.updated_at = Utc::now();
    guard.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use std::net::SocketAddr;
    use std::time::Instant;
    use tokio::net::TcpListener;

    #[test]
    fn serializes_core_turn_states_and_payloads() {
        let turn = VoiceTurnRecord::new(
            "turn_test",
            Some("session_test".to_owned()),
            "voice",
            "jmcp-voiced",
            Some("sha256:profile".to_owned()),
        );

        for state in [
            VoiceTurnState::Started,
            VoiceTurnState::Transcribed,
            VoiceTurnState::ReasoningStarted,
            VoiceTurnState::ToolCandidate,
            VoiceTurnState::ConfirmationRequested,
            VoiceTurnState::AudioStarted,
            VoiceTurnState::Completed,
            VoiceTurnState::Interrupted,
            VoiceTurnState::Failed,
        ] {
            let mut snapshot = turn.clone();
            snapshot.state = state;
            let rendered = serde_json::to_value(&snapshot).unwrap();
            assert_eq!(rendered["state"], serde_json::to_value(state).unwrap());
            assert!(rendered.get("audio").is_some());
            assert!(rendered.get("metrics").is_some());
        }
    }

    #[tokio::test]
    async fn recorder_keeps_state_order_without_blocking_background_updates() {
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/voice/turns",
                post(
                    |State(seen): State<Arc<Mutex<Vec<Value>>>>, Json(payload): Json<Value>| async move {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        seen.lock().expect("seen lock").push(payload);
                        Json(serde_json::json!({"ok": true}))
                    },
                ),
            )
            .with_state(Arc::clone(&seen));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let recorder = CoreVoiceTurnRecorder::new(
            format!("http://{addr}"),
            true,
            Duration::from_millis(250),
            Client::builder().build().unwrap(),
        );
        let mut turn = VoiceTurnRecord::new(
            "turn_test",
            Some("session_test".to_owned()),
            "voice",
            "jmcp-voiced",
            Some("sha256:profile".to_owned()),
        );
        turn.state = VoiceTurnState::Started;

        let before = Instant::now();
        recorder.submit_background(&turn);
        assert!(before.elapsed() < Duration::from_millis(10));

        turn.state = VoiceTurnState::Transcribed;
        turn.transcript = "hello".to_owned();
        turn.confirmation_evidence = vec![VoiceTurnEvidence {
            kind: "voice.transcript".to_owned(),
            uri: "sha256:turn-test".to_owned(),
            captured_at: Utc::now(),
        }];
        recorder.submit_background(&turn);

        turn.state = VoiceTurnState::ReasoningStarted;
        recorder.submit_background(&turn);

        turn.state = VoiceTurnState::AudioStarted;
        turn.audio.output_ref = Some("/tmp/turn/audio.wav".to_owned());
        recorder.submit_background(&turn);

        turn.state = VoiceTurnState::Completed;
        turn.reply = "hello back".to_owned();
        recorder.submit_final(&turn).await;

        let captured = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if seen.lock().expect("seen lock").len() >= 5 {
                    return seen.lock().expect("seen lock").clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("captured states");

        assert_eq!(captured.len(), 5);
        assert_eq!(captured[0]["state"], "started");
        assert_eq!(captured[1]["state"], "transcribed");
        assert!(captured[1]["confirmationEvidence"][0]["captured_at"].is_string());
        assert_eq!(captured[2]["state"], "reasoning_started");
        assert_eq!(captured[3]["state"], "audio_started");
        assert_eq!(captured[4]["state"], "completed");

        server.abort();
    }
}
