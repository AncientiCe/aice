/**
 * Waveshare ESP32-S3-AUDIO-Board (ESP32-S3R8: 8 MB octal PSRAM, 16 MB flash).
 *
 *   I²C      : SDA G11 / SCL G10 — ES8311 (DAC), ES7210 (4-ch ADC), TCA9555 (0x20)
 *   I²S      : MCLK G12 / BCLK G13 / LRCK G14 / DIN G15 (ES7210) / DOUT G16 (ES8311)
 *   Amp      : TCA9555 EXIO8, active high
 *   Buttons  : TCA9555 EXIO9–11, active low
 *   LEDs     : 7 × WS2812 on G38
 *
 * One I²S port runs capture and playback at the same time (32-bit stereo slots
 * at 16 kHz). The ES7210 packs four 16-bit channels into each frame:
 *   ch0 = playback reference (loopback of the DAC), ch1 = mic, ch2 = unused, ch3 = mic.
 * The pod sends ch1. ch0 is the echo-cancellation reference for spoken barge-in.
 */

#include "board.h"

#include <Adafruit_NeoPixel.h>
#include <Wire.h>
#include <driver/i2s.h>

#include "es7210_adc.h"
#include "es8311_codec.h"
#include "esp_codec_dev_defaults.h"
#include "pod_defaults.h"

#define PIN_I2C_SDA 11
#define PIN_I2C_SCL 10
#define PIN_MCLK    12
#define PIN_BCLK    13
#define PIN_LRCK    14
#define PIN_DIN     15
#define PIN_DOUT    16
#define PIN_LED     38
#define NUM_LEDS    7

#define TCA9555_ADDR  0x20
#define TCA_IN_PORT1  0x01
#define TCA_OUT_PORT1 0x03
#define TCA_DIR_PORT1 0x07
#define TCA_AMP_BIT   0x01  // EXIO8
#define TCA_BTN_MASK  0x0E  // EXIO9–11

#define I2S_PORT I2S_NUM_0

// Mic level: the ES7210 at its maximum analog gain. No digital gain: the echo of
// the pod's own voice must stay below full scale, because clipped echo cannot be
// cancelled (a clipped capture left 8.7 dB for even an ideal linear filter).
#ifndef POD_MIC_GAIN_DB
#define POD_MIC_GAIN_DB 37.5f
#endif
// Playback reference (DAC loopback) level: low enough that the loudest answer
// never clips it. The echo canceller learns the gain, so only headroom matters.
#ifndef POD_REF_GAIN_DB
#define POD_REF_GAIN_DB 18.0f
#endif
// Speaker level in dB relative to full scale (ES8311 DAC volume).
#ifndef POD_SPEAKER_DB
#define POD_SPEAKER_DB -20.0f
#endif

static constexpr int ADC_CHANNELS = 4;  // int16 values per I²S frame
static constexpr int REF_CHANNEL  = 0;  // DAC loopback: what the speaker played
// The mic at frame position 1 is ES7210 input MIC1 (lowering that input's gain
// lowered it). Every other input gets the reference gain: the loopback sits on
// one of them, and the others are unused.
static constexpr int MIC_ADC_INPUT = 0;  // zero-based: MIC1
static constexpr int MIC_CHANNEL  = 1;
static constexpr int SAMPLES_PER_FRAME = PDM_SAMPLE_RATE * AUDIO_FRAME_MS / 1000;

static Adafruit_NeoPixel strip(NUM_LEDS, PIN_LED, NEO_GRB + NEO_KHZ800);
static const audio_codec_if_t *g_dac = nullptr;
static const audio_codec_if_t *g_adc = nullptr;
static bool g_mic_on = false;
static bool g_speaker_on = false;
static int16_t g_rx_frame[SAMPLES_PER_FRAME * ADC_CHANNELS];
static size_t g_rx_filled = 0;  // bytes of g_rx_frame read so far

// ── TCA9555 I/O expander ──────────────────────────────────────────────────────
static bool tca_write(uint8_t reg, uint8_t value) {
    Wire.beginTransmission(TCA9555_ADDR);
    Wire.write(reg);
    Wire.write(value);
    return Wire.endTransmission() == 0;
}

static bool tca_read(uint8_t reg, uint8_t *value) {
    Wire.beginTransmission(TCA9555_ADDR);
    Wire.write(reg);
    if (Wire.endTransmission(false) != 0) return false;
    if (Wire.requestFrom((uint8_t)TCA9555_ADDR, (uint8_t)1) != 1) return false;
    *value = Wire.read();
    return true;
}

static void set_amp(bool on) {
    if (!tca_write(TCA_OUT_PORT1, on ? TCA_AMP_BIT : 0x00)) {
        Serial.println("[aice-pod] amp switch failed");
    }
}

// ── Codec register access over Wire (esp_codec_dev control interface) ─────────
// esp_codec_dev addresses are 8-bit (write) addresses.
struct WireCtrl {
    audio_codec_ctrl_if_t base;
    uint8_t addr;
};

