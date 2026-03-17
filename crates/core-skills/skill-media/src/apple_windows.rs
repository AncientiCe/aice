use crate::auth::{AppleMusicAuthConfig, AppleMusicAuthManager};
use crate::musickit_bridge::{BridgeCommand, MusicKitBridge, PlayerStatus};
use crate::types::{MediaResult, MediaSkill, MediaSkillError};
use async_trait::async_trait;
use serde::Deserialize;
use std::process::Command;

const ITUNES_SEARCH_URL: &str = "https://itunes.apple.com/search";
const APPLE_CATALOG_SEARCH_URL: &str = "https://api.music.apple.com/v1/catalog";

#[derive(Clone)]
pub struct AppleMusicWindowsSkill {
    client: reqwest::Client,
    dry_run: bool,
    auth: Option<AppleMusicAuthManager>,
    bridge: MusicKitBridge,
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchItem>,
}

#[derive(Clone, Deserialize)]
struct SearchItem {
    #[serde(default, rename = "trackName")]
    track_name: String,
    #[serde(default, rename = "artistName")]
    artist_name: String,
    #[serde(default, rename = "trackViewUrl")]
    track_view_url: String,
    #[serde(default, rename = "previewUrl")]
    preview_url: String,
    #[serde(default, rename = "trackId")]
    track_id: i64,
    #[serde(default)]
    song_id: String,
}

#[derive(Deserialize)]
struct AppleSearchResponse {
    #[serde(default)]
    results: AppleSearchResults,
}

#[derive(Default, Deserialize)]
struct AppleSearchResults {
    songs: Option<AppleSongs>,
    playlists: Option<ApplePlaylists>,
}

#[derive(Deserialize)]
struct AppleSongs {
    #[serde(default)]
    data: Vec<AppleSong>,
}

#[derive(Deserialize)]
struct AppleSong {
    id: String,
    attributes: AppleSongAttributes,
}

#[derive(Deserialize)]
struct AppleSongAttributes {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "artistName")]
    artist_name: String,
    #[serde(rename = "url")]
    url: String,
    #[serde(default, rename = "previews")]
    previews: Vec<AppleSongPreview>,
}

#[derive(Deserialize)]
struct AppleSongPreview {
    #[serde(rename = "url")]
    url: String,
}

#[derive(Deserialize)]
struct ApplePlaylists {
    #[serde(default)]
    data: Vec<ApplePlaylist>,
}

#[derive(Deserialize)]
struct ApplePlaylist {
    id: String,
    attributes: ApplePlaylistAttributes,
}

#[derive(Deserialize)]
struct ApplePlaylistAttributes {
    #[serde(rename = "name")]
    name: String,
    #[serde(default, rename = "curatorName")]
    curator_name: String,
}

enum PlaySelection {
    Song {
        song_id: String,
        label: String,
        is_library: bool,
    },
    Playlist {
        playlist_id: String,
        label: String,
        is_library: bool,
    },
}

