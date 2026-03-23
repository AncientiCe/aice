use crate::types::{DeviceState, SmartHomeResult, SmartHomeSkill, SmartHomeSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use serde::Deserialize;
use serde_json::json;
use std::time::Instant;

const HUE_DISCOVERY_URL: &str = "https://discovery.meethue.com/";
const BACKEND_SKILL_EXECUTE_TOTAL: &str = "backend_skill_execute_total";
const BACKEND_SKILL_EXECUTE_DURATION_SECONDS: &str = "backend_skill_execute_duration_seconds";
const BACKEND_DEPENDENCY_REQUESTS_TOTAL: &str = "backend_dependency_requests_total";
const BACKEND_DEPENDENCY_REQUEST_DURATION_SECONDS: &str =
    "backend_dependency_request_duration_seconds";

#[derive(Clone)]
pub struct HueSmartHomeSkill {
    client: reqwest::Client,
    base_url: String,
    app_key: String,
    default_light_name: String,
}

#[derive(Deserialize)]
struct HueDiscoveryItem {
    internalipaddress: String,
}

#[derive(Deserialize)]
struct HueResourceResponse<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct HueLight {
    id: String,
    metadata: HueMetadata,
    on: Option<HueOn>,
    dimming: Option<HueDimming>,
    color_temperature: Option<HueColorTemperature>,
}

#[derive(Deserialize)]
struct HueMetadata {
    name: String,
}

#[derive(Deserialize)]
struct HueOn {
    on: bool,
}

#[derive(Deserialize)]
struct HueDimming {
    brightness: f64,
}

#[derive(Deserialize)]
struct HueColorTemperature {
    mirek: Option<u16>,
}

impl HueSmartHomeSkill {
    fn record_backend_skill_result(result: &str, error_kind: Option<&str>) {
        counter!(
            BACKEND_SKILL_EXECUTE_TOTAL,
            1,
            "skill" => "smart_home",
            "result" => result.to_string(),
            "error_kind" => error_kind.unwrap_or("none").to_string()
        );
    }

    fn record_backend_skill_duration(started_at: Instant) {
        histogram!(
            BACKEND_SKILL_EXECUTE_DURATION_SECONDS,
            started_at.elapsed().as_secs_f64(),
            "skill" => "smart_home"
        );
    }

    fn record_dependency_result(operation: &str, result: &str, error_kind: Option<&str>) {
        counter!(
            BACKEND_DEPENDENCY_REQUESTS_TOTAL,
            1,
            "dependency" => "hue",
            "operation" => operation.to_string(),
            "result" => result.to_string(),
            "error_kind" => error_kind.unwrap_or("none").to_string()
        );
    }

    fn record_dependency_duration(operation: &str, started_at: Instant) {
        histogram!(
            BACKEND_DEPENDENCY_REQUEST_DURATION_SECONDS,
            started_at.elapsed().as_secs_f64(),
            "dependency" => "hue",
            "operation" => operation.to_string()
        );
    }

    fn error_kind(error: &SmartHomeSkillError) -> &'static str {
        match error {
            SmartHomeSkillError::Device(_) => "device",
            SmartHomeSkillError::UnsupportedAction(_) => "unsupported_action",
            SmartHomeSkillError::Timeout => "timeout",
        }
    }

    pub fn new(bridge_host: &str, app_key: &str, default_light_name: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: format!("https://{bridge_host}/clip/v2"),
            app_key: app_key.to_string(),
            default_light_name: default_light_name.to_string(),
        }
    }

    pub fn new_for_tests(base_url: &str, app_key: &str, default_light_name: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            app_key: app_key.to_string(),
            default_light_name: default_light_name.to_string(),
        }
    }

    pub async fn discover_bridge() -> Result<String, SmartHomeSkillError> {
        let client = reqwest::Client::new();
        let started_at = Instant::now();
        let response = client.get(HUE_DISCOVERY_URL).send().await.map_err(|e| {
            Self::record_dependency_result("discover_bridge", "error", Some("request"));
            Self::record_dependency_duration("discover_bridge", started_at);
            SmartHomeSkillError::Device(e.to_string())
        })?;
        if !response.status().is_success() {
            Self::record_dependency_result("discover_bridge", "error", Some("http_status"));
            Self::record_dependency_duration("discover_bridge", started_at);
            return Err(SmartHomeSkillError::Device(format!(
                "discovery failed with status {}",
                response.status()
            )));
        }
        let items: Vec<HueDiscoveryItem> = response.json().await.map_err(|e| {
            Self::record_dependency_result("discover_bridge", "error", Some("parse"));
            Self::record_dependency_duration("discover_bridge", started_at);
            SmartHomeSkillError::Device(e.to_string())
        })?;
        let first = items.into_iter().next().ok_or_else(|| {
            Self::record_dependency_result("discover_bridge", "error", Some("no_results"));
            Self::record_dependency_duration("discover_bridge", started_at);
            SmartHomeSkillError::Device("no Hue bridge discovered".to_string())
        })?;
        Self::record_dependency_result("discover_bridge", "success", None);
        Self::record_dependency_duration("discover_bridge", started_at);
        Ok(first.internalipaddress)
    }

    /// One-time provisioning call: press the physical bridge link button, then call this.
    pub async fn create_app_key(
        bridge_host: &str,
        device_name: &str,
    ) -> Result<String, SmartHomeSkillError> {
        #[derive(Deserialize)]
        struct LinkSuccess {
            username: String,
        }
        #[derive(Deserialize)]
        struct LinkResponseItem {
            success: Option<LinkSuccess>,
            error: Option<serde_json::Value>,
        }

        let client = reqwest::Client::new();
        let started_at = Instant::now();
        let response = client
            .post(format!("http://{bridge_host}/api"))
            .json(&json!({ "devicetype": format!("aice#{device_name}") }))
            .send()
            .await
            .map_err(|e| {
                Self::record_dependency_result("create_app_key", "error", Some("request"));
                Self::record_dependency_duration("create_app_key", started_at);
                SmartHomeSkillError::Device(e.to_string())
            })?;
        if !response.status().is_success() {
            Self::record_dependency_result("create_app_key", "error", Some("http_status"));
            Self::record_dependency_duration("create_app_key", started_at);
            return Err(SmartHomeSkillError::Device(format!(
                "app-key creation failed with status {}",
                response.status()
            )));
        }
        let body: Vec<LinkResponseItem> = response.json().await.map_err(|e| {
            Self::record_dependency_result("create_app_key", "error", Some("parse"));
            Self::record_dependency_duration("create_app_key", started_at);
            SmartHomeSkillError::Device(e.to_string())
        })?;
        for item in body {
            if let Some(success) = item.success {
                Self::record_dependency_result("create_app_key", "success", None);
                Self::record_dependency_duration("create_app_key", started_at);
                return Ok(success.username);
            }
            if item.error.is_some() {
                Self::record_dependency_result("create_app_key", "error", Some("bridge_rejected"));
                Self::record_dependency_duration("create_app_key", started_at);
                return Err(SmartHomeSkillError::Device(
                    "bridge rejected app-key creation; press bridge button and retry".to_string(),
                ));
            }
        }
        Self::record_dependency_result("create_app_key", "error", Some("missing_key"));
        Self::record_dependency_duration("create_app_key", started_at);
        Err(SmartHomeSkillError::Device(
            "bridge returned no app-key".to_string(),
        ))
    }

    pub async fn ping_bridge(&self) -> Result<(), SmartHomeSkillError> {
        let started_at = Instant::now();
        let response = self
            .client
            .get(format!("{}/resource/bridge", self.base_url))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .map_err(|e| {
                Self::record_dependency_result("ping_bridge", "error", Some("request"));
                Self::record_dependency_duration("ping_bridge", started_at);
                SmartHomeSkillError::Device(e.to_string())
            })?;
        if !response.status().is_success() {
            Self::record_dependency_result("ping_bridge", "error", Some("http_status"));
            Self::record_dependency_duration("ping_bridge", started_at);
            return Err(SmartHomeSkillError::Device(format!(
                "bridge ping failed with status {}",
                response.status()
            )));
        }
        Self::record_dependency_result("ping_bridge", "success", None);
        Self::record_dependency_duration("ping_bridge", started_at);
        Ok(())
    }

    pub fn normalize_action(action: &str) -> Option<String> {
        let a = action.to_lowercase();
        if a.contains("status") || a.contains("state") {
            return Some("status".to_string());
        }
        if a.contains("on") {
            return Some("on".to_string());
        }
        if a.contains("off") {
            return Some("off".to_string());
        }
        if a.contains("bright") && a.contains("up") {
            return Some("brightness_up".to_string());
        }
        if a.contains("bright") && a.contains("down") {
            return Some("brightness_down".to_string());
        }
        if a.contains("warm") {
            return Some("warm".to_string());
        }
        if a.contains("cool") {
            return Some("cool".to_string());
        }
        None
    }

    async fn list_lights(&self) -> Result<Vec<HueLight>, SmartHomeSkillError> {
        let started_at = Instant::now();
        let response = self
            .client
            .get(format!("{}/resource/light", self.base_url))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .map_err(|e| {
                Self::record_dependency_result("list_lights", "error", Some("request"));
                Self::record_dependency_duration("list_lights", started_at);
                SmartHomeSkillError::Device(e.to_string())
            })?;
        if !response.status().is_success() {
            Self::record_dependency_result("list_lights", "error", Some("http_status"));
            Self::record_dependency_duration("list_lights", started_at);
            return Err(SmartHomeSkillError::Device(format!(
                "light list failed with status {}",
                response.status()
            )));
        }
        let parsed: HueResourceResponse<HueLight> = response.json().await.map_err(|e| {
            Self::record_dependency_result("list_lights", "error", Some("parse"));
            Self::record_dependency_duration("list_lights", started_at);
            SmartHomeSkillError::Device(e.to_string())
        })?;
        Self::record_dependency_result("list_lights", "success", None);
        Self::record_dependency_duration("list_lights", started_at);
        Ok(parsed.data)
    }

    fn resolve_light<'a>(
        &self,
        lights: &'a [HueLight],
        target: Option<&str>,
    ) -> Option<&'a HueLight> {
        let target = target.unwrap_or(&self.default_light_name).to_lowercase();
        lights
            .iter()
            .find(|l| l.metadata.name.to_lowercase().contains(&target))
            .or_else(|| {
                lights.iter().find(|l| {
                    l.metadata.name.to_lowercase() == self.default_light_name.to_lowercase()
                })
            })
            .or_else(|| lights.first())
    }

    async fn send_light_command(
        &self,
        light_id: &str,
        body: serde_json::Value,
    ) -> Result<(), SmartHomeSkillError> {
        let started_at = Instant::now();
        let response = self
            .client
            .put(format!("{}/resource/light/{}", self.base_url, light_id))
            .header("hue-application-key", &self.app_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                Self::record_dependency_result("send_light_command", "error", Some("request"));
                Self::record_dependency_duration("send_light_command", started_at);
                SmartHomeSkillError::Device(e.to_string())
            })?;
        if !response.status().is_success() {
            Self::record_dependency_result("send_light_command", "error", Some("http_status"));
            Self::record_dependency_duration("send_light_command", started_at);
            return Err(SmartHomeSkillError::Device(format!(
                "light command failed with status {}",
                response.status()
            )));
        }
        Self::record_dependency_result("send_light_command", "success", None);
        Self::record_dependency_duration("send_light_command", started_at);
        Ok(())
    }

    fn light_state_summary(light: &HueLight) -> String {
        let power = if light.on.as_ref().map(|v| v.on).unwrap_or(false) {
            "on"
        } else {
            "off"
        };
        let brightness = light
            .dimming
            .as_ref()
            .map(|d| format!("{:.0}%", d.brightness))
            .unwrap_or_else(|| "n/a".to_string());
        let mirek = light
            .color_temperature
            .as_ref()
            .and_then(|c| c.mirek)
            .map(|m| m.to_string())
            .unwrap_or_else(|| "n/a".to_string());
        format!("power={power}, brightness={brightness}, mirek={mirek}")
    }
}

