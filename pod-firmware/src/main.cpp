/**
 * ATOM Echo pod firmware — full production implementation.
 *
 * Hardware (M5Stack ATOM Echo, ESP32-PICO-D4):
 *   PDM mic  : SPM1423 — CLK G33 / DATA G23 (PDM digital mic, not I²S input)
 *   I²S spk  : NS4168  — DOUT G22 / BCLK G19 / LRCK G33
 *   LED      : SK6812 RGBW — G27
 *   Button   : G39 (active LOW) — tap to stop playback / wake
 *
 * Official PinMap: G33 = AMP LRCK and MIC CLK (shared). Single I2S0, mode-switch;
 * mic off during playback. Say "Computer stop" when listening; button to stop mid-play.
 *
 * Protocol: see crates/pod-protocol/src/lib.rs
 *   Pod → Gateway  : Hello, Audio, Identify, TapActivate, Ping
 *   Gateway → Pod  : HelloAck, Audio, StopAudio, Led, Pong, Error
 *
 * Audio path:
 *   PDM mic → I2S0 RX PDM (16 kHz, 16-bit, mono) → WebSocket Audio frames
 *   TTS playback: Gateway Audio (base64 PCM) → queue → I2S0 TX → speaker
 */

#include <Arduino.h>
#include <WiFi.h>
#include <WebSocketsClient.h>
#include <ArduinoJson.h>
#include <Adafruit_NeoPixel.h>
#include <driver/i2s.h>
#include <cstring>

#include "pod_config.h"

// ── Pin definitions (Atom-Echo PinMap SPK & MIC) ──────────────────────────────
//   G22      G19      G33        G23
//   AMP DATA AMP BCLK AMP LRCK   —
//   —        —       MIC CLK    MIC DATA
// G33 is shared (AMP LRCK + MIC CLK) → single I2S0, mode-switch; mic off during playback.
#define PIN_LED      27
#define PIN_BTN      39
#define PIN_MIC_CLK  33   // SPM1423 MIC CLK (G33, shared with AMP LRCK)
#define PIN_MIC_DATA 23   // SPM1423 MIC DATA (G23)
#define PIN_SPK_DATA 22   // NS4168 AMP DATA (G22)
#define PIN_SPK_BCLK 19   // NS4168 AMP BCLK (G19)
#define PIN_SPK_LRCK 33   // NS4168 AMP LRCK (G33)
#define PIN_SPK_EN   21   // NS4168 amplifier enable

// ── I2S port assignments ──────────────────────────────────────────────────────
#define I2S_PORT_MIC  I2S_NUM_0
#define I2S_PORT_SPK  I2S_NUM_0   // same port; mode-switch (G33 shared)

// ── Audio sizing ──────────────────────────────────────────────────────────────
static constexpr int SAMPLES_PER_FRAME = PDM_SAMPLE_RATE * AUDIO_FRAME_MS / 1000;
static constexpr int BYTES_PER_FRAME   = SAMPLES_PER_FRAME * sizeof(int16_t);

// DMA buffer: 2 buffers × SAMPLES_PER_FRAME samples each.
static constexpr int DMA_BUF_COUNT = 4;
static constexpr int DMA_BUF_LEN   = SAMPLES_PER_FRAME;
static constexpr size_t POD_PLAYBACK_QUEUE_SLOTS = 6;
static constexpr size_t POD_PLAYBACK_MAX_CHUNK_BYTES = 2048;
// TTS gain control tuned for intelligibility without fuzz/clipping.
static constexpr int POD_TTS_TARGET_PEAK = 22000; // leave headroom to reduce harshness
static constexpr int POD_TTS_GAIN_Q10_MIN = 768;  // 0.75x
static constexpr int POD_TTS_GAIN_Q10_MAX = 1792; // 1.75x

// ── LED ───────────────────────────────────────────────────────────────────────
// SK6812 RGBW on G27 — use NeoPixel with NEO_GRBW+NEO_KHZ800
#define NUM_LEDS 1
Adafruit_NeoPixel strip(NUM_LEDS, PIN_LED, NEO_GRBW + NEO_KHZ800);

// ── State ─────────────────────────────────────────────────────────────────────
enum class PodState {
    Booting,
    WifiConnecting,
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
enum class AudioDriverMode { None, Mic, Speaker };
static AudioDriverMode g_audio_mode = AudioDriverMode::None;
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

    // LED
    strip.begin();
    strip.setBrightness(80);
    strip.show();
    set_state(PodState::Booting);

    // Button
    pinMode(PIN_BTN, INPUT_PULLUP);
    // Enable the onboard NS4168 amp; without this, speaker stays silent.
    pinMode(PIN_SPK_EN, OUTPUT);
    digitalWrite(PIN_SPK_EN, HIGH);

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

    // WebSocket
    set_state(PodState::WsConnecting);
    g_ws.begin(GATEWAY_HOST, GATEWAY_PORT, GATEWAY_PATH);
    g_ws.onEvent(ws_event_handler);
    g_ws.setReconnectInterval(RECONNECT_BACKOFF_MS);

