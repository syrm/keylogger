use crate::server::event::KeyType;
use crate::shared::event::KeyEventsPayload;
use crate::shared::signer::Signer;
use crate::shared::wirer::decode;
use crate::MAX_CLOCK_SKEW_SECS;
use axum::body::Bytes;
use axum::{extract::State, http::StatusCode, Json};
use ed25519_dalek::VerifyingKey;
use sqlx::PgPool;
use thiserror::Error;
use tracing::{error, info};

#[derive(Debug, Error)]
pub enum StatsError {
    #[error("invalid origin id")]
    InvalidOriginID,
    #[error("invalid public key format")]
    _InvalidPublicKey,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("error signature")]
    ErrorSignature,
    #[error("invalid payload")]
    InvalidPayload(#[from] anyhow::Error),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("none")]
    None,
}

impl axum::response::IntoResponse for StatsError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            Self::_InvalidPublicKey
            | Self::InvalidOriginID
            | Self::InvalidSignature
            | Self::ErrorSignature => {
                error!(error = %self, "error");
                StatusCode::BAD_REQUEST
            }
            Self::InvalidPayload(e) => {
                error!(error = %e, "invalid payload");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Db(e) => {
                error!(error = %e, "db error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::None => StatusCode::OK,
        };
        (status, self.to_string()).into_response()
    }
}

pub async fn events(
    State(signer): State<Signer>,
    State(pool): State<PgPool>,
    body: Bytes,
) -> Result<(), StatsError> {
    let mut tx = pool.begin().await?;

    let events_payload = decode(&body).map_err(|e| StatsError::InvalidPayload(e))?;
    info!("keyEvents received: {}", events_payload.events.len());

    let public_key: String = sqlx::query_scalar(
        r#"
        SELECT public_key
        FROM origin
        WHERE id = $1
        "#,
    )
    .bind(events_payload.origin_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        error!(error = %e, id = %events_payload.origin_id, "query public key");
        StatsError::Db(e)
    })?
    .ok_or(StatsError::InvalidOriginID)?;

    let public_key_bytes = hex::decode(public_key).expect("invalid hex public key");
    let verifying_key = VerifyingKey::from_bytes(
        public_key_bytes[0..32]
            .try_into()
            .expect("invalid public key"),
    )
    .expect("invalid public key");

    match signer.verify_events(&events_payload.clone(), &verifying_key, MAX_CLOCK_SKEW_SECS) {
        Ok(_) => (),
        Err(_) => return Err(StatsError::InvalidSignature),
    };

    for event in events_payload.events.clone() {
        sqlx::query(
            r#"
            INSERT INTO keyevent (origin_id, origin_event_id, timestamp_ms, key_type, duration_ms)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(events_payload.origin_id)
        .bind(event.id)
        .bind(event.ts_ms)
        .bind(KeyType::from(event.key_type))
        .bind(event.duration_ms)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if let Some(db_error) = e.as_database_error()
                && db_error.is_unique_violation()
            {
                return StatsError::None;
            }
            error!(error = %e, id = %events_payload.origin_id, "insert keyevent");
            StatsError::Db(e)
        })?;
    }
    tx.commit().await?;
    info!("keyEvents inserted: {}", events_payload.events.len());

    Ok(())
}
