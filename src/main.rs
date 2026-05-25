use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

mod client;
mod server;
mod shared;

const MAX_CLOCK_SKEW_SECS: u64 = 60;

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
        .with(
            EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into())
                .add_directive(
                    "evdevil::evdev=error"
                        .parse()
                        .expect("skip warn when evdevil try to open input as rw"),
                ),
        )
        .init();

    let mode = std::env::args().nth(1).expect("");

    let signer = shared::signer::Signer::new("./key-dev.pem".as_ref());

    if mode == "client" {
        let mut client_runner = client::client::Client::new(signer);
        client_runner.run_client().await;
        return;
    }

    if mode == "server" {
        let server_runner = server::server::Server::new(signer);
        server_runner
            .run_server()
            .await
            .expect("TODO: panic message");
        return;
    }
}