    Serial.printf("[aice-pod] connecting to gateway %s:%d\n", GATEWAY_HOST, GATEWAY_PORT);
}

// ── loop() ───────────────────────────────────────────────────────────────────
void loop() {
    g_ws.loop();
    led_update();

    // Button: tap-to-activate
    static bool btn_last = HIGH;
    bool btn_now = digitalRead(PIN_BTN);
    if (btn_last == HIGH && btn_now == LOW) {
        Serial.println("[aice-pod] button pressed — tap activate");
        if (g_ws_connected) {
            send_tap_activate();
        }
    }
    btn_last = btn_now;

    // Periodic ping
    if (g_ws_connected && millis() - g_last_ping_ms >= PING_INTERVAL_MS) {
        send_ping();
        g_last_ping_ms = millis();
    }

    // Mic capture only when mic is active (G33 shared → off during playback).
    if (g_ws_connected && g_mic_running &&
        (g_state == PodState::Listening || g_state == PodState::Thinking)) {
        capture_and_send_audio();
    }

    // Drain one queued TTS chunk per loop; keeps WS reader non-blocking.
    playback_drain_once();
}

// ── LED ───────────────────────────────────────────────────────────────────────
static void set_state(PodState s) {
    g_state = s;
    led_update();
}

static void led_update() {
    static uint32_t last_blink = 0;
    static bool     blink_on   = false;

    switch (g_state) {
        case PodState::Booting:
            strip.setPixelColor(0, strip.Color(20, 20, 20, 20));   // dim white
            break;
        case PodState::WifiConnecting:
        case PodState::WsConnecting:
            if (millis() - last_blink > 300) {
                blink_on = !blink_on;
                last_blink = millis();
            }
            strip.setPixelColor(0, blink_on ? strip.Color(0, 0, 60, 0) : strip.Color(0, 0, 0, 0));
            break;
        case PodState::Listening:
            strip.setPixelColor(0, strip.Color(0, 60, 0, 0));      // green
            break;
        case PodState::Thinking:
            strip.setPixelColor(0, strip.Color(60, 40, 0, 0));     // amber
            break;
        case PodState::Speaking:
            strip.setPixelColor(0, strip.Color(0, 0, 80, 0));      // blue
            break;
        case PodState::Error:
            if (millis() - last_blink > 150) {
                blink_on = !blink_on;
                last_blink = millis();
            }
            strip.setPixelColor(0, blink_on ? strip.Color(80, 0, 0, 0) : strip.Color(0, 0, 0, 0));
            break;
    }
    strip.show();
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
                    // Switch I2S0 from mic to speaker mode (official Echo pattern).
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
                if (g_ws_connected && g_identified && !g_mic_running) {
                    start_mic();
                }
                set_state(PodState::Listening);
                Serial.println("[aice-pod] stop_audio received");

            } else if (strcmp(msg_type, "pong") == 0) {
                // Keepalive acknowledged; nothing to do.

            } else if (strcmp(msg_type, "error") == 0) {
                Serial.printf("[aice-pod] gateway error %s: %s\n",
                    (const char *)doc["code"], (const char *)doc["message"]);
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
    doc["device_id"]        = DEVICE_ID;
    doc["room"]             = DEVICE_ROOM;
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
    Serial.printf("[aice-pod] sent hello device_id=%s\n", DEVICE_ID);
}

static void send_ping() {
    JsonDocument doc;
    doc["type"] = "ping";
    doc["seq"]  = g_ping_seq++;
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

// ── PDM microphone (I2S0; G33 CLK, G23 DATA — shared G33 with speaker) ────────
static void start_mic() {
    if (g_mic_running && g_audio_mode == AudioDriverMode::Mic) return;
    if (g_audio_mode == AudioDriverMode::Speaker) {
        stop_speaker();
    }

    i2s_config_t mic_cfg = {
        .mode                 = (i2s_mode_t)(I2S_MODE_MASTER | I2S_MODE_RX | I2S_MODE_PDM),
        .sample_rate          = PDM_SAMPLE_RATE,
        .bits_per_sample      = I2S_BITS_PER_SAMPLE_16BIT,
        .channel_format       = I2S_CHANNEL_FMT_ONLY_RIGHT,
        .communication_format = I2S_COMM_FORMAT_STAND_I2S,
        .intr_alloc_flags     = ESP_INTR_FLAG_LEVEL1,
        .dma_buf_count        = DMA_BUF_COUNT,
        .dma_buf_len          = DMA_BUF_LEN,
        .use_apll             = false,
        .tx_desc_auto_clear   = false,
        .fixed_mclk           = 0,
    };

    i2s_pin_config_t mic_pins = {
        .mck_io_num   = I2S_PIN_NO_CHANGE,
        .bck_io_num   = I2S_PIN_NO_CHANGE,
        .ws_io_num    = PIN_MIC_CLK,
        .data_out_num = I2S_PIN_NO_CHANGE,
        .data_in_num  = PIN_MIC_DATA,
    };

    esp_err_t err = i2s_driver_install(I2S_PORT_MIC, &mic_cfg, 0, nullptr);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] mic i2s_driver_install error: %d\n", err);
        set_state(PodState::Error);
        return;
    }
    err = i2s_set_pin(I2S_PORT_MIC, &mic_pins);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] mic i2s_set_pin error: %d\n", err);
        set_state(PodState::Error);
        return;
    }
    i2s_zero_dma_buffer(I2S_PORT_MIC);
    g_mic_running = true;
    g_audio_mode = AudioDriverMode::Mic;
    Serial.println("[aice-pod] PDM mic started");
}

