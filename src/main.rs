use crate::keycounter::KeyEvent;
use evdev::Device;
use sqlx::SqlitePool;
use std::fs;
use tokio::sync::mpsc;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod collector;
mod keycounter;

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

    let (tx, rx) = mpsc::channel::<KeyEvent>(100);
    let mut kc = keycounter::KeyCounter::new();

    let collector = collector::Collector::new(pool);
    let h_collector = tokio::spawn(async move {
        if let Err(e) = collector.collect(rx).await {
            tracing::error!(error = %e, "can't collect stats");
            std::process::exit(1);
        }
    });

    let res_monitor = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async move {
            if let Err(e) = kc.monitor(tx).await {
                tracing::error!(error = %e, "can't count");
                std::process::exit(1);
            }
        });
    })
    .join();

    if let Err(e) = res_monitor {
        tracing::error!(error = ?e, "can't monitor");
    }

    let res_collector = tokio::join!(h_collector);

    if let Err(e) = res_collector.0 {
        tracing::error!(error = ?e, "can't collect");
    }
}
