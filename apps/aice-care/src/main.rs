use std::io::Write;
use std::path::Path;

use property_facilitator::{serve, Pack, Settings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "property.json".to_string());
    let settings = Settings::load_or_default(Pack::Care, Path::new(&config_path))?;
    let running = serve(settings).await?;
    println!("aice-care listening on {}", running.url);
    let _ = std::io::stdout().flush();
    running.until_ctrl_c().await?;
    Ok(())
}
