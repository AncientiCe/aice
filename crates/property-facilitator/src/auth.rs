//! Staff accounts, desk sessions, and the backend service token.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::FacilitatorError;

/// Minimum staff password length.
pub const MIN_PASSWORD_CHARS: usize = 12;
/// Desk sessions last one shift.
pub(crate) const SESSION_MILLIS: i64 = 12 * 60 * 60 * 1000;
pub(crate) const SESSION_COOKIE: &str = "aice_desk";

/// Desk role. Staff handle tickets; supervisors can also reopen them and read the audit log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Staff,
    Supervisor,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Staff => "staff",
            Self::Supervisor => "supervisor",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "staff" => Some(Self::Staff),
            "supervisor" => Some(Self::Supervisor),
            _ => None,
        }
    }
}

pub(crate) fn hash_password(password: &str) -> Result<String, FacilitatorError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| FacilitatorError::Auth(error.to_string()))
}

pub(crate) fn verify_password(password: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Spend the same argon2 time for an unknown username as for a known one.
pub(crate) fn burn_password_check(password: &str) {
    static DUMMY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    if let Some(hash) = DUMMY.get_or_init(|| hash_password("aice-dummy-password").ok()) {
        let _ = verify_password(password, hash);
    }
}

/// 256 random bits, hex encoded.
pub fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Tokens are stored and compared as SHA-256 digests, never in the clear.
pub fn token_digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// True when `presented` is the service token, compared by digest in constant time.
pub(crate) fn service_token_matches(expected: &str, presented: &str) -> bool {
    !expected.is_empty()
        && constant_time_eq(
            token_digest(expected).as_bytes(),
            token_digest(presented).as_bytes(),
        )
}

pub(crate) fn bearer_token(header: Option<&str>) -> Option<&str> {
    header
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn cookie_value<'a>(header: Option<&'a str>, name: &str) -> Option<&'a str> {
    header?.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name && !value.is_empty()).then_some(value)
    })
}

pub(crate) fn validate_new_user(username: &str, password: &str) -> Result<(), FacilitatorError> {
    let valid_name = !username.is_empty()
        && username.len() <= 64
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid_name {
        return Err(FacilitatorError::Auth(
            "username must be 1-64 letters, digits, '.', '_' or '-'".to_string(),
        ));
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(FacilitatorError::Auth(format!(
            "password must be at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        bearer_token, cookie_value, hash_password, service_token_matches, token_digest,
        validate_new_user, verify_password, Role,
    };

    #[test]
    fn password_hash_round_trips_and_rejects_wrong_password() {
        let hash = match hash_password("correct horse battery") {
            Ok(hash) => hash,
            Err(error) => panic!("hash: {error}"),
        };
        assert!(hash.starts_with("$argon2"));
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("wrong horse battery", &hash));
        assert!(!verify_password("anything", "not a hash"));
    }

    #[test]
    fn service_token_needs_an_exact_match() {
        assert!(service_token_matches("abc", "abc"));
        assert!(!service_token_matches("abc", "abd"));
        assert!(!service_token_matches("", ""));
        assert_ne!(token_digest("abc"), "abc");
    }

    #[test]
    fn headers_are_parsed_strictly() {
        assert_eq!(bearer_token(Some("Bearer xyz")), Some("xyz"));
        assert_eq!(bearer_token(Some("Basic xyz")), None);
        assert_eq!(bearer_token(Some("Bearer ")), None);
        assert_eq!(
            cookie_value(Some("a=1; aice_desk=tok; b=2"), "aice_desk"),
            Some("tok")
        );
        assert_eq!(cookie_value(Some("aice_desk="), "aice_desk"), None);
        assert_eq!(cookie_value(None, "aice_desk"), None);
    }

    #[test]
    fn usernames_and_roles_are_validated() {
        assert!(validate_new_user("maria.k", "twelve chars!").is_ok());
        assert!(validate_new_user("maria k", "twelve chars!").is_err());
        assert!(validate_new_user("maria", "short").is_err());
        assert_eq!(Role::parse("supervisor"), Some(Role::Supervisor));
        assert_eq!(Role::parse("admin"), None);
    }
}
