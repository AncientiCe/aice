use crate::types::MediaSkillError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerStatus {
    pub state: String,
    pub now_playing: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BridgeCommand {
    PlayBySongId {
        song_id: String,
        label: String,
        storefront: String,
        is_library: bool,
    },
    PlayByPlaylistId {
        playlist_id: String,
        label: String,
        storefront: String,
        is_library: bool,
    },
    SearchLibrarySong {
        request_id: u64,
        query: String,
    },
    SearchLibraryPlaylist {
        request_id: u64,
        query: String,
    },
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    ShuffleOn,
    ShuffleOff,
    Status,
}

#[derive(Default)]
struct BridgeState {
    queue: VecDeque<BridgeCommand>,
    status: Option<PlayerStatus>,
    search_results: HashMap<u64, LibrarySearchResultPayload>,
    next_request_id: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct LibrarySearchResultPayload {
    request_id: u64,
    item_id: Option<String>,
    label: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Default)]
struct BridgeInner {
    state: BridgeState,
    address: Option<String>,
    browser_pid: Option<u32>,
}

#[derive(Deserialize)]
struct UserTokenPayload {
    user_token: String,
}

#[derive(Clone)]
pub struct MusicKitBridge {
    inner: Arc<Mutex<BridgeInner>>,
}

impl MusicKitBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BridgeInner::default())),
        }
    }

    pub fn enqueue(&self, command: BridgeCommand) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.state.queue.push_back(command);
        }
    }

    pub fn latest_status(&self) -> Option<PlayerStatus> {
        self.inner.lock().ok().and_then(|g| g.state.status.clone())
    }

    pub fn wait_for_state(&self, desired: &str, timeout: Duration) -> Option<PlayerStatus> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.latest_status() {
                if status.state.eq_ignore_ascii_case(desired) {
                    return Some(status);
                }
            }
            thread::sleep(Duration::from_millis(120));
        }
        self.latest_status()
    }

    pub fn search_library_song(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Option<(String, String)>, MediaSkillError> {
        self.search_library(query, timeout, true)
    }

    pub fn search_library_playlist(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Option<(String, String)>, MediaSkillError> {
        self.search_library(query, timeout, false)
    }

    fn search_library(
        &self,
        query: &str,
        timeout: Duration,
        song: bool,
    ) -> Result<Option<(String, String)>, MediaSkillError> {
        let request_id = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| MediaSkillError::Playback("bridge lock poisoned".to_string()))?;
            if guard.state.next_request_id == 0 {
                guard.state.next_request_id = 1;
            }
            let id = guard.state.next_request_id;
            guard.state.next_request_id = guard.state.next_request_id.saturating_add(1);
            let cmd = if song {
                BridgeCommand::SearchLibrarySong {
                    request_id: id,
                    query: query.to_string(),
                }
            } else {
                BridgeCommand::SearchLibraryPlaylist {
                    request_id: id,
                    query: query.to_string(),
                }
            };
            guard.state.queue.push_back(cmd);
            id
        };

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(mut guard) = self.inner.lock() {
                if let Some(result) = guard.state.search_results.remove(&request_id) {
                    if result.error.is_some() {
                        return Ok(None);
                    }
                    if let (Some(id), Some(label)) = (result.item_id, result.label) {
                        return Ok(Some((id, label)));
                    }
                    return Ok(None);
                }
            }
            thread::sleep(Duration::from_millis(80));
        }
        Ok(None)
    }

    pub fn ensure_started(
        &self,
        developer_token: &str,
        user_token: &str,
    ) -> Result<String, MediaSkillError> {
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(pid) = guard.browser_pid {
                if !is_process_running(pid) {
                    guard.address = None;
                    guard.browser_pid = None;
                }
            }
            if let Some(addr) = guard.address.clone() {
                return Ok(addr);
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| MediaSkillError::Playback(format!("bridge bind failed: {e}")))?;
        let addr = listener
            .local_addr()
            .map_err(|e| MediaSkillError::Playback(format!("bridge addr failed: {e}")))?;
        let server = Server::from_listener(listener, None)
            .map_err(|e| MediaSkillError::Playback(format!("bridge startup failed: {e}")))?;
        let inner = self.inner.clone();
        let persisted_user_token = load_persisted_user_token().unwrap_or_default();
        let effective_user_token = if !user_token.trim().is_empty() {
            user_token.to_string()
        } else {
            persisted_user_token
        };
        let html = player_html(developer_token, &effective_user_token);

        thread::spawn(move || {
            for req in server.incoming_requests() {
                let _ = handle_request(req, &inner, &html);
            }
        });

        let base = format!("http://127.0.0.1:{}", addr.port());
        let pid = launch_player_window(&format!("{base}/player"))?;
        if let Ok(mut guard) = self.inner.lock() {
            guard.address = Some(base.clone());
            guard.browser_pid = Some(pid);
        }
        Ok(base)
    }

    pub fn shutdown(&self) -> Result<(), MediaSkillError> {
        let pid = if let Ok(mut guard) = self.inner.lock() {
            guard.state.queue.clear();
            guard.state.search_results.clear();
            guard.address = None;
            guard.browser_pid.take()
        } else {
            None
        };
        if let Some(pid) = pid {
            let _ = kill_process_tree(pid);
        }
        Ok(())
    }
}

