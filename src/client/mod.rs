pub(crate) mod client;
pub(crate) mod collector;
mod deferred_drop;
pub(crate) mod keycounter;
mod sync;
pub(crate) mod tray;
pub(crate) mod ui;

pub(crate) const SERVER_URL: &str = "http://localhost:4444";
