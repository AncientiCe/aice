/**
 * Room pod firmware — Waveshare ESP32-S3-AUDIO-Board (hardware in src/board.cpp).
 *
 * Protocol: see crates/pod-protocol/src/lib.rs
 *   Pod → Gateway  : Hello, Audio, Identify, TapActivate, Ping
 *   Gateway → Pod  : HelloAck, Audio, StopAudio, Led, Pong, Error
 *
 * Trust: the pod enrols with the property facilitator over HTTPS (property CA
 * from property_trust.h), gets a device token once a supervisor assigns its
 * room, and connects to the room bridge over WSS with that token. Updates are
 * installed only when their ECDSA signature verifies. Hold the button while
 * booting for 5 s to forget the pod's identity.
 *
 * Audio path:
 *   ES7210 mic → 16 kHz mono PCM16 → WebSocket Audio frames
 *   TTS playback: Gateway Audio (base64 PCM) → queue → ES8311 → speaker
 * Full duplex: the mic keeps streaming while an answer plays, each frame with
 * the speaker signal of the same samples; the bridge cancels the echo and lets
 * the guest's speech interrupt the answer (docs/hardware/signal-pod.md).
 */

#include <Arduino.h>
#include <WiFi.h>
#include <ESPmDNS.h>
#include <WebSocketsClient.h>
#include <ArduinoJson.h>
#include <cstring>

#if __has_include("pod_config.h")
#include "pod_config.h"
#else
#include "pod_config.example.h"
#endif
#if __has_include("property_trust.h")
#include "property_trust.h"
#else
#include "property_trust.example.h"
#endif
#include "board.h"
#include "pod_defaults.h"
#include "provisioning.h"

// ── Audio sizing ──────────────────────────────────────────────────────────────
static constexpr int SAMPLES_PER_FRAME = PDM_SAMPLE_RATE * AUDIO_FRAME_MS / 1000;
static constexpr int BYTES_PER_FRAME   = SAMPLES_PER_FRAME * sizeof(int16_t);
// Frames sent per loop at most; the capture buffer holds six.
static constexpr int MAX_FRAMES_PER_LOOP = 4;
// Any clock before 2023-11 means NTP has not answered yet.
static constexpr time_t MIN_VALID_EPOCH = 1700000000;
static constexpr size_t POD_PLAYBACK_QUEUE_SLOTS = 6;
static constexpr size_t POD_PLAYBACK_MAX_CHUNK_BYTES = 2048;
// TTS gain control tuned for intelligibility without fuzz/clipping.
static constexpr int POD_TTS_TARGET_PEAK = 22000; // leave headroom to reduce harshness
static constexpr int POD_TTS_GAIN_Q10_MIN = 768;  // 0.75x
static constexpr int POD_TTS_GAIN_Q10_MAX = 1792; // 1.75x

// ── State ─────────────────────────────────────────────────────────────────────
enum class PodState {
    Booting,
    WifiConnecting,
    Provisioning,
    WsConnecting,
    Listening,
    Thinking,
    Speaking,
    Error,
};

static PodState    g_state         = PodState::Booting;
static bool        g_ws_connected  = false;
static uint64_t    g_ping_seq      = 0;
static uint32_t    g_last_ping_ms  = 0;
static bool        g_mic_running   = false;
static bool        g_identified    = false;
static uint32_t    g_playback_chunks_played = 0;
static uint32_t    g_last_tts_enqueue_ms = 0;
static constexpr uint32_t TTS_END_GRACE_MS = 180;

static WebSocketsClient g_ws;
static uint32_t g_ws_disconnects = 0;
static String   g_device_id;
static bool     g_muted = false;
static String   g_token;
static uint32_t g_last_update_check_ms = 0;
static int32_t g_tts_gain_q10 = 1024; // smoothed gain state (1.0x)

struct PlaybackChunk {
    size_t len;
    uint8_t data[POD_PLAYBACK_MAX_CHUNK_BYTES];
};

static PlaybackChunk g_playback_queue[POD_PLAYBACK_QUEUE_SLOTS];
static size_t g_playback_head = 0;
static size_t g_playback_tail = 0;
static size_t g_playback_count = 0;

