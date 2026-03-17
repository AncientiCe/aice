use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn cargo_aliases_include_canonical_local_jarvis_commands() {
    let config_path = workspace_root().join(".cargo").join("config.toml");
    let content = fs::read_to_string(config_path).expect("read .cargo/config.toml");
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
}

#[test]
fn repository_has_no_powershell_scripts() {
    let scripts_dir = workspace_root().join("scripts");
    if !scripts_dir.exists() {
        return;
    }
    let mut ps1_files = Vec::new();
    for entry in fs::read_dir(scripts_dir).expect("read scripts dir") {
        let path = entry.expect("dir entry").path();
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
}
