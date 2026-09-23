//! Shared command line for `aice-hotels`, `aice-care`, and `aice-ward`.
//!
//! ```text
//! aice-<pack> [property.json]                        run the facilitator
//! aice-<pack> <property.json> user add <name> <role> create a desk account
//! aice-<pack> <property.json> user remove <name>     delete a desk account
//! aice-<pack> <property.json> user list              list desk accounts
//! aice-<pack> <property.json> device list            list room pods
//! aice-<pack> <property.json> device assign <id> <room>
//! aice-<pack> <property.json> device revoke <id>
//! aice-<pack> <property.json> stay list              list stays
//! aice-<pack> <property.json> stay open <room> [<continue-from-stay>]
//! aice-<pack> <property.json> stay close <stay>
//! aice-<pack> <property.json> tls fingerprint        print the CA pin for pods
//! aice-<pack> <property.json> tls issue <cert> <key> <host>...
//! ```
//!
//! `user add` reads the password from `AICE_PROPERTY_PASSWORD`, or prompts twice.

use std::io::Write;
use std::path::Path;

use crate::{
    add_user, assign_device, close_stay, list_devices, list_stays, list_users, open_stay,
    remove_user, revoke_device, serve, FacilitatorError, Pack, Role, Settings,
};

/// Env var `user add` reads the new password from (for scripted installs).
pub const PASSWORD_ENV: &str = "AICE_PROPERTY_PASSWORD";

pub async fn run_pack(pack: Pack) -> Result<(), FacilitatorError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "property.json".to_string());
    let settings = Settings::load_or_default(pack, Path::new(&config_path))?;
    match args.get(1).map(String::as_str) {
        None => run_server(settings).await,
        Some("user") => user_command(&settings, &args[2..]),
        Some("device") => device_command(&settings, &args[2..]),
        Some("tls") => tls_command(&settings, &args[2..]),
        Some("stay") => stay_command(&settings, &args[2..]),
        Some(other) => Err(usage(pack, &format!("unknown command '{other}'"))),
    }
}

async fn run_server(settings: Settings) -> Result<(), FacilitatorError> {
    core_observability::init_json_logging().ok();
    if let Some(bind) = settings.metrics_bind.clone() {
        if let Err(error) = core_observability::init_prometheus_exporter(&bind) {
            tracing::warn!(%error, %bind, "property metrics exporter failed to start");
        }
    }
    core_observability::register_metrics();
    let pack = settings.pack;
    let running = serve(settings).await?;
    println!("aice-{} listening on {}", pack.as_str(), running.url);
    let _ = std::io::stdout().flush();
    running.until_ctrl_c().await
}

fn user_command(settings: &Settings, args: &[String]) -> Result<(), FacilitatorError> {
    let pack = settings.pack;
    let db = settings.database_path.as_path();
    match args {
        [action, name, role] if action == "add" => {
            let role = Role::parse(role)
                .ok_or_else(|| usage(pack, "role must be 'staff' or 'supervisor'"))?;
            let password = read_new_password()?;
            add_user(db, name, &password, role)?;
            println!("added {} ({})", name.trim(), role.as_str());
            Ok(())
        }
        [action, name] if action == "remove" => {
            remove_user(db, name)?;
            println!("removed {}", name.trim());
            Ok(())
        }
        [action] if action == "list" => {
            for (name, role) in list_users(db)? {
                println!("{name}\t{}", role.as_str());
            }
            Ok(())
        }
        _ => Err(usage(pack, "unknown user command")),
    }
}

