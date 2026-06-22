use crate::server::routes;
use crate::shared::signer::Signer;
use anyhow::Context;
use axum::extract::FromRef;
use axum::{routing::post, Router};
use sqlx::{migrate, PgPool};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::TraceLayer;
use tracing::error;

#[derive(Clone, FromRef)]
struct AppState {
    signer: Signer,
    pool: PgPool,
}

pub(crate) struct Server {
    signer: Signer,
}

impl Server {
    pub fn new(signer: Signer) -> Self {
        Self { signer }
    }

    pub async fn run_server(&self) -> anyhow::Result<()> {
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL not set")?;

        let pool = PgPool::connect(&database_url)
            .await
            .context("failed to connect to database")?;

        if let Err(e) = migrate!("./migration/server").run(&pool).await {
            error!(error = %e, "can't run migrations");
            std::process::exit(1);
        }

        let state = AppState {
            signer: self.signer.clone(),
            pool,
        };

        let app = Router::new()
            .route("/auth/register", post(routes::auth::register))
            .route("/key_events", post(routes::stats::events))
            .layer(TraceLayer::new_for_http())
            .layer(CatchPanicLayer::new())
            .layer(RequestDecompressionLayer::new())
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("0.0.0.0:4444").await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}
