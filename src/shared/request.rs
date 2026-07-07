use serde::{Deserialize, Serialize};

pub(crate) const ACTION_REGISTER: &str = "register";

#[derive(serde::Serialize, Clone)]
pub(crate) struct SignedRegisterRequest {
    pub action: String,
    pub code: String,
    pub name: String,
    pub public_key: String,
    pub issued_at: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct RegisterRequest {
    pub code: String,       // invitation code
    pub name: String,       // nom lisible du client
    pub public_key: String, // base64url-encoded public key (32 bytes Ed25519)
    pub issued_at: u64,
    pub signature: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RegisterResponse {
    pub origin_id: i32,
}
