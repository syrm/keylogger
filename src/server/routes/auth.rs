use crate::shared::request::{RegisterRequest, RegisterResponse};
use crate::MAX_CLOCK_SKEW_SECS;
use axum::{extract::State, http::StatusCode, Json};
use ed25519_dalek::VerifyingKey;
use sqlx::PgPool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegisterError {
    #[error("invalid or expired invitation code")]
    InvalidCode,
    #[error("invitation already used")]
    _AlreadyUsed,
    #[error("invalid public key format")]
    _InvalidPublicKey,
    #[error("expired timestamp")]
    ExpiredTimestamp,
    #[error("invalid signature")]
    InvalidSignature,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl axum::response::IntoResponse for RegisterError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            Self::InvalidCode
            | Self::_AlreadyUsed
            | Self::_InvalidPublicKey
            | Self::InvalidSignature
            | Self::ExpiredTimestamp => StatusCode::BAD_REQUEST,
            Self::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

pub async fn register(
    State(mut signer): State<crate::shared::signer::Signer>,
    State(pool): State<PgPool>,
    Json(register_request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, RegisterError> {
    let public_key_bytes =
        hex::decode(register_request.clone().public_key).expect("invalid hex public key");
    let verifying_key = VerifyingKey::from_bytes(
        public_key_bytes[0..32]
            .try_into()
            .expect("invalid public key"),
    )
    .expect("invalid public key");

    match signer.verify_register(
        &register_request.clone(),
        &verifying_key,
        MAX_CLOCK_SKEW_SECS,
    ) {
        Ok(_) => (),
        Err(_) => return Err(RegisterError::InvalidSignature),
    };

    let signed_register_request = match signer.sign_register(register_request.clone()) {
        Ok(signed_register_request) => signed_register_request,
        Err(_) => return Err(RegisterError::InvalidSignature),
    };

    if signed_register_request.signature != register_request.signature {
        return Err(RegisterError::InvalidSignature);
    }

    // 2. Vérifier l'invitation dans une transaction
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        SELECT 1
        FROM invitation
        WHERE code = $1
        "#,
    )
    .bind(register_request.code.clone())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, code = %register_request.code, "query invitation");
        RegisterError::Db(e)
    })?
    .ok_or(RegisterError::InvalidCode)?;

    // 3. Créer l'origin
    let origin_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO origin (name, public_key)
        VALUES ($1, $2)
        RETURNING id
        "#,
    )
    .bind(register_request.name)
    .bind(register_request.public_key)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, code = %register_request.code, "query origin");
        RegisterError::Db(e)
    })?;

    // 4. Marquer l'invitation comme utilisée
    sqlx::query(
        r#"
        DELETE FROM invitation WHERE code = $1
        "#,
    )
    .bind(register_request.code.clone())
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, code = %register_request.code, "delete invitation");
        RegisterError::Db(e)
    })?;

    tx.commit().await?;

    Ok(Json(RegisterResponse { origin_id }))
}
