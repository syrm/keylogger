use crate::client::keycounter;
use crate::client::tray::MyTray;
use crate::client::ui::run_ui;
use crate::client::{collector, sync};
use crate::shared::event::KeyEvent;
use crate::shared::request::{RegisterRequest, RegisterResponse};
use crate::shared::signer::Signer;
use ksni::TrayMethods;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{migrate, SqlitePool};
use std::str::FromStr;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

pub(crate) struct Client {
    signer: Signer,
}

impl Client {
    pub(crate) fn new(signer: Signer) -> Self {
        Self { signer }
    }

    pub(crate) async fn run_client(&mut self) {
        let opts = SqliteConnectOptions::from_str("sqlite://keylogger-dev.db?mode=rwc")
            .expect("invalid db path")
            .journal_mode(SqliteJournalMode::Wal);

        let pool = match SqlitePoolOptions::new().connect_with(opts).await {
            Ok(pool) => pool,
            Err(e) => {
                error!(error = %e, "can't connect to db");
                std::process::exit(1);
            }
        };

        if let Err(e) = migrate!("./migration/client").run(&pool).await {
            error!(error = %e, "can't run migrations");
            std::process::exit(1);
        }

        let arg_type = std::env::args().nth(2).unwrap_or_default();

        if arg_type == "register" {
            let arg_name = std::env::args().nth(3).expect("name parameter is required");
            let arg_code = std::env::args().nth(4).expect("code parameter is required");

            let result = self.register(pool.clone(), &arg_name, &arg_code).await;
            info!("register result: {:?}", result);
            return;
        }

        let token = CancellationToken::new();

        let (tx_key, rx_key) = mpsc::channel::<KeyEvent>(100);
        let kc = keycounter::KeyCounter::new();

        let collector = collector::Collector::new(pool.clone());
        let token_collector = token.clone();
        let h_collector = tokio::spawn(async move {
            tokio::select! {
                _ = token_collector.cancelled() => {},
                resp = collector.collect(rx_key) => {
                    if let Err(e) = resp {
                        error!(error = %e, "can't collect stats");
                        std::process::exit(1);
                    }
                }
            }
        });

        let mut sync = sync::Sync::new(pool.clone(), self.signer.clone());
        let token_sync = token.clone();
        let h_sync = tokio::spawn(async move {
            tokio::select! {
                _ = token_sync.cancelled() => {},
                resp = sync.sync() => {
                    if let Err(e) = resp {
                        error!(error = %e, "can't sync stats");
                        std::process::exit(1);
                    }
                }
            }
        });

        let (tx_tray, rx_tray) = mpsc::channel::<bool>(100);

        let tray = MyTray::new(tx_tray);
        let handle = tray.spawn().await.unwrap();
        let token_ui = token.clone();

        // Register ctrl_c before spawning the monitor thread's secondary tokio
        // runtime to avoid signal handler conflicts between runtimes.
        let token_ctrlc = token.clone();
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    info!("Shutting down...");
                    token_ctrlc.cancel();
                    handle.shutdown().await;
                }
                Err(err) => {
                    eprintln!("Unable to listen for shutdown signal: {}", err);
                }
            }
        });

        let token_monitor = token.clone();
        let task_monitor = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            rt.block_on(async move {
                tokio::select! {
                    _ = token_monitor.cancelled() => {},
                    resp = kc.monitor(tx_key) => {
                        if let Err(e) = resp {
                            error!(error = %e, "can't count");
                            std::process::exit(1);
                        }
                    }
                }
            });
        });

        run_ui(rx_tray, pool, token_ui).expect("TODO: panic ui");
        info!("UI exited");

        // The monitor thread blocks on evdev I/O and may not wake up on token
        // cancellation. Since the UI has exited cleanly, exit the process now.
        if token.is_cancelled() {
            return;
        }

        let res_monitor = task_monitor.join();
        info!("Monitor exited");

        if let Err(e) = res_monitor {
            error!(error = ?e, "can't monitor");
        }

        let res_collector = tokio::join!(h_collector, h_sync);
        info!("Collector exited");

        if let Err(e) = res_collector.0 {
            error!(error = ?e, "can't collect");
        }
    }

    async fn register(&mut self, pool: SqlitePool, name: &str, code: &str) -> anyhow::Result<bool> {
        let mut register_request = RegisterRequest {
            code: code.to_string(),
            name: name.to_string(),
            public_key: self.signer.get_public_key()?,
            issued_at: 0,
            signature: "".to_string(),
        };

        register_request = self.signer.sign_register(register_request)?;

        let response = reqwest::Client::new()
            .post(format!("{}/auth/register", crate::client::SERVER_URL))
            .json(&register_request)
            .send()
            .await?
            .error_for_status()?
            .json::<RegisterResponse>()
            .await?;

        if response.origin_id > 0 {
            let result = sqlx::query("INSERT INTO metadata (key, value) VALUES (?, ?)")
                .bind("origin_id")
                .bind(response.origin_id)
                .execute(&pool)
                .await;

            if let Err(e) = result {
                tracing::warn!(error = ?e, "failed to insert metadata");
                return Err(anyhow::anyhow!("failed to insert metadata"));
            }

            return Ok(true);
        }

        Err(anyhow::anyhow!("invalid response"))
    }
}