impl AppleMusicWindowsSkill {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            dry_run: false,
            auth: None,
            bridge: MusicKitBridge::new(),
        }
    }

    pub fn with_auth(config: AppleMusicAuthConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            dry_run: false,
            auth: Some(AppleMusicAuthManager::new(config)),
            bridge: MusicKitBridge::new(),
        }
    }

    pub fn new_for_tests() -> Self {
        Self {
            client: reqwest::Client::new(),
            dry_run: true,
            auth: None,
            bridge: MusicKitBridge::new(),
        }
    }

    fn normalize_action(action: Option<&str>) -> String {
        action
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "status".to_string())
    }

    fn should_prefer_playlist(query: &str) -> bool {
        query
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .any(|token| {
                token.eq_ignore_ascii_case("playlist")
                    || token.eq_ignore_ascii_case("playlists")
                    || token.eq_ignore_ascii_case("favorites")
                    || token.eq_ignore_ascii_case("favourites")
                    || token.eq_ignore_ascii_case("favorite")
                    || token.eq_ignore_ascii_case("favourite")
                    || token.eq_ignore_ascii_case("mix")
                    || token.eq_ignore_ascii_case("mixes")
            })
    }

    fn canonical_playlist_query(query: &str) -> String {
        Self::canonical_query(query, &["play", "playlist", "playlists", "my", "the"])
    }

    fn canonical_song_query(query: &str) -> String {
        Self::canonical_query(query, &["play", "song", "track", "music", "my", "the"])
    }

    fn canonical_query(query: &str, drop_words: &[&str]) -> String {
        let mut kept = Vec::new();
        for token in query
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            if drop_words.iter().any(|w| token.eq_ignore_ascii_case(w)) {
                continue;
            }
            kept.push(token.to_ascii_lowercase());
        }
        if kept.is_empty() {
            query.trim().to_string()
        } else {
            kept.join(" ")
        }
    }

    fn playback_url(track: &SearchItem) -> &str {
        if !track.preview_url.is_empty() {
            &track.preview_url
        } else {
            &track.track_view_url
        }
    }

    async fn search_track(&self, query: &str) -> Result<SearchItem, MediaSkillError> {
        if self.auth.is_some() {
            let found = self
                .search_track_apple_catalog(query)
                .await?
                .ok_or_else(|| {
                    MediaSkillError::Playback("apple catalog returned no match".to_string())
                })?;
            return Ok(found);
        }

        let response = self
            .client
            .get(ITUNES_SEARCH_URL)
            .query(&[("term", query), ("entity", "song"), ("limit", "1")])
            .send()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !response.status().is_success() {
            return Err(MediaSkillError::Playback(format!(
                "catalog search failed with status {}",
                response.status()
            )));
        }
        let body: SearchResponse = response
            .json()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        body.results
            .into_iter()
            .next()
            .ok_or(MediaSkillError::NoSource)
    }

    async fn search_playlist_apple_catalog(
        &self,
        query: &str,
    ) -> Result<Option<(String, String)>, MediaSkillError> {
        let Some(auth) = &self.auth else {
            return Ok(None);
        };
        let Some(dev_token) = auth.developer_token()? else {
            return Ok(None);
        };

        let storefront = auth.config().storefront.as_str();
        let req = self
            .client
            .get(format!(
                "{}/{}/search",
                APPLE_CATALOG_SEARCH_URL, storefront
            ))
            .query(&[("term", query), ("types", "playlists"), ("limit", "1")])
            .bearer_auth(dev_token);

        let response = req
            .send()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !response.status().is_success() {
            return Err(MediaSkillError::Playback(format!(
                "apple playlist search failed with status {}",
                response.status()
            )));
        }
        let body: AppleSearchResponse = response
            .json()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        Ok(Self::select_best_playlist(
            body.results.playlists.map(|p| p.data).unwrap_or_default(),
            query,
            true,
        ))
    }

    async fn resolve_play_selection(&self, query: &str) -> Result<PlaySelection, MediaSkillError> {
        if self.auth.is_some() && Self::should_prefer_playlist(query) {
            let playlist_query = Self::canonical_playlist_query(query);
            if let Some((playlist_id, label)) = self
                .bridge
                .search_library_playlist(&playlist_query, std::time::Duration::from_secs(3))?
            {
                return Ok(PlaySelection::Playlist {
                    playlist_id,
                    label,
                    is_library: true,
                });
            }
            if let Some((playlist_id, label)) =
                self.search_playlist_apple_catalog(&playlist_query).await?
            {
                return Ok(PlaySelection::Playlist {
                    playlist_id,
                    label,
                    is_library: false,
                });
            }
        }
        if self.auth.is_some() {
            let song_query = Self::canonical_song_query(query);
            if let Some((song_id, label)) = self
                .bridge
                .search_library_song(&song_query, std::time::Duration::from_secs(3))?
            {
                return Ok(PlaySelection::Song {
                    song_id,
                    label,
                    is_library: true,
                });
            }
        }
        let song_query = Self::canonical_song_query(query);
        let track = self.search_track(&song_query).await?;
        let song_id = self.ensure_song_id(&track)?;
        let label = format!("{} - {}", track.track_name, track.artist_name);
        Ok(PlaySelection::Song {
            song_id,
            label,
            is_library: false,
        })
    }

    fn select_best_playlist(
        playlists: Vec<ApplePlaylist>,
        query: &str,
        include_curator: bool,
    ) -> Option<(String, String)> {
        let query_norm = Self::playlist_match_key(query);
        let mut best: Option<(u8, String, String)> = None;
        for pl in playlists {
            let name = pl.attributes.name.clone();
            let score = Self::playlist_match_score(&name, &query_norm);
            let label = if include_curator && !pl.attributes.curator_name.trim().is_empty() {
                format!("{} - {}", name, pl.attributes.curator_name)
            } else {
                name
            };
            match &best {
                Some((best_score, _, _)) if *best_score >= score => {}
                _ => best = Some((score, pl.id, label)),
            }
        }
        best.map(|(_, id, label)| (id, label))
    }

    fn playlist_match_score(name: &str, query_norm: &str) -> u8 {
        let name_norm = Self::playlist_match_key(name);
        if name_norm == query_norm {
            return 3;
        }
        if name_norm.contains(query_norm) || query_norm.contains(&name_norm) {
            return 2;
        }
        1
    }

    fn playlist_match_key(input: &str) -> String {
        input
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c.is_ascii_whitespace() {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    async fn search_track_apple_catalog(
        &self,
        query: &str,
    ) -> Result<Option<SearchItem>, MediaSkillError> {
        let Some(auth) = &self.auth else {
            return Ok(None);
        };
        let Some(dev_token) = auth.developer_token()? else {
            return Ok(None);
        };

        let storefront = auth.config().storefront.as_str();
        let req = self
            .client
            .get(format!(
                "{}/{}/search",
                APPLE_CATALOG_SEARCH_URL, storefront
            ))
            .query(&[("term", query), ("types", "songs"), ("limit", "5")])
            .bearer_auth(dev_token);

        let response = req
            .send()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !response.status().is_success() {
            return Err(MediaSkillError::Playback(format!(
                "apple catalog search failed with status {}",
                response.status()
            )));
        }
        let body: AppleSearchResponse = response
            .json()
            .await
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        let songs = body.results.songs.map(|s| s.data).unwrap_or_default();
        let Some(song) = Self::select_best_song(songs, query) else {
            return Ok(None);
        };
        Ok(Some(SearchItem {
            track_name: song.attributes.name,
            artist_name: song.attributes.artist_name,
            track_view_url: song.attributes.url,
            preview_url: song
                .attributes
                .previews
                .into_iter()
                .next()
                .map(|p| p.url)
                .unwrap_or_default(),
            track_id: 0,
            song_id: song.id,
        }))
    }

    fn select_best_song(songs: Vec<AppleSong>, query: &str) -> Option<AppleSong> {
        let query_norm = Self::playlist_match_key(query);
        let mut best: Option<(u8, AppleSong)> = None;
        for song in songs {
            let score = Self::playlist_match_score(&song.attributes.name, &query_norm);
            match &best {
                Some((best_score, _)) if *best_score >= score => {}
                _ => best = Some((score, song)),
            }
        }
        best.map(|(_, song)| song)
    }

    async fn ensure_musickit_ready(&self) -> Result<(), MediaSkillError> {
        let Some(auth) = &self.auth else {
            return Err(MediaSkillError::Auth(
                "Apple Music auth is required for MusicKit playback".to_string(),
            ));
        };
        let dev = auth.developer_token()?.ok_or_else(|| {
            MediaSkillError::Auth("missing Apple developer token configuration".to_string())
        })?;
        // MusicKit JS obtains/refreshes its own user token via authorize() in the player runtime.
        // The desktop OAuth access token is not a MusicKit user token.
        let _ = self.bridge.ensure_started(&dev, "")?;
        Ok(())
    }

    fn ensure_song_id(&self, track: &SearchItem) -> Result<String, MediaSkillError> {
        if !track.song_id.is_empty() {
            return Ok(track.song_id.clone());
        }
        if track.track_id > 0 {
            return Ok(track.track_id.to_string());
        }
        Err(MediaSkillError::Playback(
            "apple catalog did not return playable song id".to_string(),
        ))
    }

    pub fn shutdown(&self) -> Result<(), MediaSkillError> {
        self.bridge.shutdown()
    }

    fn ensure_bridge_play_started(
        status: Option<PlayerStatus>,
        expected_label: &str,
    ) -> Result<(), MediaSkillError> {
        let Some(status) = status else {
            return Err(MediaSkillError::Playback(
                "MusicKit player did not report playback state".to_string(),
            ));
        };
        if status.state.eq_ignore_ascii_case("playing") {
            return Ok(());
        }
        let detail = status
            .error
            .unwrap_or_else(|| "no error detail from MusicKit bridge".to_string());
        Err(MediaSkillError::Playback(format!(
            "MusicKit did not start playback for '{}': state={}, detail={}",
            expected_label, status.state, detail
        )))
    }

    fn open_url(&self, url: &str) -> Result<(), MediaSkillError> {
        if self.dry_run {
            return Ok(());
        }
        let status = Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !status.success() {
            return Err(MediaSkillError::Playback(
                "failed to launch Apple Music URL".to_string(),
            ));
        }
        Ok(())
    }

    fn send_media_key(&self, media_key: &str) -> Result<(), MediaSkillError> {
        if self.dry_run {
            return Ok(());
        }
        let vk = match media_key {
            "MEDIA_PLAY_PAUSE" => "0xB3",
            "MEDIA_NEXT_TRACK" => "0xB0",
            "MEDIA_PREV_TRACK" => "0xB1",
            "MEDIA_STOP" => "0xB2",
            other => {
                return Err(MediaSkillError::Playback(format!(
                    "unsupported media key {other}"
                )))
            }
        };
        let script = format!(
            "Add-Type -MemberDefinition @'\n[System.Runtime.InteropServices.DllImport(\"user32.dll\")]\npublic static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, System.UIntPtr dwExtraInfo);\n'@ -Name Native -Namespace Win32;\n[Win32.Native]::keybd_event({vk}, 0, 0, [UIntPtr]::Zero);\nStart-Sleep -Milliseconds 60;\n[Win32.Native]::keybd_event({vk}, 0, 2, [UIntPtr]::Zero);"
        );
        let status = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !status.success() {
            return Err(MediaSkillError::Playback(format!(
                "failed to send media key {media_key}"
            )));
        }
        Ok(())
    }
}

impl Default for AppleMusicWindowsSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MediaSkill for AppleMusicWindowsSkill {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<MediaResult, MediaSkillError> {
        let action = Self::normalize_action(action);

        match action.as_str() {
            "play" => {
                let query = target.ok_or(MediaSkillError::NoSource)?;
                if self.dry_run {
                    return Ok(MediaResult {
                        summary: format!("Play requested for {}", query),
                        now_playing: Some(query.to_string()),
                        state: "playing".to_string(),
                    });
                }
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    let storefront = self
                        .auth
                        .as_ref()
                        .map(|a| a.config().storefront.clone())
                        .unwrap_or_else(|| "us".to_string());
                    let selection = self.resolve_play_selection(query).await?;
                    let label = match selection {
                        PlaySelection::Song {
                            song_id,
                            label,
                            is_library,
                        } => {
                            self.bridge.enqueue(BridgeCommand::PlayBySongId {
                                song_id,
                                label: label.clone(),
                                storefront,
                                is_library,
                            });
                            label
                        }
                        PlaySelection::Playlist {
                            playlist_id,
                            label,
                            is_library,
                        } => {
                            self.bridge.enqueue(BridgeCommand::PlayByPlaylistId {
                                playlist_id,
                                label: label.clone(),
                                storefront,
                                is_library,
                            });
                            label
                        }
                    };
                    let status = self
                        .bridge
                        .wait_for_state("playing", std::time::Duration::from_secs(8));
                    Self::ensure_bridge_play_started(status, &label)?;
                    return Ok(MediaResult {
                        summary: format!("Now Playing - {}", label),
                        now_playing: Some(label),
                        state: "playing".to_string(),
                    });
                } else {
                    let track = self.search_track(query).await?;
                    let playback_url = Self::playback_url(&track).to_string();
                    self.open_url(&playback_url)?;
                    return Ok(MediaResult {
                        summary: format!(
                            "Now Playing - {} - {}",
                            track.track_name, track.artist_name
                        ),
                        now_playing: Some(format!("{} - {}", track.track_name, track.artist_name)),
                        state: "playing".to_string(),
                    });
                }
            }
            "pause" => {
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::Pause);
                } else {
                    self.send_media_key("MEDIA_PLAY_PAUSE")?;
                }
                Ok(MediaResult {
                    summary: "Pause toggled".to_string(),
                    now_playing: None,
                    state: "paused".to_string(),
                })
            }
            "stop" => {
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::Stop);
                } else {
                    self.send_media_key("MEDIA_STOP")?;
                }
                Ok(MediaResult {
                    summary: "Playback stopped".to_string(),
                    now_playing: None,
                    state: "stopped".to_string(),
                })
            }
            "resume" => {
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::Resume);
                } else {
                    self.send_media_key("MEDIA_PLAY_PAUSE")?;
                }
                Ok(MediaResult {
                    summary: "Playback resumed".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "next" => {
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::Next);
                } else {
                    self.send_media_key("MEDIA_NEXT_TRACK")?;
                }
                Ok(MediaResult {
                    summary: "Skipped to next track".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "previous" => {
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::Previous);
                } else {
                    self.send_media_key("MEDIA_PREV_TRACK")?;
                }
                Ok(MediaResult {
                    summary: "Went to previous track".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "shuffle_on" => {
                if self.dry_run {
                    return Ok(MediaResult {
                        summary: "Shuffle enabled".to_string(),
                        now_playing: None,
                        state: "playing".to_string(),
                    });
                }
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::ShuffleOn);
                } else {
                    return Err(MediaSkillError::UnsupportedAction(
                        "shuffle_on requires Apple Music linked auth".to_string(),
                    ));
                }
                Ok(MediaResult {
                    summary: "Shuffle enabled".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "shuffle_off" => {
                if self.dry_run {
                    return Ok(MediaResult {
                        summary: "Shuffle disabled".to_string(),
                        now_playing: None,
                        state: "playing".to_string(),
                    });
                }
                if self.auth.is_some() {
                    self.ensure_musickit_ready().await?;
                    self.bridge.enqueue(BridgeCommand::ShuffleOff);
                } else {
                    return Err(MediaSkillError::UnsupportedAction(
                        "shuffle_off requires Apple Music linked auth".to_string(),
                    ));
                }
                Ok(MediaResult {
                    summary: "Shuffle disabled".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "status" => {
                if let Some(auth) = &self.auth {
                    let status = self.bridge.latest_status();
                    let state = status
                        .as_ref()
                        .map(|s| s.state.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    return Ok(MediaResult {
                        summary: format!(
                            "Apple Music control ready (storefront: {})",
                            auth.config().storefront
                        ),
                        now_playing: status.and_then(|s| s.now_playing),
                        state,
                    });
                }
                Ok(MediaResult {
                    summary: "Apple Music control ready".to_string(),
                    now_playing: None,
                    state: "unknown".to_string(),
                })
            }
            other => Err(MediaSkillError::UnsupportedAction(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppleMusicAuthConfig, AppleMusicWindowsSkill, ApplePlaylist, ApplePlaylistAttributes,
        AppleSong, AppleSongAttributes, PlayerStatus, SearchItem,
    };
    use crate::types::MediaSkill;

    #[test]
    fn playback_url_prefers_preview_when_available() {
        let item = SearchItem {
            track_name: "Song".to_string(),
            artist_name: "Artist".to_string(),
            track_view_url: "https://music.apple.com/track".to_string(),
            preview_url: "https://audio.example/preview.m4a".to_string(),
            track_id: 1,
            song_id: "1".to_string(),
        };
        assert_eq!(
            AppleMusicWindowsSkill::playback_url(&item),
            "https://audio.example/preview.m4a"
        );
    }

    #[test]
    fn playback_url_falls_back_to_track_view() {
        let item = SearchItem {
            track_name: "Song".to_string(),
            artist_name: "Artist".to_string(),
            track_view_url: "https://music.apple.com/track".to_string(),
            preview_url: String::new(),
            track_id: 1,
            song_id: "1".to_string(),
        };
        assert_eq!(
            AppleMusicWindowsSkill::playback_url(&item),
            "https://music.apple.com/track"
        );
    }

    #[test]
    fn bridge_play_started_accepts_playing_state() {
        let status = PlayerStatus {
            state: "playing".to_string(),
            now_playing: Some("Song - Artist".to_string()),
            error: None,
            updated_at_unix_ms: 1,
        };
        assert!(
            AppleMusicWindowsSkill::ensure_bridge_play_started(Some(status), "Song - Artist")
                .is_ok()
        );
    }

    #[test]
    fn bridge_play_started_rejects_non_playing_state() {
        let status = PlayerStatus {
            state: "error".to_string(),
            now_playing: None,
            error: Some("autoplay denied".to_string()),
            updated_at_unix_ms: 1,
        };
        let err = AppleMusicWindowsSkill::ensure_bridge_play_started(Some(status), "Song - Artist")
            .expect_err("must fail");
        assert!(format!("{err}").contains("autoplay denied"));
    }

    #[test]
    fn playlist_intent_detects_playlist_keywords() {
        assert!(AppleMusicWindowsSkill::should_prefer_playlist(
            "favorites playlist"
        ));
        assert!(AppleMusicWindowsSkill::should_prefer_playlist(
            "my favourites"
        ));
        assert!(AppleMusicWindowsSkill::should_prefer_playlist(
            "play favorite"
        ));
        assert!(AppleMusicWindowsSkill::should_prefer_playlist(
            "play favourite."
        ));
        assert!(AppleMusicWindowsSkill::should_prefer_playlist("daily mix"));
        assert!(!AppleMusicWindowsSkill::should_prefer_playlist(
            "blinding lights"
        ));
    }

    #[test]
    fn select_best_playlist_prefers_exact_match() {
        let playlists = vec![
            ApplePlaylist {
                id: "1".to_string(),
                attributes: ApplePlaylistAttributes {
                    name: "Family Favorites".to_string(),
                    curator_name: "Apple Music".to_string(),
                },
            },
            ApplePlaylist {
                id: "2".to_string(),
                attributes: ApplePlaylistAttributes {
                    name: "Favorites".to_string(),
                    curator_name: String::new(),
                },
            },
        ];
        let selected = AppleMusicWindowsSkill::select_best_playlist(playlists, "favorites", true)
            .expect("playlist selected");
        assert_eq!(selected.0, "2");
        assert_eq!(selected.1, "Favorites");
    }

    #[test]
    fn select_best_song_prefers_exact_match() {
        let songs = vec![
            AppleSong {
                id: "x1".to_string(),
                attributes: AppleSongAttributes {
                    name: "Blinding".to_string(),
                    artist_name: "Artist".to_string(),
                    url: "https://music.apple.com/song/x1".to_string(),
                    previews: Vec::new(),
                },
            },
            AppleSong {
                id: "x2".to_string(),
                attributes: AppleSongAttributes {
                    name: "Blinding Lights".to_string(),
                    artist_name: "The Weeknd".to_string(),
                    url: "https://music.apple.com/song/x2".to_string(),
                    previews: Vec::new(),
                },
            },
        ];
        let selected = AppleMusicWindowsSkill::select_best_song(songs, "blinding lights")
            .expect("song selected");
        assert_eq!(selected.id, "x2");
    }

    #[test]
    fn canonical_playlist_query_strips_intent_words() {
        assert_eq!(
            AppleMusicWindowsSkill::canonical_playlist_query("play my favorites playlist"),
            "favorites"
        );
    }

    #[tokio::test]
    async fn status_with_auth_does_not_require_oauth_fields() {
        let skill = AppleMusicWindowsSkill::with_auth(AppleMusicAuthConfig {
            team_id: None,
            key_id: None,
            private_key_path: None,
            storefront: "us".to_string(),
        });
        let result = skill.execute(Some("status"), None).await.expect("status");
        assert!(result.summary.contains("storefront: us"));
    }
}
