use crate::keycounter::KeyEvent;
use crate::tray::MyTray;
use crate::ui::run_ui;
use evdev::Device;
use futures_util::future::err;
use ksni::{Handle, TrayMethods};
use sqlx::SqlitePool;
use std::fs;
use std::ptr::null;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod collector;
mod keycounter;
mod tray;
mod ui;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(
            fmt::layer()
                .with_file(true)
                .with_line_number(true)
                .json()
                .with_current_span(false)
                .with_span_list(false),
        )
        .init();

    let pool = match SqlitePool::connect("sqlite://keylogger.db?mode=rwc").await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::error!(error = %e, "can't connect to db");
            std::process::exit(1);
        }
    };

    if let Err(e) = sqlx::migrate!("./migration").run(&pool).await {
        tracing::error!(error = %e, "can't run migrations");
        std::process::exit(1);
    }

    /*
    8.3mo, 5m
    8.4mo, 10m
    8.1mo, 15m
    */

    let token = CancellationToken::new();

    let (tx, rx) = mpsc::channel::<KeyEvent>(100);
    let mut kc = keycounter::KeyCounter::new();

    let collector = collector::Collector::new(pool.clone());
    let token_collector = token.clone();
    let h_collector = tokio::spawn(async move {
        tokio::select! {
            _ = token_collector.cancelled() => {
                return;
            },
            resp = collector.collect(rx) => {
                if let Err(e) = resp {
                    tracing::error!(error = %e, "can't collect stats");
                    std::process::exit(1);
                }
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
                _ = token_monitor.cancelled() => {
                    return;
                },
                resp = kc.monitor(tx) => {
                    if let Err(e) = resp {
                        tracing::error!(error = %e, "can't count");
                        std::process::exit(1);
                    }
                }
            }
        });
    });

    let (tx, rx) = mpsc::channel::<bool>(100);

    let tray = MyTray::new(tx);
    let handle = tray.spawn().await.unwrap();
    let token_ui = token.clone();

    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                handle.shutdown().await;
                token.cancel();
                tracing::info!("Shutting down...");
                slint::invoke_from_event_loop(|| {
                    slint::quit_event_loop().unwrap();
                })
                .unwrap();
            }
            Err(err) => {
                eprintln!("Unable to listen for shutdown signal: {}", err);
                // we also shut down in case of error
            }
        }
    });

    run_ui(rx, pool, token_ui);
    tracing::info!("UI exited");

    let res_monitor = task_monitor.join();
    tracing::info!("Monitor exited");

    if let Err(e) = res_monitor {
        tracing::error!(error = ?e, "can't monitor");
    }

    let res_collector = tokio::join!(h_collector);
    tracing::info!("Collector exited");

    if let Err(e) = res_collector.0 {
        tracing::error!(error = ?e, "can't collect");
    }
}
