use crate::keycounter::KeyEvent;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Receiver;
use tokio::time;

pub(crate) struct Collector {
    pool: sqlx::SqlitePool,
}

impl Collector {
    pub fn new(pool: sqlx::SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn collect(&self, mut receiver: Receiver<KeyEvent>) -> anyhow::Result<()> {
        let mut keys: Vec<KeyEvent> = Vec::new();

        let mut interval = time::interval(std::time::Duration::from_secs(10));
        interval.tick().await;

        loop {
            tokio::select! {
                biased;

                _ = interval.tick() => {
                    let keys_to_save = std::mem::take(&mut keys);
                    self.save(keys_to_save).await;
                }

                key_event = receiver.recv() => {
                    if let Some(key_event) = key_event {
                        tracing::info!(key_event = ?key_event, "counted");
                        keys.push(key_event);
                    }
                }
            }
        }
    }

    async fn save(&self, keys: Vec<KeyEvent>) {
        for key_event in keys {
            let result = sqlx::query("INSERT INTO keycount (ts_ms, duration_us) VALUES (?, ?)")
                .bind(key_event.ts_ms as i64)
                .bind(key_event.duration_us as i64)
                .execute(&self.pool)
                .await;

            if let Err(e) = result {
                tracing::warn!(error = ?e, key_event = ?key_event, "failed to insert key");
                continue;
            }
        }
    }
}
