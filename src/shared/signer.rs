use crate::shared::event::{KeyEventsPayload, SignedKeyEventsPayload, ACTION_EVENTS};
use crate::shared::request::{RegisterRequest, SignedRegisterRequest, ACTION_REGISTER};
use anyhow::bail;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::rand_core::UnwrapErr;
use ed25519_dalek::{Signature, VerifyingKey};
use ed25519_dalek::{Signer as DalekSigner, SigningKey};
use rand::rngs::SysRng;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub(crate) struct Signer {
    signing_key: Option<SigningKey>,
    path_key: PathBuf,
}

impl Signer {
    pub fn new(path_key: &Path) -> Self {
        Self {
            signing_key: None,
            path_key: PathBuf::from(path_key),
        }
    }

    fn new_key(&self) -> anyhow::Result<SigningKey> {
        let mut csprng = UnwrapErr(SysRng);
        let signing_key = SigningKey::generate(&mut csprng);
        let bytes = signing_key.to_bytes();

        std::fs::write(self.path_key.as_path(), bytes)?;
        std::fs::set_permissions(
            self.path_key.as_path(),
            std::fs::Permissions::from_mode(0o600),
        )?;

        Ok(signing_key)
    }

    fn load_signing_key(&self) -> anyhow::Result<SigningKey> {
        let bytes = std::fs::read(self.path_key.as_path())?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid key file: expected 32 bytes"))?;

        Ok(SigningKey::from_bytes(&bytes))
    }

    pub fn get_public_key(&mut self) -> anyhow::Result<String> {
        if self.signing_key.is_none() {
            let signing_key = match self.load_signing_key() {
                Ok(signing_key) => signing_key,
                Err(_) => self.new_key()?,
            };

            self.signing_key = Some(signing_key);
        }

        Ok(hex::encode(
            self.signing_key
                .as_ref()
                .unwrap()
                .verifying_key()
                .as_bytes(),
        ))
    }

    fn get_timestamp(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("horloge système avant 1970")
            .as_secs()
    }

    fn sign(&mut self, message: &[u8]) -> anyhow::Result<String> {
        self.get_public_key()?;

        match self.signing_key.clone() {
            None => bail!("no signing key"),
            Some(signing_key) => Ok(URL_SAFE_NO_PAD.encode(signing_key.sign(message).to_bytes())),
        }
    }

    pub fn sign_register(&mut self, register: RegisterRequest) -> anyhow::Result<RegisterRequest> {
        let public_key_b64 = self.get_public_key()?;
        let timestamp = self.get_timestamp();

        let to_sign = SignedRegisterRequest {
            action: ACTION_REGISTER.parse()?,
            code: register.code.clone(),
            name: register.name.clone(),
            public_key: public_key_b64.clone(),
            issued_at: timestamp,
        };
        let message = serde_json::to_vec(&to_sign).expect("can't serialize to json");
        let signature_b64 = self.sign(&message)?;

        let new_register_request = RegisterRequest {
            code: register.code,
            name: register.name,
            public_key: public_key_b64,
            issued_at: timestamp,
            signature: signature_b64,
        };

        anyhow::Ok(new_register_request)
    }

    pub fn sign_events(
        &mut self,
        events_payload: KeyEventsPayload,
    ) -> anyhow::Result<KeyEventsPayload> {
        let timestamp = self.get_timestamp();

        let to_sign = SignedKeyEventsPayload {
            action: ACTION_EVENTS.parse()?,
            events: events_payload.events.clone(),
            origin_id: events_payload.origin_id,
            issued_at: timestamp,
        };
        let message = serde_json::to_vec(&to_sign).expect("can't serialize to json");
        let signature_b64 = self.sign(&message)?;

        let new_events_payload = KeyEventsPayload {
            events: events_payload.events,
            origin_id: events_payload.origin_id,
            issued_at: timestamp,
            signature: signature_b64,
        };

        anyhow::Ok(new_events_payload)
    }

    pub fn verify(
        &self,
        message: &[u8],
        signature_b64: &str,
        verifying_key: &VerifyingKey,
    ) -> anyhow::Result<()> {
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(signature_b64)
            .map_err(|e| anyhow::anyhow!("invalid base64 signature: {e}"))?;

        let signature_bytes: [u8; 64] = signature_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("signature has wrong length, expected 64 bytes"))?;

        let signature = Signature::from_bytes(&signature_bytes);

        verifying_key
            .verify_strict(message, &signature)
            .map_err(|e| anyhow::anyhow!("signature verification failed: {e}"))?;

        Ok(())
    }

    pub fn verify_register(
        &self,
        register: &RegisterRequest,
        verifying_key: &VerifyingKey,
        max_age_secs: u64,
    ) -> anyhow::Result<()> {
        let now = self.get_timestamp();
        let age = now
            .checked_sub(register.issued_at)
            .ok_or_else(|| anyhow::anyhow!("issued_at is in the future"))?;
        if age > max_age_secs {
            bail!("signature expired: {age}s old, max {max_age_secs}s allowed");
        }

        let to_verify = SignedRegisterRequest {
            action: ACTION_REGISTER.parse()?,
            code: register.code.clone(),
            name: register.name.clone(),
            public_key: register.public_key.clone(),
            issued_at: register.issued_at,
        };
        let message =
            serde_json::to_vec(&to_verify).expect("failed to serialize payload for verification");

        self.verify(&message, &register.signature, verifying_key)
    }

    pub fn verify_events(
        &self,
        events_payload: &KeyEventsPayload,
        verifying_key: &VerifyingKey,
        max_age_secs: u64,
    ) -> anyhow::Result<()> {
        let now = self.get_timestamp();
        let age = now
            .checked_sub(events_payload.issued_at)
            .ok_or_else(|| anyhow::anyhow!("issued_at is in the future"))?;
        if age > max_age_secs {
            bail!("signature expired: {age}s old, max {max_age_secs}s allowed");
        }

        let to_verify = SignedKeyEventsPayload {
            action: ACTION_EVENTS.parse()?,
            events: events_payload.events.clone(),
            origin_id: events_payload.origin_id,
            issued_at: events_payload.issued_at,
        };
        let message =
            serde_json::to_vec(&to_verify).expect("failed to serialize payload for verification");

        self.verify(&message, &events_payload.signature, verifying_key)
    }
}
