use property_facilitator::{
    assign_device, firmware_keygen, publish_firmware, serve, set_firmware_rollout,
    verify_firmware_signature,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::common::{client, hotels, temp_db};

async fn pod_token(base: &str, db: &std::path::Path, device_id: &str) -> String {
    let enroll = || async {
        match client()
            .post(format!("{base}/api/devices/enroll"))
            .json(&json!({ "device_id": device_id, "nonce": format!("{device_id}-nonce-0123456789"), "firmware": "1.0.0" }))
            .send()
            .await
        {
            Ok(response) => response.json::<Value>().await.unwrap_or(Value::Null),
            Err(error) => panic!("enroll: {error}"),
        }
    };
    let _ = enroll().await;
    if let Err(error) = assign_device(db, device_id, "1") {
        panic!("assign: {error}");
    }
    enroll().await["token"].as_str().unwrap_or("").to_string()
}

async fn manifest(base: &str, token: &str, current: &str) -> (StatusCode, Value) {
    match client()
        .get(format!("{base}/api/firmware/manifest?current={current}"))
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            (status, response.json().await.unwrap_or(Value::Null))
        }
        Err(error) => panic!("manifest: {error}"),
    }
}

fn firmware_dir(label: &str) -> std::path::PathBuf {
    let dir = temp_db(label).with_extension("fw");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        panic!("dir: {error}");
    }
    dir
}

#[tokio::test]
async fn a_signed_release_is_offered_downloaded_and_verifiable() {
    let db = temp_db("fw-offer");
    let dir = firmware_dir("fw-offer");
    let key = dir.join("firmware_signing.pk8");
    let public_key = match firmware_keygen(&key) {
        Ok(public_key) => public_key,
        Err(error) => panic!("keygen: {error}"),
    };
    let image = dir.join("pod.bin");
    let bytes: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
    if let Err(error) = std::fs::write(&image, &bytes) {
        panic!("write image: {error}");
    }
    let release = match publish_firmware(&db, &key, &image, "1.1.0", 100) {
        Ok(release) => release,
        Err(error) => panic!("publish: {error}"),
    };
    assert_eq!(release.size, 70_000);

    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let token = pod_token(&running.url, &db, "pod-fw1").await;

    let (status, offer) = manifest(&running.url, &token, "1.0.0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(offer["version"], "1.1.0");
    assert_eq!(offer["size"], 70_000);
    let url = offer["url"].as_str().unwrap_or("").to_string();
    let downloaded = match client()
        .get(format!("{}{url}", running.url))
        .bearer_auth(&token)
        .send()
        .await
    {
        Ok(response) => {
            assert_eq!(response.status(), StatusCode::OK);
            response
                .bytes()
                .await
                .map(|b| b.to_vec())
                .unwrap_or_default()
        }
        Err(error) => panic!("download: {error}"),
    };
    assert_eq!(downloaded, bytes);
    let signature = offer["signature_hex"].as_str().unwrap_or("");
    assert!(verify_firmware_signature(
        &public_key,
        &downloaded,
        signature
    ));
    let mut tampered = downloaded.clone();
    tampered[10] ^= 0xFF;
    assert!(!verify_firmware_signature(
        &public_key,
        &tampered,
        signature
    ));

    let (status, _) = manifest(&running.url, &token, "1.1.0").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "already up to date");
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_staged_rollout_offers_the_update_to_part_of_the_fleet() {
    let db = temp_db("fw-stage");
    let dir = firmware_dir("fw-stage");
    let key = dir.join("firmware_signing.pk8");
    if let Err(error) = firmware_keygen(&key) {
        panic!("keygen: {error}");
    }
    let image = dir.join("pod.bin");
    if let Err(error) = std::fs::write(&image, vec![1u8; 4096]) {
        panic!("write image: {error}");
    }
    if let Err(error) = publish_firmware(&db, &key, &image, "2.0.0", 0) {
        panic!("publish: {error}");
    }
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let mut tokens = Vec::new();
    for index in 0..20 {
        tokens.push(pod_token(&running.url, &db, &format!("pod-stage-{index}")).await);
    }
    let mut offered = 0;
    for token in &tokens {
        if manifest(&running.url, token, "1.0.0").await.0 == StatusCode::OK {
            offered += 1;
        }
    }
    assert_eq!(offered, 0, "0% rollout offers nobody");

    if let Err(error) = set_firmware_rollout(&db, "2.0.0", 50) {
        panic!("rollout: {error}");
    }
    let mut offered = 0;
    for token in &tokens {
        if manifest(&running.url, token, "1.0.0").await.0 == StatusCode::OK {
            offered += 1;
        }
    }
    assert!(
        (3..=17).contains(&offered),
        "about half the fleet: {offered}"
    );

    if let Err(error) = set_firmware_rollout(&db, "2.0.0", 100) {
        panic!("rollout: {error}");
    }
    for token in &tokens {
        assert_eq!(
            manifest(&running.url, token, "1.0.0").await.0,
            StatusCode::OK
        );
    }
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn firmware_needs_a_device_token() {
    let db = temp_db("fw-auth");
    let running = match serve(hotels(db.clone(), None, Vec::new())).await {
        Ok(running) => running,
        Err(error) => panic!("serve failed: {error}"),
    };
    let (status, _) = manifest(&running.url, "not-a-token", "1.0.0").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let download = match client()
        .get(format!("{}/api/firmware/1.0.0.bin", running.url))
        .send()
        .await
    {
        Ok(response) => response.status(),
        Err(error) => panic!("download: {error}"),
    };
    assert_eq!(download, StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_file(db);
}
