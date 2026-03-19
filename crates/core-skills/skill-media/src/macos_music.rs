use crate::types::{MediaResult, MediaSkill, MediaSkillError};
use async_trait::async_trait;
use metrics::{counter, histogram};
use serde::Deserialize;
use std::process::Command;
use std::time::Instant;

const ITUNES_SEARCH_URL: &str = "https://itunes.apple.com/search";
const MEDIA_SKILL_EXECUTE_TOTAL: &str = "media_skill_execute_total";
const MEDIA_SKILL_ERRORS_TOTAL: &str = "media_skill_errors_total";
const MEDIA_SKILL_EXECUTE_DURATION_SECONDS: &str = "media_skill_execute_duration_seconds";

#[derive(Clone)]
pub struct MacOsMusicSkill {
    client: reqwest::Client,
    dry_run: bool,
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
}

impl MacOsMusicSkill {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            dry_run: false,
        }
    }

    pub fn new_for_tests() -> Self {
        Self {
            client: reqwest::Client::new(),
            dry_run: true,
        }
    }

    fn normalize_action(action: Option<&str>) -> String {
        action
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| "status".to_string())
    }

    async fn execute_inner(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<MediaResult, MediaSkillError> {
        if !cfg!(target_os = "macos") {
            return Err(MediaSkillError::UnsupportedAction(
                "media skill requires macOS".to_string(),
            ));
        }

        let action = Self::normalize_action(action);
        match action.as_str() {
            "play" => {
                let query = target.ok_or(MediaSkillError::NoSource)?;
                if self.dry_run {
                    return Ok(MediaResult {
                        summary: format!("Play requested for {query}"),
                        now_playing: Some(query.to_string()),
                        state: "playing".to_string(),
                    });
                }

                let label = match self.play_best_library_match(query)? {
                    Some(label) => label,
                    None => {
                        let track = self.search_track(query).await?;
                        let playback_url = Self::playback_url(&track).ok_or_else(|| {
                            MediaSkillError::Playback(
                                "catalog did not provide a playable track URL".to_string(),
                            )
                        })?;
                        self.open_track_url(playback_url)?;
                        format!("{} - {}", track.track_name, track.artist_name)
                    }
                };

                Ok(MediaResult {
                    summary: format!("Now Playing - {label}"),
                    now_playing: Some(label),
                    state: "playing".to_string(),
                })
            }
            "pause" => {
                if !self.dry_run {
                    self.run_music_script("tell application \"Music\" to pause")?;
                }
                Ok(MediaResult {
                    summary: "Paused Music.app".to_string(),
                    now_playing: None,
                    state: "paused".to_string(),
                })
            }
            "stop" => {
                if !self.dry_run {
                    self.run_music_script("tell application \"Music\" to stop")?;
                }
                Ok(MediaResult {
                    summary: "Playback stopped".to_string(),
                    now_playing: None,
                    state: "stopped".to_string(),
                })
            }
            "resume" => {
                if !self.dry_run {
                    self.run_music_script("tell application \"Music\" to play")?;
                }
                Ok(MediaResult {
                    summary: "Playback resumed".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "next" => {
                if !self.dry_run {
                    self.run_music_script("tell application \"Music\" to next track")?;
                }
                Ok(MediaResult {
                    summary: "Skipped to next track".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "previous" => {
                if !self.dry_run {
                    self.run_music_script("tell application \"Music\" to previous track")?;
                }
                Ok(MediaResult {
                    summary: "Went to previous track".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "shuffle_on" => {
                if !self.dry_run {
                    self.run_music_script(
                        "tell application \"Music\" to set shuffle enabled to true",
                    )?;
                }
                Ok(MediaResult {
                    summary: "Shuffle enabled".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "shuffle_off" => {
                if !self.dry_run {
                    self.run_music_script(
                        "tell application \"Music\" to set shuffle enabled to false",
                    )?;
                }
                Ok(MediaResult {
                    summary: "Shuffle disabled".to_string(),
                    now_playing: None,
                    state: "playing".to_string(),
                })
            }
            "status" => {
                if self.dry_run {
                    return Ok(MediaResult {
                        summary: "Music.app control ready".to_string(),
                        now_playing: None,
                        state: "unknown".to_string(),
                    });
                }
                let (state, now_playing) = self.read_status()?;
                Ok(MediaResult {
                    summary: "Music.app control ready".to_string(),
                    now_playing,
                    state,
                })
            }
            other => Err(MediaSkillError::UnsupportedAction(other.to_string())),
        }
    }

    fn escape_applescript_string(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }

    fn run_music_script(&self, script: &str) -> Result<String, MediaSkillError> {
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(MediaSkillError::Playback(if stderr.is_empty() {
                "osascript command failed".to_string()
            } else {
                stderr
            }));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn play_best_library_match(&self, query: &str) -> Result<Option<String>, MediaSkillError> {
        let script = Self::best_library_match_script(query);
        let out = self.run_music_script(&script)?;
        if out.is_empty() {
            Ok(None)
        } else {
            Ok(Some(out))
        }
    }

    fn best_library_match_script(query: &str) -> String {
        let escaped = Self::escape_applescript_string(query.trim());
        format!(
            "tell application \"Music\"\nset q to \"{escaped}\"\ntry\nplay playlist \"{escaped}\"\nreturn \"playlist|\" & q\nend try\nset chosenPlaylist to missing value\nrepeat with p in user playlists\ntry\nset trackCount to count of (tracks of p)\nif trackCount > 0 then\nset pname to (name of p as text)\nif ((pname contains q) or (q contains pname)) then\nset chosenPlaylist to p\nexit repeat\nend if\nend if\nend try\nend repeat\nif chosenPlaylist is not missing value then\ntry\nplay chosenPlaylist\non error\nset chosenTracks to tracks of chosenPlaylist\nif (count of chosenTracks) is 0 then\nset chosenPlaylist to missing value\nelse\nplay (item 1 of chosenTracks)\nend if\nend try\nif chosenPlaylist is not missing value then\nreturn \"playlist|\" & (name of chosenPlaylist as text)\nend if\nend if\nset albumExact to (tracks of library playlist 1 whose album is q)\nif (count of albumExact) > 0 then\nset chosenAlbum to item 1 of albumExact\nplay chosenAlbum\nreturn \"album|\" & ((album of chosenAlbum as text) & \" - \" & (artist of chosenAlbum as text))\nend if\nset albumContains to (tracks of library playlist 1 whose album contains q)\nif (count of albumContains) > 0 then\nset chosenAlbumPartial to item 1 of albumContains\nplay chosenAlbumPartial\nreturn \"album|\" & ((album of chosenAlbumPartial as text) & \" - \" & (artist of chosenAlbumPartial as text))\nend if\nset trackExact to (tracks of library playlist 1 whose name is q)\nif (count of trackExact) > 0 then\nset chosenTrack to item 1 of trackExact\nplay chosenTrack\nreturn \"track|\" & ((name of chosenTrack as text) & \" - \" & (artist of chosenTrack as text))\nend if\nset trackContains to (tracks of library playlist 1 whose (name contains q) or (artist contains q))\nif (count of trackContains) > 0 then\nset chosenTrackPartial to item 1 of trackContains\nplay chosenTrackPartial\nreturn \"track|\" & ((name of chosenTrackPartial as text) & \" - \" & (artist of chosenTrackPartial as text))\nend if\nreturn \"\"\nend tell"
        )
    }

    fn read_status(&self) -> Result<(String, Option<String>), MediaSkillError> {
        let script = "tell application \"Music\"\nset st to (player state as text)\nif st is \"playing\" or st is \"paused\" then\nset tname to name of current track\nset aname to artist of current track\nreturn st & \"|\" & tname & \"|\" & aname\nend if\nreturn st & \"||\"\nend tell";
        let out = self.run_music_script(script)?;
        let mut parts = out.splitn(3, '|');
        let state = parts.next().unwrap_or("unknown").trim().to_string();
        let track = parts.next().unwrap_or("").trim();
        let artist = parts.next().unwrap_or("").trim();
        let now_playing = if track.is_empty() {
            None
        } else if artist.is_empty() {
            Some(track.to_string())
        } else {
            Some(format!("{track} - {artist}"))
        };
        Ok((state, now_playing))
    }

    fn playback_url(track: &SearchItem) -> Option<&str> {
        if track.track_view_url.trim().is_empty() {
            None
        } else {
            Some(track.track_view_url.as_str())
        }
    }

    fn open_track_url(&self, url: &str) -> Result<(), MediaSkillError> {
        if self.dry_run {
            return Ok(());
        }
        let status = Command::new("open")
            .args(["-a", "Music", url])
            .status()
            .map_err(|e| MediaSkillError::Playback(e.to_string()))?;
        if !status.success() {
            return Err(MediaSkillError::Playback(
                "failed to open track in Music.app".to_string(),
            ));
        }
        Ok(())
    }

    async fn search_track(&self, query: &str) -> Result<SearchItem, MediaSkillError> {
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

    pub fn shutdown(&self) -> Result<(), MediaSkillError> {
        Ok(())
    }
}

impl Default for MacOsMusicSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MediaSkill for MacOsMusicSkill {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<MediaResult, MediaSkillError> {
        let action_label = Self::normalize_action(action);
        let t0 = Instant::now();
        let result = self
            .execute_inner(Some(action_label.as_str()), target)
            .await;

        match &result {
            Ok(_) => {
                counter!(
                    MEDIA_SKILL_EXECUTE_TOTAL,
                    1,
                    "result" => "success",
                    "action" => action_label.clone()
                );
            }
            Err(e) => {
                counter!(
                    MEDIA_SKILL_EXECUTE_TOTAL,
                    1,
                    "result" => "error",
                    "action" => action_label.clone()
                );
                counter!(
                    MEDIA_SKILL_ERRORS_TOTAL,
                    1,
                    "action" => action_label.clone(),
                    "kind" => e.to_string()
                );
            }
        }
        histogram!(
            MEDIA_SKILL_EXECUTE_DURATION_SECONDS,
            t0.elapsed().as_secs_f64(),
            "action" => action_label
        );

        result
    }
}

#[cfg(test)]
mod tests {
    pub trait TestOptionExt<T> {
        fn must(self) -> T;
    }

    impl<T> TestOptionExt<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(value) => value,
                None => panic!("expected Some(..) in test"),
            }
        }
    }

    use super::{MacOsMusicSkill, SearchItem};

    #[test]
    fn normalize_action_defaults_to_status() {
        assert_eq!(MacOsMusicSkill::normalize_action(None), "status");
    }

    #[test]
    fn escape_applescript_string_escapes_quotes() {
        let escaped = MacOsMusicSkill::escape_applescript_string("daft \"punk\"");
        assert_eq!(escaped, "daft \\\"punk\\\"");
    }

    #[test]
    fn playback_url_returns_track_view_when_present() {
        let item = SearchItem {
            track_name: "Song".to_string(),
            artist_name: "Artist".to_string(),
            track_view_url: "https://music.apple.com/us/song/id123".to_string(),
        };
        assert_eq!(
            MacOsMusicSkill::playback_url(&item),
            Some("https://music.apple.com/us/song/id123")
        );
    }

    #[test]
    fn playback_url_returns_none_when_missing() {
        let item = SearchItem {
            track_name: "Song".to_string(),
            artist_name: "Artist".to_string(),
            track_view_url: String::new(),
        };
        assert!(MacOsMusicSkill::playback_url(&item).is_none());
    }

    #[test]
    fn best_match_script_uses_bidirectional_playlist_contains() {
        let script = MacOsMusicSkill::best_library_match_script("favorites playlist");
        assert!(script.contains("play playlist \"favorites playlist\""));
        assert!(script.contains("pname contains q"));
        assert!(script.contains("q contains pname"));
        assert!(script.contains("repeat with p in user playlists"));
        assert!(script.contains("set chosenPlaylist to missing value"));
        assert!(script.contains("play chosenPlaylist"));
        assert!(script.contains("set trackCount to count of (tracks of p)"));
        assert!(script.contains("if trackCount > 0 then"));
        assert!(script.contains("play (item 1 of chosenTracks)"));
    }

    #[test]
    fn best_match_script_checks_playlist_album_track_in_order() {
        let script = MacOsMusicSkill::best_library_match_script("favorites");
        let playlist_idx = script.find("play playlist").must();
        let album_idx = script
            .find("set albumExact to (tracks of library playlist 1 whose album is q)")
            .must();
        let track_idx = script
            .find("set trackExact to (tracks of library playlist 1 whose name is q)")
            .must();
        assert!(playlist_idx < album_idx);
        assert!(album_idx < track_idx);
        assert!(script.contains("return \"playlist|\""));
        assert!(script.contains("return \"album|\""));
        assert!(script.contains("return \"track|\""));
    }
}
