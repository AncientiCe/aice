//! Protocol types for M5Stack pod communication (WebSocket control + audio frames).

use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Audio payload (PCM bytes). Serializes as base64; deserializes from base64 string or byte array.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioPayload(pub Vec<u8>);

impl Serialize for AudioPayload {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.0);
        s.serialize_str(&b64)
    }
}

impl<'de> Deserialize<'de> for AudioPayload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(d)?;
        match value {
            serde_json::Value::String(s) => base64::engine::general_purpose::STANDARD
                .decode(s.as_bytes())
                .map(AudioPayload)
                .map_err(|e| D::Error::custom(e.to_string())),
            serde_json::Value::Array(arr) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    let n = v
                        .as_u64()
                        .ok_or_else(|| D::Error::custom("expected u8 in array"))?
                        as u8;
                    out.push(n);
                }
                Ok(AudioPayload(out))
            }
            _ => Err(D::Error::custom(
                "audio payload must be base64 string or byte array",
            )),
        }
    }
}

/// Message from pod to gateway.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PodToGateway {
    /// Initial hello + protocol version.
    Hello {
        protocol_version: u16,
        device_id: String,
        room: Option<String>,
    },
    /// Audio frame (PCM 16kHz mono 16-bit). Payload is base64 on wire.
    Audio { payload: AudioPayload },
    /// Pod identification / room label.
    Identify {
        device_id: String,
        room: Option<String>,
    },
    /// Tap-to-activate (optional).
    TapActivate,
    /// Keepalive ping from pod.
    Ping { seq: u64 },
}

/// Message from gateway to pod.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayToPod {
    /// Protocol hello ack.
    HelloAck { protocol_version: u16 },
    /// TTS audio to play. Payload is base64 on wire.
    Audio { payload: AudioPayload },
    /// Stop queued/current pod playback immediately.
    StopAudio,
    /// LED state: listening / thinking / speaking.
    Led { state: LedState },
    /// Keepalive pong.
    Pong { seq: u64 },
    /// Structured protocol/runtime error.
    Error { code: String, message: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedState {
    Listening,
    Thinking,
    Speaking,
}
