use std::collections::HashMap;

use crate::FacilitatorError;

/// Resolve the place a request belongs to.
///
/// A mapped phone extension wins. A pod or button room is used when no
/// extension was supplied. An unknown extension does not guess a room.
pub fn resolve_place(
    extensions: &HashMap<String, String>,
    room: Option<&str>,
    extension: Option<&str>,
) -> Result<String, FacilitatorError> {
    if let Some(extension) = extension.map(str::trim).filter(|value| !value.is_empty()) {
        return match extensions.get(extension) {
            Some(mapped) if !mapped.trim().is_empty() => Ok(mapped.trim().to_string()),
            Some(_) | None => Err(FacilitatorError::UnknownExtension(extension.to_string())),
        };
    }
    match room.map(str::trim).filter(|value| !value.is_empty()) {
        Some(room) => Ok(room.to_string()),
        None => Err(FacilitatorError::MissingPlace),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_place;
    use std::collections::HashMap;

    #[test]
    fn extension_maps_to_room_ahead_of_pod_room() {
        let mut extensions = HashMap::new();
        extensions.insert("101".to_string(), "204".to_string());
        let room = match resolve_place(&extensions, Some("lobby"), Some("101")) {
            Ok(room) => room,
            Err(error) => panic!("expected mapped room: {error}"),
        };
        assert_eq!(room, "204");
    }

    #[test]
    fn pod_room_is_used_when_there_is_no_extension() {
        let room = match resolve_place(&HashMap::new(), Some("12"), None) {
            Ok(room) => room,
            Err(error) => panic!("expected pod room: {error}"),
        };
        assert_eq!(room, "12");
    }
}
