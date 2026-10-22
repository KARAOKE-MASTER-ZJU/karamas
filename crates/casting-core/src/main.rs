#[cfg(not(target_os = "android"))]
mod cli;

#[cfg(not(target_os = "android"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}

#[cfg(target_os = "android")]
fn main() {}