// ── Forward declarations ──────────────────────────────────────────────────────
static void set_state(PodState s);
static void led_update();
static void ws_event_handler(WStype_t type, uint8_t *payload, size_t length);
static void send_hello();
static void send_ping();
static void send_tap_activate();
static void send_help_button();
static void start_mic();
static void stop_mic();
static void start_speaker();
static void stop_speaker();
static void capture_and_send_audio();
static void play_audio_frame(const uint8_t *pcm_bytes, size_t len);
static bool enqueue_playback(const uint8_t *pcm_bytes, size_t len);
static void playback_drain_once();
static void clear_playback_queue();
static String base64_encode(const uint8_t *data, size_t len);
static size_t base64_decode(const char *src, uint8_t *dst, size_t dst_max);

// ── setup() ──────────────────────────────────────────────────────────────────
void setup() {
    Serial.begin(115200);
    delay(200);
    Serial.println("[aice-pod] booting...");

    if (!board_init()) {
        // Without working audio the pod cannot serve the room; blink red and retry.
        set_state(PodState::Error);
        uint32_t failed_at = millis();
        while (millis() - failed_at < 5000) {
            led_update();
            delay(50);
        }
        ESP.restart();
    }
    set_state(PodState::Booting);

    // Button held at boot: factory reset.
    pod_factory_reset_if_held(board_button_down);

    // Wi-Fi
    set_state(PodState::WifiConnecting);
    Serial.printf("[aice-pod] connecting to SSID: %s\n", WIFI_SSID);
    WiFi.mode(WIFI_STA);
    WiFi.begin(WIFI_SSID, WIFI_PASSWORD);

    uint32_t t0 = millis();
    while (WiFi.status() != WL_CONNECTED) {
        if (millis() - t0 > WIFI_TIMEOUT_MS) {
            Serial.println("[aice-pod] Wi-Fi timeout — rebooting");
            delay(1000);
            ESP.restart();
        }
        delay(250);
        led_update(); // blink while connecting
    }
    Serial.printf("[aice-pod] Wi-Fi OK, IP: %s\n", WiFi.localIP().toString().c_str());
    g_device_id = pod_device_id();
    // mDNS lets the pod resolve a host name ending in .local (the facilitator and
    // bridge on a property without its own DNS).
    if (!MDNS.begin(g_device_id.c_str())) {
        Serial.println("[aice-pod] mDNS did not start; .local names will not resolve");
    }
    Serial.printf("[aice-pod] device id %s, firmware %s\n", g_device_id.c_str(), FIRMWARE_VERSION);

    // Certificates are checked against the clock; set it before any TLS.
    configTime(0, 0, NTP_SERVER_1, NTP_SERVER_2);
    uint32_t ntp_started = millis();
    while (time(nullptr) < MIN_VALID_EPOCH && millis() - ntp_started < NTP_TIMEOUT_MS) {
        led_update();
        delay(100);
    }
    if (time(nullptr) < MIN_VALID_EPOCH) {
        Serial.println("[aice-pod] clock not set (time servers unreachable); TLS will fail");
    }
    IPAddress gateway_ip;
    if (WiFi.hostByName(GATEWAY_HOST, gateway_ip)) {
        Serial.printf("[aice-pod] %s is %s\n", GATEWAY_HOST, gateway_ip.toString().c_str());
    } else {
        Serial.printf("[aice-pod] cannot resolve %s\n", GATEWAY_HOST);
    }

    // Enrol until a supervisor assigns this pod a room.
    g_token = pod_load_token();
    if (g_token.length() == 0) {
        set_state(PodState::Provisioning);
        while (true) {
            EnrollResult result = pod_enroll(false);
            if (result == EnrollResult::Enrolled) {
                g_token = pod_load_token();
                break;
            }
            set_state(result == EnrollResult::Pending || result == EnrollResult::Failed
                          ? PodState::Provisioning
                          : PodState::Error);
            uint32_t wait_started = millis();
            while (millis() - wait_started < ENROLL_POLL_MS) {
                led_update();
                delay(50);
            }
        }
    }

    pod_check_for_update(g_token);
    g_last_update_check_ms = millis();

    // WebSocket to the room bridge, TLS with the property CA, device token as bearer.
    set_state(PodState::WsConnecting);
    g_ws.beginSslWithCA(GATEWAY_HOST, GATEWAY_PORT, GATEWAY_PATH, PROPERTY_CA_PEM);
    String auth_header = "Authorization: Bearer " + g_token;
    g_ws.setExtraHeaders(auth_header.c_str());
    g_ws.onEvent(ws_event_handler);
    g_ws.setReconnectInterval(RECONNECT_BACKOFF_MS);

    Serial.printf("[aice-pod] connecting to gateway %s:%d\n", GATEWAY_HOST, GATEWAY_PORT);
}

