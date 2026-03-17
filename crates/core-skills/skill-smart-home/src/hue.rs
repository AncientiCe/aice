use crate::types::{DeviceState, SmartHomeResult, SmartHomeSkill, SmartHomeSkillError};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

const HUE_DISCOVERY_URL: &str = "https://discovery.meethue.com/";

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
        let response = client
            .get(HUE_DISCOVERY_URL)
            .send()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SmartHomeSkillError::Device(format!(
                "discovery failed with status {}",
                response.status()
            )));
        }
        let items: Vec<HueDiscoveryItem> = response
            .json()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        let first = items
            .into_iter()
            .next()
            .ok_or_else(|| SmartHomeSkillError::Device("no Hue bridge discovered".to_string()))?;
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
        let response = client
            .post(format!("http://{bridge_host}/api"))
            .json(&json!({ "devicetype": format!("aice#{device_name}") }))
            .send()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SmartHomeSkillError::Device(format!(
                "app-key creation failed with status {}",
                response.status()
            )));
        }
        let body: Vec<LinkResponseItem> = response
            .json()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        for item in body {
            if let Some(success) = item.success {
                return Ok(success.username);
            }
            if item.error.is_some() {
                return Err(SmartHomeSkillError::Device(
                    "bridge rejected app-key creation; press bridge button and retry".to_string(),
                ));
            }
        }
        Err(SmartHomeSkillError::Device(
            "bridge returned no app-key".to_string(),
        ))
    }

    pub async fn ping_bridge(&self) -> Result<(), SmartHomeSkillError> {
        let response = self
            .client
            .get(format!("{}/resource/bridge", self.base_url))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SmartHomeSkillError::Device(format!(
                "bridge ping failed with status {}",
                response.status()
            )));
        }
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
        let response = self
            .client
            .get(format!("{}/resource/light", self.base_url))
            .header("hue-application-key", &self.app_key)
            .send()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SmartHomeSkillError::Device(format!(
                "light list failed with status {}",
                response.status()
            )));
        }
        let parsed: HueResourceResponse<HueLight> = response
            .json()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
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
        let response = self
            .client
            .put(format!("{}/resource/light/{}", self.base_url, light_id))
            .header("hue-application-key", &self.app_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| SmartHomeSkillError::Device(e.to_string()))?;
        if !response.status().is_success() {
            return Err(SmartHomeSkillError::Device(format!(
                "light command failed with status {}",
                response.status()
            )));
        }
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
        let final_light = self
            .resolve_light(&latest, target)
            .ok_or_else(|| SmartHomeSkillError::Device("target light disappeared".to_string()))?;

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
}
