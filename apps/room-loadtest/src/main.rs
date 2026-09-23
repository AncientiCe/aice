//! room-loadtest: simulated pods against a staging property.
//!
//! ```text
//! room-loadtest --bridge wss://voice.property.local:8765/ --ca tls/ca.pem \
//!   --facilitator https://desk.property.local:8791 --db property.sqlite \
//!   --rooms 40 --turns 3 [--pcm utterance.raw] [--timeout-secs 30]
//! ```
//!
//! `--pcm` is raw PCM16 LE 16 kHz mono; without it a synthetic tone is sent,
//! which the real STT will not understand but still exercises every stage.
//! Pods are enrolled as `loadtest-NNNN` and assigned rooms `load-NNNN`; revoke
//! them afterwards with `device revoke`.

use std::path::PathBuf;
use std::time::Duration;

use room_loadtest::{provision_pods, run_load, speech_like_pcm, LoadSettings};

struct Args {
    bridge: String,
    ca: Option<PathBuf>,
    server_name: Option<String>,
    facilitator: String,
    db: PathBuf,
    rooms: usize,
    turns: usize,
    pcm: Option<PathBuf>,
    timeout: Duration,
}

fn parse() -> Result<Args, String> {
    let mut values = std::collections::HashMap::new();
    let mut raw = std::env::args().skip(1);
    while let Some(flag) = raw.next() {
        let value = raw.next().ok_or_else(|| format!("{flag} needs a value"))?;
        values.insert(flag, value);
    }
    let take = |name: &str| values.get(name).cloned();
    let number = |name: &str, default: usize| -> Result<usize, String> {
        take(name)
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| format!("{name} must be a number"))
            })
            .unwrap_or(Ok(default))
    };
    Ok(Args {
        bridge: take("--bridge").ok_or("--bridge is required")?,
        ca: take("--ca").map(PathBuf::from),
        server_name: take("--server-name"),
        facilitator: take("--facilitator").ok_or("--facilitator is required")?,
        db: take("--db").map(PathBuf::from).ok_or("--db is required")?,
        rooms: number("--rooms", 10)?,
        turns: number("--turns", 3)?,
        pcm: take("--pcm").map(PathBuf::from),
        timeout: Duration::from_secs(number("--timeout-secs", 30)? as u64),
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = parse()?;
    let utterance = match &args.pcm {
        Some(path) => std::fs::read(path)?,
        None => speech_like_pcm(Duration::from_millis(1500)),
    };
    println!("provisioning {} simulated pods", args.rooms);
    let tokens = provision_pods(
        args.facilitator.trim_end_matches('/'),
        args.ca.as_deref(),
        &args.db,
        "loadtest",
        args.rooms,
    )
    .await?;
    println!(
        "running {} turns in each of {} rooms",
        args.turns, args.rooms
    );
    let report = run_load(&LoadSettings {
        bridge_url: args.bridge,
        server_name: args.server_name,
        ca_file: args.ca,
        tokens,
        utterance,
        turns_per_room: args.turns,
        answer_timeout: args.timeout,
    })
    .await;
    println!("{report}");
    if report.failures > 0 {
        return Err(format!("{} turns got no answer", report.failures).into());
    }
    Ok(())
}
