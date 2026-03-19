use std::fs;
use std::io;
use std::path::PathBuf;

fn workspace_root() -> Result<PathBuf, io::Error> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| io::Error::other("workspace root not found"))
}

#[test]
fn cargo_aliases_include_canonical_local_jarvis_commands() -> Result<(), Box<dyn std::error::Error>>
{
    let config_path = workspace_root()?.join(".cargo").join("config.toml");
    let content = fs::read_to_string(config_path)?;
    for alias in [
        "aice-pod-voice",
        "aice-desktop",
        "aice-gateway",
        "aice-fmt",
        "aice-clippy",
        "aice-audit",
        "aice-test",
    ] {
        assert!(
            content.contains(&format!("{alias} = ")),
            "missing cargo alias: {alias}"
        );
    }
    Ok(())
}

#[test]
fn repository_has_no_powershell_scripts() -> Result<(), Box<dyn std::error::Error>> {
    let scripts_dir = workspace_root()?.join("scripts");
    if !scripts_dir.exists() {
        return Ok(());
    }
    let mut ps1_files = Vec::new();
    for entry in fs::read_dir(scripts_dir)? {
        let path = entry?.path();
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("ps1"))
            .unwrap_or(false)
        {
            ps1_files.push(path);
        }
    }
    assert!(
        ps1_files.is_empty(),
        "PowerShell scripts should be removed: {:?}",
        ps1_files
    );
    Ok(())
}