static int wire_ctrl_open(const audio_codec_ctrl_if_t *, void *, int) {
    return ESP_CODEC_DEV_OK;
}

static bool wire_ctrl_is_open(const audio_codec_ctrl_if_t *) {
    return true;
}

static int wire_ctrl_read(const audio_codec_ctrl_if_t *ctrl, int reg, int reg_len, void *data, int data_len) {
    const WireCtrl *c = reinterpret_cast<const WireCtrl *>(ctrl);
    const uint8_t addr = c->addr >> 1;
    Wire.beginTransmission(addr);
    for (int i = reg_len - 1; i >= 0; i--) Wire.write((uint8_t)(reg >> (8 * i)));
    if (Wire.endTransmission(false) != 0) return ESP_CODEC_DEV_READ_FAIL;
    if (Wire.requestFrom(addr, (uint8_t)data_len) != data_len) return ESP_CODEC_DEV_READ_FAIL;
    uint8_t *out = static_cast<uint8_t *>(data);
    for (int i = 0; i < data_len; i++) out[i] = Wire.read();
    return ESP_CODEC_DEV_OK;
}

static int wire_ctrl_write(const audio_codec_ctrl_if_t *ctrl, int reg, int reg_len, void *data, int data_len) {
    const WireCtrl *c = reinterpret_cast<const WireCtrl *>(ctrl);
    Wire.beginTransmission((uint8_t)(c->addr >> 1));
    for (int i = reg_len - 1; i >= 0; i--) Wire.write((uint8_t)(reg >> (8 * i)));
    Wire.write(static_cast<const uint8_t *>(data), data_len);
    return Wire.endTransmission() == 0 ? ESP_CODEC_DEV_OK : ESP_CODEC_DEV_WRITE_FAIL;
}

static int wire_ctrl_close(const audio_codec_ctrl_if_t *) {
    return ESP_CODEC_DEV_OK;
}

static WireCtrl g_dac_ctrl = {
    {wire_ctrl_open, wire_ctrl_is_open, wire_ctrl_read, wire_ctrl_write, wire_ctrl_close},
    ES8311_CODEC_DEFAULT_ADDR,
};
static WireCtrl g_adc_ctrl = {
    {wire_ctrl_open, wire_ctrl_is_open, wire_ctrl_read, wire_ctrl_write, wire_ctrl_close},
    ES7210_CODEC_DEFAULT_ADDR,
};

// ── Init ──────────────────────────────────────────────────────────────────────
static bool i2s_init() {
    i2s_config_t cfg = {
        .mode                 = (i2s_mode_t)(I2S_MODE_MASTER | I2S_MODE_TX | I2S_MODE_RX),
        .sample_rate          = PDM_SAMPLE_RATE,
        .bits_per_sample      = I2S_BITS_PER_SAMPLE_32BIT,
        .channel_format       = I2S_CHANNEL_FMT_RIGHT_LEFT,
        .communication_format = I2S_COMM_FORMAT_STAND_I2S,
        .intr_alloc_flags     = ESP_INTR_FLAG_LEVEL1,
        .dma_buf_count        = 6,
        .dma_buf_len          = SAMPLES_PER_FRAME,
        .use_apll             = false,
        .tx_desc_auto_clear   = true,
        .fixed_mclk           = 0,
        .mclk_multiple        = I2S_MCLK_MULTIPLE_256,
        .bits_per_chan        = I2S_BITS_PER_CHAN_32BIT,
    };
    i2s_pin_config_t pins = {
        .mck_io_num   = PIN_MCLK,
        .bck_io_num   = PIN_BCLK,
        .ws_io_num    = PIN_LRCK,
        .data_out_num = PIN_DOUT,
        .data_in_num  = PIN_DIN,
    };
    esp_err_t err = i2s_driver_install(I2S_PORT, &cfg, 0, nullptr);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] i2s_driver_install error: %d\n", err);
        return false;
    }
    err = i2s_set_pin(I2S_PORT, &pins);
    if (err != ESP_OK) {
        Serial.printf("[aice-pod] i2s_set_pin error: %d\n", err);
        return false;
    }
    i2s_zero_dma_buffer(I2S_PORT);
    return true;
}

