use serde::{Deserialize, Serialize};

pub(crate) const ACTION_EVENTS: &str = "events";

#[derive(
    Debug, Copy, Clone, Serialize, Deserialize, sqlx::Type, bincode::Encode, bincode::Decode,
)]
#[repr(i32)]
pub(crate) enum KeyType {
    Typing = 1,
    Deletion = 2,
    Other = 3,
}

#[derive(
    Debug, Copy, Clone, Serialize, Deserialize, sqlx::FromRow, bincode::Encode, bincode::Decode,
)]
pub(crate) struct KeyEvent {
    pub id: i64,
    pub ts_ms: i64,
    pub duration_ms: i32,
    pub key_type: KeyType,
}

#[derive(Serialize, Clone)]
pub(crate) struct SignedKeyEventsPayload {
    pub action: String,
    pub events: Vec<KeyEvent>,
    pub origin_id: i32,
    pub issued_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, bincode::Encode, bincode::Decode)]
pub(crate) struct KeyEventsPayload {
    pub events: Vec<KeyEvent>,
    pub origin_id: i32,
    pub issued_at: u64,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct KeyEventsPayloadWire {
    pub origin_id: i32,
    pub issued_at: u64,
    pub signature: String,
    pub id_deltas: Vec<i64>,
    pub ts_deltas: Vec<i64>,
    pub durations_ms: Vec<i32>,
    pub key_types: Vec<KeyType>,
}