#[async_trait]
impl SmartHomeSkill for HueSmartHomeSkill {
    async fn execute(
        &self,
        target: Option<&str>,
        action: Option<&str>,
    ) -> Result<SmartHomeResult, SmartHomeSkillError> {
        let started_at = Instant::now();
        let result = async {
            let action = match action {
                Some(a) => Self::normalize_action(a)
                    .ok_or_else(|| SmartHomeSkillError::UnsupportedAction(a.to_string()))?,
                None => "status".to_string(),
            };

            self.ping_bridge().await?;
            let lights = self.list_lights().await?;

            let light = self
                .resolve_light(&lights, target)
                .ok_or_else(|| SmartHomeSkillError::Device("no Hue lights found".to_string()))?;

            match action.as_str() {
                "status" => {}
                "on" => {
                    self.send_light_command(&light.id, json!({"on": {"on": true}}))
                        .await?
                }
                "off" => {
                    self.send_light_command(&light.id, json!({"on": {"on": false}}))
                        .await?
                }
                "brightness_up" => {
                    let current = light.dimming.as_ref().map(|d| d.brightness).unwrap_or(30.0);
                    let next = (current + 20.0).clamp(1.0, 100.0);
                    self.send_light_command(&light.id, json!({"dimming": {"brightness": next}}))
                        .await?;
                }
                "brightness_down" => {
                    let current = light.dimming.as_ref().map(|d| d.brightness).unwrap_or(30.0);
                    let next = (current - 20.0).clamp(1.0, 100.0);
                    self.send_light_command(&light.id, json!({"dimming": {"brightness": next}}))
                        .await?;
                }
                "warm" => {
                    self.send_light_command(&light.id, json!({"color_temperature": {"mirek": 400}}))
                        .await?
                }
                "cool" => {
                    self.send_light_command(&light.id, json!({"color_temperature": {"mirek": 180}}))
                        .await?
                }
                _ => unreachable!("validated above"),
            }

            let latest = self.list_lights().await?;
            let final_light = self.resolve_light(&latest, target).ok_or_else(|| {
                SmartHomeSkillError::Device("target light disappeared".to_string())
            })?;

            Ok(SmartHomeResult {
                summary: format!(
                    "Hue action '{}' completed for {}",
                    action, final_light.metadata.name
                ),
                device_states: vec![DeviceState {
                    id: final_light.id.clone(),
                    name: final_light.metadata.name.clone(),
                    state: Self::light_state_summary(final_light),
                }],
            })
        }
        .await;
        match &result {
            Ok(_) => Self::record_backend_skill_result("success", None),
            Err(error) => Self::record_backend_skill_result("error", Some(Self::error_kind(error))),
        }
        Self::record_backend_skill_duration(started_at);
        result
    }
}