fn device_command(settings: &Settings, args: &[String]) -> Result<(), FacilitatorError> {
    let db = settings.database_path.as_path();
    match args {
        [action] if action == "list" => {
            for device in list_devices(db)? {
                println!(
                    "{}\t{}\t{}\t{}",
                    device.device_id,
                    device.status,
                    device.room.as_deref().unwrap_or("-"),
                    device.firmware
                );
            }
            Ok(())
        }
        [action, id, room] if action == "assign" => {
            assign_device(db, id, room)?;
            println!("{} -> room {}", id.trim(), room.trim());
            Ok(())
        }
        [action, id] if action == "revoke" => {
            revoke_device(db, id)?;
            println!("revoked {}", id.trim());
            Ok(())
        }
        _ => Err(usage(settings.pack, "unknown device command")),
    }
}

fn stay_command(settings: &Settings, args: &[String]) -> Result<(), FacilitatorError> {
    let db = settings.database_path.as_path();
    match args {
        [action] if action == "list" => {
            for stay in list_stays(db)? {
                let status = match (stay.closed_millis, stay.purged_millis) {
                    (None, _) => "open",
                    (Some(_), Some(_)) => "purged",
                    (Some(_), None) => "closed",
                };
                println!("{}\t{}\t{status}", stay.id, stay.room);
            }
            Ok(())
        }
        [action, room] if action == "open" => {
            let stay = open_stay(db, room, None)?;
            println!("{}\t{}", stay.id, stay.room);
            Ok(())
        }
        [action, room, previous] if action == "open" => {
            let stay = open_stay(db, room, Some(previous.trim()))?;
            println!("{}\t{}\tcontinues {}", stay.id, stay.room, previous.trim());
            Ok(())
        }
        [action, id] if action == "close" => {
            let stay = close_stay(db, id, settings.memory_retention)?;
            println!("closed {} ({})", stay.id, settings.memory_retention.label());
            Ok(())
        }
        _ => Err(usage(settings.pack, "unknown stay command")),
    }
}

fn tls_command(settings: &Settings, args: &[String]) -> Result<(), FacilitatorError> {
    let ca = settings
        .tls
        .as_ref()
        .and_then(|files| files.ca.clone())
        .ok_or_else(|| FacilitatorError::Tls("no property CA: set tls.mode to auto".to_string()))?;
    match args {
        [action] if action == "fingerprint" => {
            println!("{}", core_tls::fingerprint_sha256(&ca)?);
            Ok(())
        }
        [action, cert, key, hosts @ ..] if action == "issue" && !hosts.is_empty() => {
            let files = core_tls::CaFiles {
                key_path: ca.with_file_name("ca.key"),
                cert_path: ca,
            };
            core_tls::issue_server_cert(&files, hosts, Path::new(cert), Path::new(key))?;
            println!("issued {cert} for {}", hosts.join(", "));
            Ok(())
        }
        _ => Err(usage(settings.pack, "unknown tls command")),
    }
}

fn read_new_password() -> Result<String, FacilitatorError> {
    if let Ok(value) = std::env::var(PASSWORD_ENV) {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    let first = rpassword::prompt_password("New password: ")?;
    let second = rpassword::prompt_password("Repeat password: ")?;
    if first != second {
        return Err(FacilitatorError::Auth("passwords do not match".to_string()));
    }
    Ok(first)
}

fn usage(pack: Pack, problem: &str) -> FacilitatorError {
    let name = pack.as_str();
    FacilitatorError::Auth(format!(
        "{problem}\nusage:\n  aice-{name} [property.json]\n  \
        aice-{name} <property.json> user add <name> <staff|supervisor>\n  \
        aice-{name} <property.json> user remove <name>\n  \
        aice-{name} <property.json> user list\n  \
        aice-{name} <property.json> device list\n  \
        aice-{name} <property.json> device assign <id> <room>\n  \
        aice-{name} <property.json> device revoke <id>\n  \
        aice-{name} <property.json> stay list\n  \
        aice-{name} <property.json> stay open <room> [<continue-from-stay>]\n  \
        aice-{name} <property.json> stay close <stay>\n  \
        aice-{name} <property.json> tls fingerprint\n  \
        aice-{name} <property.json> tls issue <cert> <key> <host>..."
    ))
}
