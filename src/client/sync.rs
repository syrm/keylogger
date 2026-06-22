use crate::shared::event::{KeyEvent, KeyEventWire, KeyEventsPayload, KeyEventsPayloadWire};
use crate::shared::signer::Signer;
use crate::shared::wirer::encode;
use flate2::{write::GzEncoder, Compression};
use futures_util::TryStreamExt;
use std::io::Write;
use std::time::Duration;
use tokio::time::{self, MissedTickBehavior};
use tracing::{error, info};

pub(crate) struct Sync {
    pool: sqlx::SqlitePool,
    signer: Signer,
    origin_id: i32,
}

impl Sync {
    pub fn new(pool: sqlx::SqlitePool, signer: Signer) -> Self {
        Self {
            pool,
            signer,
            origin_id: 0,
        }
    }

    pub async fn sync(&mut self) -> anyhow::Result<()> {
        self.origin_id = sqlx::query_scalar(
            r#"
            SELECT value
            FROM metadata
            WHERE key = 'origin_id'
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .or_else(|_| return Err(anyhow::anyhow!("you should register first")))?;

        let mut interval = time::interval(Duration::from_secs(300));
        // Si un tick est manqué (tâche trop lente), on retarde le suivant
        // au lieu de rafale-rattraper.
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            interval.tick().await;
            info!("tick");
            if let Err(e) = self.send_events().await {
                error!(error = %e, "can't send events");
            }
            info!("events sent");
        }
    }

    async fn send_events(&mut self) -> anyhow::Result<()> {
        let pool = self.pool.clone();
        let mut keycodes = sqlx::query_as::<_, KeyEvent>(
            r#"
            SELECT id, ts_ms, duration_ms, key_type
            FROM keyevent
            WHERE id > (SELECT value FROM metadata WHERE key = 'last_event_id_synced')
            ORDER BY id
            "#,
        )
        .fetch(&pool);

        info!("Fetched events");
        let mut events = vec![];

        while let Some(event) = keycodes.try_next().await? {
            events.push(event);

            if events.len() > 50000 {
                if let Ok(last_event_id) = self.send_event(events.clone()).await {
                    sqlx::query(
                        r#"
                        UPDATE metadata SET value = $1
                        WHERE key = 'last_event_id_synced' AND value < $1
                        "#,
                    )
                    .bind(last_event_id)
                    .execute(&pool)
                    .await?;
                }

                events.clear();
            }
        }

        if events.len() > 0 {
            if let Ok(last_event_id) = self.send_event(events).await {
                sqlx::query(
                    r#"
                        UPDATE metadata SET value = $1
                        WHERE key = 'last_event_id_synced' AND value < $1
                        "#,
                )
                .bind(last_event_id)
                .execute(&pool)
                .await?;
            }
        }

        Ok(())
    }

    async fn send_event(&mut self, events: Vec<KeyEvent>) -> anyhow::Result<i64> {
        if events.is_empty() {
            return Ok(0);
        }

        let mut key_events_payload = KeyEventsPayload {
            events: events.clone(),
            origin_id: self.origin_id,
            issued_at: 0,
            signature: "".to_string(),
        };

        key_events_payload = self.signer.sign_events(key_events_payload)?;

        let server_url = "http://localhost:4444";

        reqwest::Client::new()
            .post(format!("{server_url}/key_events"))
            .body(encode(&key_events_payload)?)
            .send()
            .await?
            .error_for_status()?;

        info!("keyEvents sent: {}", events.len());

        Ok(events.last().expect("events is empty").id)
    }
}