impl Drop for MusicKitBridge {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }
        let _ = self.shutdown();
    }
}

fn json_response<T: Serialize>(value: &T) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let mut resp = Response::from_data(body);
    if let Ok(h) = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]) {
        resp = resp.with_header(h);
    }
    resp
}

fn handle_request(
    req: Request,
    inner: &Arc<Mutex<BridgeInner>>,
    html: &str,
) -> Result<(), MediaSkillError> {
    let mut req = req;
    let url = req.url().to_string();
    match (req.method(), url.as_str()) {
        (&Method::Get, "/favicon.ico") => {
            let _ = req.respond(Response::empty(StatusCode(204)));
        }
        (&Method::Get, "/player") => {
            let mut resp = Response::from_string(html.to_string());
            if let Ok(h) =
                Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
            {
                resp = resp.with_header(h);
            }
            let _ = req.respond(resp);
        }
        (&Method::Get, "/command") => {
            let command = inner
                .lock()
                .ok()
                .and_then(|mut g| g.state.queue.pop_front());
            let payload = command.unwrap_or(BridgeCommand::Status);
            let _ = req.respond(json_response(&payload));
        }
        (&Method::Post, "/status") => {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
            if let Ok(status) = serde_json::from_str::<PlayerStatus>(&body) {
                if let Ok(mut guard) = inner.lock() {
                    guard.state.status = Some(status);
                }
            }
            let _ = req.respond(Response::empty(StatusCode(204)));
        }
        (&Method::Post, "/user-token") => {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
            if let Ok(payload) = serde_json::from_str::<UserTokenPayload>(&body) {
                let _ = save_persisted_user_token(&payload.user_token);
            }
            let _ = req.respond(Response::empty(StatusCode(204)));
        }
        (&Method::Post, "/library-search-result") => {
            let mut body = String::new();
            let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
            if let Ok(payload) = serde_json::from_str::<LibrarySearchResultPayload>(&body) {
                if let Ok(mut guard) = inner.lock() {
                    guard
                        .state
                        .search_results
                        .insert(payload.request_id, payload);
                }
            }
            let _ = req.respond(Response::empty(StatusCode(204)));
        }
        _ => {
            let _ = req.respond(Response::empty(StatusCode(404)));
        }
    }
    Ok(())
}

fn launch_player_window(url: &str) -> Result<u32, MediaSkillError> {
    let profile_dir = musickit_profile_dir()?;
    let args = [
        format!("--app={url}"),
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        "--autoplay-policy=no-user-gesture-required".to_string(),
        "--window-size=420,240".to_string(),
        "--window-position=20,20".to_string(),
    ];
    if let Ok(child) = Command::new("msedge").args(&args).spawn() {
        return Ok(child.id());
    }

    for candidate in edge_executable_candidates() {
        if candidate.exists() {
            if let Ok(child) = Command::new(&candidate).args(&args).spawn() {
                return Ok(child.id());
            }
        }
    }

    Err(MediaSkillError::Playback(
        "failed to launch MusicKit player: program not found".to_string(),
    ))
}