// ── loop() ───────────────────────────────────────────────────────────────────
void loop() {
    g_ws.loop();
    led_update();

    // Button: tap stops playback, double tap toggles the privacy mute, and a
    // long press calls for help without speech (also while muted).
    static bool btn_last = false;
    static uint32_t btn_down_ms = 0;
    static uint32_t tap_released_ms = 0;
    static bool tap_pending = false;
    bool btn_now = board_button_down();
    if (!btn_last && btn_now) {
        btn_down_ms = millis();
    } else if (btn_last && !btn_now && g_ws_connected) {
        if (millis() - btn_down_ms >= HELP_PRESS_MS) {
            Serial.println("[aice-pod] long press — calling for help");
            tap_pending = false;
            send_help_button();
        } else if (tap_pending && millis() - tap_released_ms <= DOUBLE_TAP_MS) {
            tap_pending = false;
            g_muted = !g_muted;
            Serial.printf("[aice-pod] privacy mute %s\n", g_muted ? "on" : "off");
            led_update();
        } else {
            tap_pending = true;
            tap_released_ms = millis();
        }
    }
    if (tap_pending && millis() - tap_released_ms > DOUBLE_TAP_MS) {
        tap_pending = false;
        Serial.println("[aice-pod] button tap — stop");
        send_tap_activate();
    }
    btn_last = btn_now;

    // Periodic ping
    if (g_ws_connected && millis() - g_last_ping_ms >= PING_INTERVAL_MS) {
        send_ping();
        g_last_ping_ms = millis();
    }

    // Mic capture whenever connected (also while an answer plays, so the guest
    // can talk over it) unless the room has muted the pod.
    if (g_ws_connected && g_mic_running && !g_muted) {
        capture_and_send_audio();
    }

    // Drain one queued TTS chunk per loop; keeps WS reader non-blocking.
    playback_drain_once();

    // Look for signed updates while idle.
    if (g_state == PodState::Listening && g_playback_count == 0 &&
        millis() - g_last_update_check_ms >= OTA_CHECK_INTERVAL_MS) {
        g_last_update_check_ms = millis();
        pod_check_for_update(g_token);
    }
}

// ── LED ───────────────────────────────────────────────────────────────────────
static void set_state(PodState s) {
    g_state = s;
    led_update();
}

static void led_update() {
    static uint32_t last_blink = 0;
    static bool     blink_on   = false;
    uint8_t r = 0, g = 0, b = 0;

    switch (g_state) {
        case PodState::Booting:
            r = g = b = 20;                                   // dim white
            break;
        case PodState::Provisioning:
            // Slow amber blink: waiting for a supervisor to assign a room.
            if (millis() - last_blink > 700) {
                blink_on = !blink_on;
                last_blink = millis();
            }
            if (blink_on) { r = 60; g = 30; }
            break;
        case PodState::WifiConnecting:
        case PodState::WsConnecting:
            if (millis() - last_blink > 300) {
                blink_on = !blink_on;
                last_blink = millis();
            }
            if (blink_on) b = 60;
            break;
        case PodState::Listening:
            // Magenta while muted so the room can see nothing is being heard.
            if (g_muted) { r = 60; b = 60; } else { g = 60; }
            break;
        case PodState::Thinking:
            r = 60; g = 40;                                   // amber
            break;
        case PodState::Speaking:
            b = 80;                                           // blue
            break;
        case PodState::Error:
            if (millis() - last_blink > 150) {
                blink_on = !blink_on;
                last_blink = millis();
            }
            if (blink_on) r = 80;
            break;
    }
    // Only refresh the ring when the colour changes; this runs every loop.
    static uint32_t last_rgb = 0xFFFFFFFF;
    uint32_t rgb = ((uint32_t)r << 16) | ((uint32_t)g << 8) | b;
    if (rgb != last_rgb) {
        last_rgb = rgb;
        board_led_set(r, g, b);
    }
}

