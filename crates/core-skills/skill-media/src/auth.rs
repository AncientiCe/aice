use crate::types::MediaSkillError;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppleMusicAuthConfig {
    pub team_id: Option<String>,
    pub key_id: Option<String>,
    pub private_key_path: Option<PathBuf>,
    pub storefront: String,
}

#[derive(Clone)]
pub struct AppleMusicAuthManager {
    config: AppleMusicAuthConfig,
}

#[derive(Serialize)]
struct DevClaims {
    iss: String,
    iat: usize,
    exp: usize,
}

impl AppleMusicAuthManager {
    pub fn new(config: AppleMusicAuthConfig) -> Self {
        Self { config }
    }

    pub fn new_for_tests(config: AppleMusicAuthConfig) -> Self {
        Self::new(config)
    }

    pub fn config(&self) -> &AppleMusicAuthConfig {
        &self.config
    }

    pub fn developer_token(&self) -> Result<Option<String>, MediaSkillError> {
        let team_id = match self.config.team_id.as_deref() {
            Some(v) => v,
            None => return Ok(None),
        };
        let key_id = match self.config.key_id.as_deref() {
            Some(v) => v,
            None => return Ok(None),
        };
        let key_path = match self.config.private_key_path.as_ref() {
            Some(v) => v,
            None => return Ok(None),
        };

        let key_bytes =
            std::fs::read(key_path).map_err(|e| MediaSkillError::Auth(e.to_string()))?;
        let encoding_key = EncodingKey::from_ec_pem(&key_bytes)
            .map_err(|e| MediaSkillError::Auth(e.to_string()))?;
        let now = Utc::now().timestamp() as usize;
        let claims = DevClaims {
            iss: team_id.to_string(),
            iat: now,
            exp: (Utc::now() + Duration::days(30)).timestamp() as usize,
        };
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(key_id.to_string());
        let token = jsonwebtoken::encode(&header, &claims, &encoding_key)
            .map_err(|e| MediaSkillError::Auth(e.to_string()))?;
        Ok(Some(token))
    }
}
