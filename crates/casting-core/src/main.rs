// CLI 入口
#[cfg(not(target_os = "android"))]
mod cli;

#[cfg(not(target_os = "android"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::run().await
}

// Android 入口
#[cfg(target_os = "android")]
fn main() {}
