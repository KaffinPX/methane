use std::panic;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;
use xnt_rpc_client::http::HttpClient;

use crate::upgrader::flow::Upgrader;

mod upgrader;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("methane=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        default_hook(panic_info);
        std::process::exit(1);
    }));

    info!("Initializing methane, the proof upgrader...");

    let client = HttpClient::new("http://2.tcp.eu.ngrok.io:10856");
    let upgrader = Upgrader::new(client);

    upgrader.main_loop().await;

    Ok(())
}