// ── WebSocket events ──────────────────────────────────────────────────────────
static void ws_event_handler(WStype_t type, uint8_t *payload, size_t length) {
    switch (type) {
        case WStype_DISCONNECTED:
            g_ws_disconnects++;
            Serial.printf("[aice-pod] WS disconnected (count=%lu)\n", (unsigned long)g_ws_disconnects);
            g_ws_connected = false;
            g_identified   = false;
            stop_mic();
            stop_speaker();
            clear_playback_queue();
            set_state(PodState::WsConnecting);
            break;

        case WStype_CONNECTED:
            Serial.printf("[aice-pod] WS connected to %s\n", (char *)payload);
            g_ws_connected = true;
            send_hello();
            break;

        case WStype_TEXT: {
            // Parse gateway message
            JsonDocument doc;
            DeserializationError err = deserializeJson(doc, payload, length);
            if (err) {
                Serial.printf("[aice-pod] JSON parse error: %s\n", err.c_str());
                return;
            }
            const char *msg_type = doc["type"];
            if (!msg_type) return;

            if (strcmp(msg_type, "hello_ack") == 0) {
                Serial.printf("[aice-pod] hello_ack proto=%d\n", (int)doc["protocol_version"]);
                g_identified = true;
                start_mic();
                set_state(PodState::Listening);

            } else if (strcmp(msg_type, "led") == 0) {
                const char *state_str = doc["state"];
                if (!state_str) return;
                if (strcmp(state_str, "listening") == 0) {
                    set_state(PodState::Listening);
                } else if (strcmp(state_str, "thinking") == 0) {
                    set_state(PodState::Thinking);
                } else if (strcmp(state_str, "speaking") == 0) {
                    set_state(PodState::Speaking);
                }

            } else if (strcmp(msg_type, "audio") == 0) {
                // TTS audio from gateway — base64 PCM to play on speaker
                const char *b64 = doc["payload"];
                if (!b64) return;
                static uint8_t pcm_buf[16384];
                size_t decoded = base64_decode(b64, pcm_buf, sizeof(pcm_buf));
                if (decoded > 0) {
                    start_speaker();
                    if (!enqueue_playback(pcm_buf, decoded)) {
                        Serial.println("[aice-pod] playback queue full; dropping chunk");
                    } else {
                        g_last_tts_enqueue_ms = millis();
                        set_state(PodState::Speaking);
                    }
                }

            } else if (strcmp(msg_type, "stop_audio") == 0) {
                clear_playback_queue();
                stop_speaker();
                if (g_ws_connected && g_identified && !g_mic_running) {
                    start_mic();
                }
                set_state(PodState::Listening);
                Serial.println("[aice-pod] stop_audio received");

            } else if (strcmp(msg_type, "pong") == 0) {
                // Keepalive acknowledged; nothing to do.

            } else if (strcmp(msg_type, "error") == 0) {
                const char *code = doc["code"] | "";
                Serial.printf("[aice-pod] gateway error %s: %s\n",
                    code, (const char *)(doc["message"] | ""));
                if (strcmp(code, "unauthorized") == 0) {
                    // Token revoked or lost: forget it and enrol again after a restart.
                    pod_clear_token();
                    delay(500);
                    ESP.restart();
                }
            }
            break;
        }

        case WStype_ERROR:
            Serial.println("[aice-pod] WS error");
            set_state(PodState::Error);
            break;

        default:
            break;
    }
}