static bool codecs_init() {
    esp_codec_dev_sample_info_t fs = {};
    fs.bits_per_sample = 32;
    fs.channel = 2;
    fs.sample_rate = PDM_SAMPLE_RATE;

    es7210_codec_cfg_t adc_cfg = {};
    adc_cfg.ctrl_if = &g_adc_ctrl.base;
    adc_cfg.mic_selected = ES7210_SEL_MIC1 | ES7210_SEL_MIC2 | ES7210_SEL_MIC3 | ES7210_SEL_MIC4;
    g_adc = es7210_codec_new(&adc_cfg);
    if (g_adc == nullptr || g_adc->set_fs(g_adc, &fs) != ESP_CODEC_DEV_OK ||
        g_adc->enable(g_adc, true) != ESP_CODEC_DEV_OK) {
        Serial.println("[aice-pod] ES7210 (mic ADC) did not start");
        return false;
    }
    for (int ch = 0; ch < ADC_CHANNELS; ch++) {
        float gain = ch == MIC_ADC_INPUT ? POD_MIC_GAIN_DB : POD_REF_GAIN_DB;
        g_adc->set_mic_channel_gain(g_adc, ESP_CODEC_DEV_MAKE_CHANNEL_MASK(ch), gain);
    }

    es8311_codec_cfg_t dac_cfg = {};
    dac_cfg.ctrl_if = &g_dac_ctrl.base;
    dac_cfg.codec_mode = ESP_CODEC_DEV_WORK_MODE_DAC;
    dac_cfg.pa_pin = -1;  // the amp is on the I/O expander
    dac_cfg.use_mclk = false;
    g_dac = es8311_codec_new(&dac_cfg);
    if (g_dac == nullptr || g_dac->set_fs(g_dac, &fs) != ESP_CODEC_DEV_OK ||
        g_dac->enable(g_dac, true) != ESP_CODEC_DEV_OK) {
        Serial.println("[aice-pod] ES8311 (speaker DAC) did not start");
        return false;
    }
    g_dac->set_vol(g_dac, POD_SPEAKER_DB);
    return true;
}

bool board_init() {
    strip.begin();
    strip.setBrightness(80);
    strip.show();

    Wire.begin(PIN_I2C_SDA, PIN_I2C_SCL, 100000);
    // Port 1: bit 0 (amp) is an output, the button bits stay inputs.
    if (!tca_write(TCA_OUT_PORT1, 0x00) || !tca_write(TCA_DIR_PORT1, (uint8_t)~TCA_AMP_BIT)) {
        Serial.println("[aice-pod] TCA9555 I/O expander did not answer");
        return false;
    }
    return i2s_init() && codecs_init();
}

bool board_button_down() {
    uint8_t in = 0xFF;
    if (!tca_read(TCA_IN_PORT1, &in)) return false;
    return (in & TCA_BTN_MASK) != TCA_BTN_MASK;
}

void board_led_set(uint8_t r, uint8_t g, uint8_t b) {
    for (int i = 0; i < NUM_LEDS; i++) strip.setPixelColor(i, strip.Color(r, g, b));
    strip.show();
}

// ── Microphone ────────────────────────────────────────────────────────────────
static void drain_rx() {
    size_t got = 0;
    do {
        got = 0;
        i2s_read(I2S_PORT, g_rx_frame, sizeof(g_rx_frame), &got, 0);
    } while (got == sizeof(g_rx_frame));
    g_rx_filled = 0;
}

bool board_mic_start() {
    if (g_mic_on) return true;
    // Drop audio captured while the mic was off (it may hold the pod's own answer).
    drain_rx();
    g_mic_on = true;
    return true;
}

void board_mic_stop() {
    g_mic_on = false;
}

bool board_mic_read(int16_t *pcm, int16_t *reference, size_t samples) {
    if (!g_mic_on || samples != (size_t)SAMPLES_PER_FRAME) return false;
    uint8_t *raw = reinterpret_cast<uint8_t *>(g_rx_frame);
    size_t got = 0;
    i2s_read(I2S_PORT, raw + g_rx_filled, sizeof(g_rx_frame) - g_rx_filled, &got, 0);
    g_rx_filled += got;
    if (g_rx_filled < sizeof(g_rx_frame)) return false;
    g_rx_filled = 0;
    for (size_t i = 0; i < samples; i++) {
        pcm[i] = g_rx_frame[ADC_CHANNELS * i + MIC_CHANNEL];
        reference[i] = g_rx_frame[ADC_CHANNELS * i + REF_CHANNEL];
    }
    return true;
}

// ── Speaker ───────────────────────────────────────────────────────────────────
bool board_speaker_start() {
    if (!g_speaker_on) {
        set_amp(true);
        g_speaker_on = true;
    }
    return true;
}

void board_speaker_stop() {
    if (!g_speaker_on) return;
    set_amp(false);
    g_speaker_on = false;
}

void board_speaker_write(const int16_t *pcm, size_t samples) {
    if (!g_speaker_on) return;
    // Mono PCM16 → both 32-bit slots, sample in the top 16 bits.
    int32_t frames[2 * 128];
    while (samples > 0) {
        size_t n = samples > 128 ? 128 : samples;
        for (size_t i = 0; i < n; i++) {
            int32_t s = (int32_t)pcm[i] << 16;
            frames[2 * i] = s;
            frames[2 * i + 1] = s;
        }
        size_t written = 0;
        // Short timeout keeps the loop responsive while draining the queue.
        i2s_write(I2S_PORT, frames, n * 2 * sizeof(int32_t), &written, pdMS_TO_TICKS(200));
        pcm += n;
        samples -= n;
    }
}