fn musickit_profile_dir() -> Result<PathBuf, MediaSkillError> {
    let cwd = std::env::current_dir()
        .map_err(|e| MediaSkillError::Playback(format!("failed to resolve working dir: {e}")))?;
    let dir = cwd.join(".aice").join("musickit-edge-profile");
    std::fs::create_dir_all(&dir).map_err(|e| {
        MediaSkillError::Playback(format!("failed to create MusicKit profile dir: {e}"))
    })?;
    Ok(dir)
}

fn edge_executable_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(v) = std::env::var("ProgramFiles") {
        out.push(
            PathBuf::from(v)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe"),
        );
    }
    if let Ok(v) = std::env::var("ProgramFiles(x86)") {
        out.push(
            PathBuf::from(v)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe"),
        );
    }
    if let Ok(v) = std::env::var("LocalAppData") {
        out.push(
            PathBuf::from(v)
                .join("Microsoft")
                .join("Edge")
                .join("Application")
                .join("msedge.exe"),
        );
    }
    out
}

fn kill_process_tree(pid: u32) -> Result<(), MediaSkillError> {
    let graceful = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .status()
        .map_err(|e| MediaSkillError::Playback(format!("failed to stop MusicKit player: {e}")))?;
    if graceful.success() {
        return Ok(());
    }
    let forced = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map_err(|e| {
            MediaSkillError::Playback(format!("failed to force-stop MusicKit player: {e}"))
        })?;
    if forced.success() {
        return Ok(());
    }
    Err(MediaSkillError::Playback(format!(
        "failed to stop MusicKit player pid {pid}"
    )))
}

fn is_process_running(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout).to_lowercase();
    text.contains(&pid.to_string()) && !text.contains("no tasks are running")
}

fn js_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\"', "\\\"")
}

fn musickit_user_token_path() -> Result<PathBuf, MediaSkillError> {
    let cwd = std::env::current_dir()
        .map_err(|e| MediaSkillError::Playback(format!("failed to resolve working dir: {e}")))?;
    Ok(cwd.join(".aice").join("musickit-user-token.txt"))
}

fn load_persisted_user_token() -> Result<String, MediaSkillError> {
    let path = musickit_user_token_path()?;
    if !path.exists() {
        return Ok(String::new());
    }
    let token = std::fs::read_to_string(path).map_err(|e| {
        MediaSkillError::Playback(format!("failed to read MusicKit user token: {e}"))
    })?;
    Ok(token.trim().to_string())
}

fn save_persisted_user_token(token: &str) -> Result<(), MediaSkillError> {
    let normalized = token.trim();
    if normalized.is_empty() {
        return Ok(());
    }
    let path = musickit_user_token_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            MediaSkillError::Playback(format!("failed to create MusicKit token dir: {e}"))
        })?;
    }
    std::fs::write(path, normalized).map_err(|e| {
        MediaSkillError::Playback(format!("failed to persist MusicKit user token: {e}"))
    })?;
    Ok(())
}