static void stop_mic() {
    if (!g_mic_running && g_audio_mode != AudioDriverMode::Mic) return;
    i2s_driver_uninstall(I2S_PORT_MIC);
    g_mic_running = false;
    g_audio_mode = AudioDriverMode::None;
    Serial.println("[aice-pod] PDM mic stopped");
}

// ── I2S speaker (I2S0 mode-switch; G33 LRCK shared with mic CLK) ──────────────
static void start_speaker() {
    if (g_audio_mode == AudioDriverMode::Speaker) return;
    if (g_audio_mode == AudioDriverMode::Mic || g_mic_running) {
        stop_mic();
    }
    i2s_config_t spk_cfg = {
        .mode                 = (i2s_mode_t)(I2S_MODE_MASTER | I2S_MODE_TX),
        .sample_rate          = PDM_SAMPLE_RATE,
        .bits_per_sample      = I2S_BITS_PER_SAMPLE_16BIT,
        .channel_format       = I2S_CHANNEL_FMT_ALL_RIGHT,
        .communication_format = I2S_COMM_FORMAT_I2S,
        .intr_alloc_flags     = ESP_INTR_FLAG_LEVEL1,
        .dma_buf_count        = DMA_BUF_COUNT,
        .dma_buf_len          = DMA_BUF_LEN,
        .use_apll             = false,
        .tx_desc_auto_clear   = true,
        .fixed_mclk           = 0,
    };

    i2s_pin_config_t spk_pins = {
        .mck_io_num   = I2S_PIN_NO_CHANGE,
        .bck_io_num   = PIN_SPK_BCLK,
        .ws_io_num    = PIN_SPK_LRCK,
        .data_out_num = PIN_SPK_DATA,
        .data_in_num  = I2S_PIN_NO_CHANGE,
    };

    esp_err_t err = i2s_driver_install(I2S_PORT_SPK, &spk_cfg, 0, nullptr);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] spk i2s_driver_install error: %d\n", err);
        return;
    }
    err = i2s_set_pin(I2S_PORT_SPK, &spk_pins);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] spk i2s_set_pin error: %d\n", err);
        return;
    }
    err = i2s_set_clk(
        I2S_PORT_SPK,
        PDM_SAMPLE_RATE,
        I2S_BITS_PER_SAMPLE_16BIT,
        I2S_CHANNEL_MONO
    );
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] spk i2s_set_clk error: %d\n", err);
        return;
    }
    g_audio_mode = AudioDriverMode::Speaker;
    Serial.println("[aice-pod] I2S speaker started");
}

static void stop_speaker() {
    if (g_audio_mode != AudioDriverMode::Speaker) return;
    i2s_driver_uninstall(I2S_PORT_SPK);
    g_audio_mode = AudioDriverMode::None;
    Serial.println("[aice-pod] I2S speaker stopped");
}

// ── Audio capture → WebSocket ─────────────────────────────────────────────────
static void capture_and_send_audio() {
    static int16_t pcm_buf[SAMPLES_PER_FRAME];
    size_t bytes_read = 0;

    // Non-blocking read: only send if a full frame is ready.
    esp_err_t err = i2s_read(I2S_PORT_MIC,
                             pcm_buf,
                             BYTES_PER_FRAME,
                             &bytes_read,
                             0 /* ticks_to_wait = 0 → non-blocking */);

    if (err != ESP_OK || bytes_read < (size_t)BYTES_PER_FRAME) {
        return; // not enough data yet
    }

    // Encode as base64 for JSON transport.
    String b64 = base64_encode(reinterpret_cast<const uint8_t *>(pcm_buf), bytes_read);

    // Build and send JSON frame.
    JsonDocument doc;
    doc["type"]    = "audio";
    doc["payload"] = b64;
    String out;
    serializeJson(doc, out);
    g_ws.sendTXT(out);
}

// ── TTS playback ──────────────────────────────────────────────────────────────
static void play_audio_frame(const uint8_t *pcm_bytes, size_t len) {
    if (!pcm_bytes || len < sizeof(int16_t)) return;

    // Network TTS arrives as mono PCM16; keep mono and drive the configured slot.
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
        size_t written = 0;
        // Short timeout keeps loop responsive while draining queue.
        i2s_write(
            I2S_PORT_SPK,
            mono_buf,
            chunk * sizeof(int16_t),
            &written,
            pdMS_TO_TICKS(200)
        );
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