// ── Protocol helpers ──────────────────────────────────────────────────────────
static void send_hello() {
    JsonDocument doc;
    doc["type"]             = "hello";
    doc["protocol_version"] = PROTOCOL_VERSION;
    doc["device_id"]        = g_device_id;
    doc["full_duplex"]      = true;
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
    Serial.printf("[aice-pod] sent hello device_id=%s\n", g_device_id.c_str());
}

static void send_ping() {
    JsonDocument doc;
    doc["type"] = "ping";
    doc["seq"]  = g_ping_seq++;
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
}

static void send_help_button() {
    JsonDocument doc;
    doc["type"] = "help_button";
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
}

static void send_tap_activate() {
    JsonDocument doc;
    doc["type"] = "tap_activate";
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
}

// ── Microphone and speaker ────────────────────────────────────────────────────
static void start_mic() {
    if (g_mic_running) return;
    if (!board_mic_start()) {
        set_state(PodState::Error);
        return;
    }
    g_mic_running = true;
    Serial.println("[aice-pod] mic started");
}

static void stop_mic() {
    if (!g_mic_running) return;
    board_mic_stop();
    g_mic_running = false;
    Serial.println("[aice-pod] mic stopped");
}

static void start_speaker() {
    board_speaker_start();
}

static void stop_speaker() {
    board_speaker_stop();
}

// ── Audio capture → WebSocket ─────────────────────────────────────────────────
static void capture_and_send_audio() {
    static int16_t pcm_buf[SAMPLES_PER_FRAME];
    static int16_t ref_buf[SAMPLES_PER_FRAME];
    // Send every frame that is ready: a speaker write can hold the loop for a
    // whole 64 ms chunk, longer than one 40 ms frame.
    for (int sent = 0; sent < MAX_FRAMES_PER_LOOP; sent++) {
        if (!board_mic_read(pcm_buf, ref_buf, SAMPLES_PER_FRAME)) {
            return;
        }
        JsonDocument doc;
        doc["type"]      = "audio";
        doc["payload"]   = base64_encode(reinterpret_cast<const uint8_t *>(pcm_buf), BYTES_PER_FRAME);
        doc["reference"] = base64_encode(reinterpret_cast<const uint8_t *>(ref_buf), BYTES_PER_FRAME);
        String out;
        serializeJson(doc, out);
        g_ws.sendTXT(out);
    }
}

// ── TTS playback ──────────────────────────────────────────────────────────────
static void play_audio_frame(const uint8_t *pcm_bytes, size_t len) {
    if (!pcm_bytes || len < sizeof(int16_t)) return;

    // Network TTS arrives as mono PCM16.
    const int16_t *mono = reinterpret_cast<const int16_t *>(pcm_bytes);
    size_t samples = len / sizeof(int16_t);
    int16_t mono_buf[256];

    while (samples > 0) {
        size_t chunk = samples > 256 ? 256 : samples;
        int32_t peak = 0;
        for (size_t i = 0; i < chunk; i++) {
            int32_t a = mono[i];
            if (a < 0) a = -a;
            if (a > peak) peak = a;
        }
        int32_t desired_q10 = 1024; // 1.0x
        if (peak > 0) {
            desired_q10 = (POD_TTS_TARGET_PEAK * 1024) / peak;
            if (desired_q10 < POD_TTS_GAIN_Q10_MIN) desired_q10 = POD_TTS_GAIN_Q10_MIN;
            if (desired_q10 > POD_TTS_GAIN_Q10_MAX) desired_q10 = POD_TTS_GAIN_Q10_MAX;
        }
        // Smoothing avoids rapid per-chunk pumping that sounds fuzzy.
        if (desired_q10 < g_tts_gain_q10) {
            // Reduce gain quickly on loud chunks (protect from clipping).
            g_tts_gain_q10 = (g_tts_gain_q10 * 3 + desired_q10) / 4;
        } else {
            // Raise gain slowly on quiet chunks (avoid breathing/noise lift).
            g_tts_gain_q10 = (g_tts_gain_q10 * 15 + desired_q10) / 16;
        }
        for (size_t i = 0; i < chunk; i++) {
            int32_t s = mono[i];
            s = (s * g_tts_gain_q10) / 1024;
            if (s > 32767) {
                s = 32767;
            } else if (s < -32768) {
                s = -32768;
            }
            mono_buf[i] = (int16_t)s;
        }
        board_speaker_write(mono_buf, chunk);
        mono += chunk;
        samples -= chunk;
    }
}

