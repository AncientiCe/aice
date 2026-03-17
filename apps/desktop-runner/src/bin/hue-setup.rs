use core_config::Config;
use core_skills::HueSmartHomeSkill;
use std::path::Path;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut config = Config::load(Path::new("config.json"))?;

    let bridge_host = match config.smart_home.hue.bridge_host.clone() {
        Some(host) if !host.is_empty() => host,
        _ => {
            let discovered = HueSmartHomeSkill::discover_bridge().await?;
            println!("Discovered Hue bridge at {discovered}");
            discovered
        }
    };

    let pre_wait_secs = std::env::var("AICE_HUE_SETUP_PREWAIT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    println!(
        "Press the physical Hue bridge link button now. Starting attempts in {pre_wait_secs}s..."
    );
    tokio::time::sleep(Duration::from_secs(pre_wait_secs)).await;
    println!("Starting Hue app-key link attempts (up to ~30s)...");

    let mut app_key = None;
    let mut last_err: Option<String> = None;
    for attempt in 1..=15 {
        match HueSmartHomeSkill::create_app_key(&bridge_host, "aice").await {
            Ok(k) => {
                app_key = Some(k);
                println!("Hue link succeeded on attempt {attempt}.");
                break;
            }
            Err(e) => {
                let msg = e.to_string();
                println!("Attempt {attempt}/15 failed: {msg}");
                last_err = Some(msg);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    let app_key = app_key.ok_or_else(|| {
        let msg = last_err.unwrap_or_else(|| "unknown Hue link error".to_string());
        format!("Failed to create Hue app key after retries: {msg}")
    })?;
    println!("Created Hue app key successfully.");

    config.smart_home.hue.enabled = true;
    config.smart_home.hue.bridge_host = Some(bridge_host);
    config.smart_home.hue.app_key = Some(app_key);

    let raw = serde_json::to_string_pretty(&config)?;
    std::fs::write("config.json", raw)?;

    println!("Updated config.json with smart_home.hue credentials.");
    Ok(())
}