fn player_html(dev_token: &str, user_token: &str) -> String {
    let dev = js_string(dev_token);
    let user = js_string(user_token);
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8" />
  <title>AICE MusicKit Bridge</title>
  <script src="https://js-cdn.music.apple.com/musickit/v3/musickit.js"></script>
</head>
<body>
<button id="unlockBtn" style="display:none;position:fixed;inset:0;border:0;background:#111;color:#fff;font:600 20px sans-serif;z-index:9999;">
  Click to enable Apple Music playback
</button>
<script>
const DEV_TOKEN = "{dev}";
const USER_TOKEN = "{user}";
let mk = null;
let state = "idle";
let nowPlaying = null;
let lastError = null;
let pendingPlay = null;

function showUnlockButton() {{
  const btn = document.getElementById("unlockBtn");
  if (btn) btn.style.display = "block";
}}

function hideUnlockButton() {{
  const btn = document.getElementById("unlockBtn");
  if (btn) btn.style.display = "none";
}}

function hasUserToken() {{
  return !!(mk && typeof mk.musicUserToken === "string" && mk.musicUserToken.trim());
}}

async function postUserToken() {{
  if (!hasUserToken()) return;
  try {{
    await fetch("/user-token", {{
      method: "POST",
      headers: {{ "Content-Type": "application/json" }},
      body: JSON.stringify({{ user_token: mk.musicUserToken }})
    }});
  }} catch (_e) {{}}
}}

function normalizeForMatch(input) {{
  return (input || "")
    .toLowerCase()
    .replace(/[^a-z0-9\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}}

function scoreMatch(name, query) {{
  const n = normalizeForMatch(name);
  const q = normalizeForMatch(query);
  if (!n || !q) return 0;
  if (n === q) return 3;
  if (n.includes(q) || q.includes(n)) return 2;
  return 1;
}}

function pickBestLibraryItem(items, query, kind) {{
  let best = null;
  for (const it of items || []) {{
    const attrs = it && it.attributes ? it.attributes : {{}};
    const name = attrs.name || "";
    const score = scoreMatch(name, query);
    const label = kind === "song"
      ? `${{name}} - ${{attrs.artistName || "Unknown Artist"}}`
      : name;
    if (!best || score > best.score) {{
      best = {{ score, id: it.id, label }};
    }}
  }}
  return best;
}}

async function postLibrarySearchResult(payload) {{
  try {{
    await fetch("/library-search-result", {{
      method: "POST",
      headers: {{ "Content-Type": "application/json" }},
      body: JSON.stringify(payload)
    }});
  }} catch (_e) {{}}
}}

async function searchLibrary(kind, requestId, query) {{
  if (!hasUserToken()) {{
    await postLibrarySearchResult({{
      request_id: requestId,
      item_id: null,
      label: null,
      error: "not_authorized"
    }});
    return;
  }}
  const apiType = kind === "song" ? "library-songs" : "library-playlists";
  const url = `https://api.music.apple.com/v1/me/library/search?term=${{encodeURIComponent(query)}}&types=${{apiType}}&limit=8`;
  try {{
    const resp = await fetch(url, {{
      headers: {{
        "Authorization": `Bearer ${{DEV_TOKEN}}`,
        "Music-User-Token": mk.musicUserToken
      }}
    }});
    if (!resp.ok) {{
      await postLibrarySearchResult({{
        request_id: requestId,
        item_id: null,
        label: null,
        error: `status_${{resp.status}}`
      }});
      return;
    }}
    const body = await resp.json();
    const bucket = body && body.results ? body.results[apiType] : null;
    const items = bucket && Array.isArray(bucket.data) ? bucket.data : [];
    const best = pickBestLibraryItem(items, query, kind);
    await postLibrarySearchResult({{
      request_id: requestId,
      item_id: best ? best.id : null,
      label: best ? best.label : null,
      error: null
    }});
  }} catch (_e) {{
    await postLibrarySearchResult({{
      request_id: requestId,
      item_id: null,
      label: null,
      error: (_e && _e.message) ? _e.message : "library_search_failed"
    }});
  }}
}}

async function reportStatus() {{
  try {{
    await fetch("/status", {{
      method: "POST",
      headers: {{ "Content-Type": "application/json" }},
      body: JSON.stringify({{
        state,
        now_playing: nowPlaying,
        error: lastError,
        updated_at_unix_ms: Date.now()
      }})
    }});
  }} catch (_e) {{}}
}}

async function init() {{
  if (!window.MusicKit || typeof MusicKit.configure !== "function") {{
    state = "error";
    lastError = "MusicKit script unavailable";
    await reportStatus();
    return;
  }}
  try {{
    const configured = await MusicKit.configure({{
      developerToken: DEV_TOKEN,
      app: {{
        name: "AICE Hidden MusicKit",
        build: "1.0.0"
      }}
    }});
    mk = configured || (typeof MusicKit.getInstance === "function" ? MusicKit.getInstance() : null);
    if (!mk) {{
      throw new Error("MusicKit instance unavailable after configure");
    }}
    if (USER_TOKEN && USER_TOKEN.trim() && !hasUserToken()) {{
      mk.musicUserToken = USER_TOKEN;
    }}
    await postUserToken();
    if (typeof mk.play !== "function") {{
      throw new Error("MusicKit instance missing play()");
    }}
    state = "ready";
    lastError = null;
  }} catch (e) {{
    state = "error";
    lastError = (e && e.message) ? e.message : "MusicKit init failed";
  }}
  await reportStatus();
}}

async function doPlay(songId, label, storefront, isLibrary) {{
  pendingPlay = {{ kind: "song", songId, label, storefront, isLibrary }};
  if (!hasUserToken()) {{
    state = "blocked_user_interaction";
    lastError = "authorization required";
    showUnlockButton();
    await reportStatus();
    return;
  }}
  const candidates = isLibrary
    ? [
        {{ song: songId, isLibrary: true }},
        {{ songs: [songId], isLibrary: true }},
        {{ kind: "library-songs", id: songId }},
        {{ librarySong: songId }},
        {{ librarySongs: [songId] }},
      ]
    : [
        {{ song: songId }},
        {{ songs: [songId] }},
      ];
  let queued = false;
  for (const q of candidates) {{
    try {{
      await mk.setQueue(q);
      queued = true;
      break;
    }} catch (_e) {{}}
  }}
  if (!queued) {{
    throw new Error("queue setup failed");
  }}
  try {{
    await mk.play();
  }} catch (e) {{
    const message = (e && e.message) ? e.message : "play failed";
    if (message.toLowerCase().includes("didn't interact") || message.toLowerCase().includes("did not interact")) {{
      state = "blocked_user_interaction";
      lastError = message;
      showUnlockButton();
      await reportStatus();
      return;
    }}
    if (message.includes("403")) {{
      state = "error";
      lastError = "apple account access failed (403)";
      await reportStatus();
      return;
    }}
    throw e;
  }}
  nowPlaying = label || null;
  state = "playing";
  lastError = null;
  pendingPlay = null;
  hideUnlockButton();
}}

async function doPlayPlaylist(playlistId, label, storefront, isLibrary) {{
  pendingPlay = {{ kind: "playlist", playlistId, label, storefront, isLibrary }};
  if (!hasUserToken()) {{
    state = "blocked_user_interaction";
    lastError = "authorization required";
    showUnlockButton();
    await reportStatus();
    return;
  }}
  const candidates = isLibrary
    ? [
        {{ playlist: playlistId, isLibrary: true }},
        {{ playlists: [playlistId], isLibrary: true }},
        {{ kind: "library-playlists", id: playlistId }},
        {{ libraryPlaylist: playlistId }},
        {{ libraryPlaylists: [playlistId] }},
      ]
    : [
        {{ playlist: playlistId }},
        {{ playlists: [playlistId] }},
      ];
  let queued = false;
  for (const q of candidates) {{
    try {{
      await mk.setQueue(q);
      queued = true;
      break;
    }} catch (_e) {{}}
  }}
  if (!queued) {{
    throw new Error("playlist queue setup failed");
  }}
  try {{
    await mk.play();
  }} catch (e) {{
    const message = (e && e.message) ? e.message : "play failed";
    if (message.toLowerCase().includes("didn't interact") || message.toLowerCase().includes("did not interact")) {{
      state = "blocked_user_interaction";
      lastError = message;
      showUnlockButton();
      await reportStatus();
      return;
    }}
    throw e;
  }}
  nowPlaying = label || null;
  state = "playing";
  lastError = null;
  pendingPlay = null;
  hideUnlockButton();
}}

async function unlockAndPlay() {{
  if (!mk || !pendingPlay) return;
  hideUnlockButton();
  try {{
    if (!hasUserToken()) {{
      await mk.authorize();
    }}
    if (!hasUserToken()) {{
      throw new Error("authorization not completed");
    }}
    await postUserToken();
    if (pendingPlay.kind === "playlist") {{
      await doPlayPlaylist(
        pendingPlay.playlistId,
        pendingPlay.label,
        pendingPlay.storefront,
        !!pendingPlay.isLibrary
      );
    }} else {{
      await doPlay(
        pendingPlay.songId,
        pendingPlay.label,
        pendingPlay.storefront,
        !!pendingPlay.isLibrary
      );
    }}
  }} catch (e) {{
    state = "blocked_user_interaction";
    lastError = (e && e.message) ? e.message : "play failed after click";
    showUnlockButton();
  }}
  await reportStatus();
}}

async function applyCommand(cmd) {{
  if (!cmd || !cmd.type) return;
  try {{
    switch (cmd.type) {{
      case "PlayBySongId":
        await doPlay(cmd.song_id, cmd.label, cmd.storefront, !!cmd.is_library);
        break;
      case "PlayByPlaylistId":
        await doPlayPlaylist(cmd.playlist_id, cmd.label, cmd.storefront, !!cmd.is_library);
        break;
      case "SearchLibrarySong":
        await searchLibrary("song", cmd.request_id, cmd.query || "");
        break;
      case "SearchLibraryPlaylist":
        await searchLibrary("playlist", cmd.request_id, cmd.query || "");
        break;
      case "Pause":
        await mk.pause();
        state = "paused";
        break;
      case "Resume":
        await mk.play();
        state = "playing";
        break;
      case "Stop":
        if (typeof mk.stop === "function") {{
          await mk.stop();
        }} else {{
          await mk.pause();
        }}
        state = "stopped";
        break;
      case "Next":
        await mk.skipToNextItem();
        state = "playing";
        break;
      case "Previous":
        await mk.skipToPreviousItem();
        state = "playing";
        break;
      case "ShuffleOn":
        if (typeof mk.setShuffleMode === "function") {{
          await mk.setShuffleMode(1);
        }} else if (typeof mk.shuffleMode !== "undefined") {{
          mk.shuffleMode = 1;
        }} else if (mk.player && typeof mk.player.setShuffleMode === "function") {{
          await mk.player.setShuffleMode(1);
        }} else if (mk.player && typeof mk.player.shuffleMode !== "undefined") {{
          mk.player.shuffleMode = 1;
        }} else {{
          throw new Error("shuffle mode unsupported");
        }}
        state = "playing";
        break;
      case "ShuffleOff":
        if (typeof mk.setShuffleMode === "function") {{
          await mk.setShuffleMode(0);
        }} else if (typeof mk.shuffleMode !== "undefined") {{
          mk.shuffleMode = 0;
        }} else if (mk.player && typeof mk.player.setShuffleMode === "function") {{
          await mk.player.setShuffleMode(0);
        }} else if (mk.player && typeof mk.player.shuffleMode !== "undefined") {{
          mk.player.shuffleMode = 0;
        }} else {{
          throw new Error("shuffle mode unsupported");
        }}
        state = "playing";
        break;
      default:
        break;
    }}
  }} catch (_e) {{
    state = "error";
    lastError = (_e && _e.message) ? _e.message : "command failed";
  }}
  await reportStatus();
}}

async function poll() {{
  if (!mk) return;
  try {{
    const resp = await fetch("/command", {{ cache: "no-store" }});
    const cmd = await resp.json();
    await applyCommand(cmd);
  }} catch (_e) {{
    state = "error";
    lastError = (_e && _e.message) ? _e.message : "bridge poll failed";
    await reportStatus();
  }}
}}

const unlockBtn = document.getElementById("unlockBtn");
if (unlockBtn) {{
  unlockBtn.addEventListener("click", () => {{
    unlockAndPlay();
  }});
}}

init().then(() => {{
  setInterval(poll, 250);
  setInterval(reportStatus, 1000);
}});
</script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::{
        edge_executable_candidates, load_persisted_user_token, musickit_user_token_path,
        save_persisted_user_token, MusicKitBridge,
    };

    #[test]
    fn edge_candidates_include_standard_install_locations() {
        let candidates = edge_executable_candidates();
        let joined = candidates
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(";");
        assert!(joined
            .to_lowercase()
            .contains("microsoft\\edge\\application\\msedge.exe"));
    }

    #[test]
    fn shutdown_without_running_browser_is_ok() {
        let bridge = MusicKitBridge::new();
        assert!(bridge.shutdown().is_ok());
    }

    #[test]
    fn persisted_user_token_roundtrip() {
        let path = musickit_user_token_path().expect("token path");
        let _ = std::fs::remove_file(&path);
        save_persisted_user_token("test_user_token").expect("save");
        let token = load_persisted_user_token().expect("load");
        assert_eq!(token, "test_user_token");
        let _ = std::fs::remove_file(path);
    }
}