static bool enqueue_playback(const uint8_t *pcm_bytes, size_t len) {
    if (len == 0) return true;
    if (g_playback_count >= POD_PLAYBACK_QUEUE_SLOTS) return false;
    PlaybackChunk &slot = g_playback_queue[g_playback_tail];
    const size_t n = len > POD_PLAYBACK_MAX_CHUNK_BYTES ? POD_PLAYBACK_MAX_CHUNK_BYTES : len;
    memcpy(slot.data, pcm_bytes, n);
    slot.len = n;
    g_playback_tail = (g_playback_tail + 1) % POD_PLAYBACK_QUEUE_SLOTS;
    g_playback_count++;
    return true;
}

static void playback_drain_once() {
    if (g_playback_count == 0) {
        if (g_state == PodState::Speaking && g_ws_connected) {
            if ((millis() - g_last_tts_enqueue_ms) >= TTS_END_GRACE_MS) {
                stop_speaker();
                if (g_identified && !g_mic_running) {
                    start_mic();
                }
                set_state(PodState::Listening);
            }
        }
        return;
    }
    PlaybackChunk &slot = g_playback_queue[g_playback_head];
    play_audio_frame(slot.data, slot.len);
    g_playback_chunks_played++;
    if ((g_playback_chunks_played % 20) == 1) {
        Serial.printf(
            "[aice-pod] playing chunk #%lu (%u bytes), queued=%u\n",
            (unsigned long)g_playback_chunks_played,
            (unsigned int)slot.len,
            (unsigned int)g_playback_count
        );
    }
    g_playback_head = (g_playback_head + 1) % POD_PLAYBACK_QUEUE_SLOTS;
    g_playback_count--;
}

static void clear_playback_queue() {
    g_playback_head = 0;
    g_playback_tail = 0;
    g_playback_count = 0;
}

// ── Base64 ────────────────────────────────────────────────────────────────────
static const char B64_TABLE[] =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

static String base64_encode(const uint8_t *data, size_t len) {
    String out;
    out.reserve(((len + 2) / 3) * 4 + 1);
    for (size_t i = 0; i < len; i += 3) {
        uint32_t b = (uint32_t)data[i] << 16;
        if (i + 1 < len) b |= (uint32_t)data[i + 1] << 8;
        if (i + 2 < len) b |= (uint32_t)data[i + 2];
        out += B64_TABLE[(b >> 18) & 0x3F];
        out += B64_TABLE[(b >> 12) & 0x3F];
        out += (i + 1 < len) ? B64_TABLE[(b >> 6) & 0x3F] : '=';
        out += (i + 2 < len) ? B64_TABLE[(b >> 0) & 0x3F] : '=';
    }
    return out;
}

static int b64_val(char c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '+') return 62;
    if (c == '/') return 63;
    return -1;
}

static size_t base64_decode(const char *src, uint8_t *dst, size_t dst_max) {
    size_t out = 0;
    for (; *src && src[1]; src += 4) {
        int a = b64_val(src[0]);
        int b = b64_val(src[1]);
        int c = (src[2] && src[2] != '=') ? b64_val(src[2]) : 0;
        int d = (src[3] && src[3] != '=') ? b64_val(src[3]) : 0;
        if (a < 0 || b < 0) break;
        if (out + 1 > dst_max) break;
        dst[out++] = (uint8_t)((a << 2) | (b >> 4));
        if (src[2] && src[2] != '=' && out < dst_max)
            dst[out++] = (uint8_t)((b << 4) | (c >> 2));
        if (src[3] && src[3] != '=' && out < dst_max)
            dst[out++] = (uint8_t)((c << 6) | d);
    }
    return out;
}
