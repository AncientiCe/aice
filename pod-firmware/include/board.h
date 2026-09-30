// Pod hardware: Waveshare ESP32-S3-AUDIO-Board (src/board.cpp).
//
// Audio is 16 kHz mono PCM16 in both directions, the `audio` frame format of
// the room bridge.

#pragma once

#include <Arduino.h>

// Pins, codecs, and the LED ring. Returns false when the audio hardware did not answer.
bool board_init();

// True while any of the three user buttons is held.
bool board_button_down();

// Sets every LED on the ring to one colour.
void board_led_set(uint8_t r, uint8_t g, uint8_t b);

// Microphone capture. Reads never block: board_mic_read returns true only when
// a full frame of `samples` samples is ready.
bool board_mic_start();
void board_mic_stop();
bool board_mic_read(int16_t *pcm, size_t samples);

// Speaker playback. board_speaker_write blocks until the samples are queued to
// the audio hardware (bounded by a short timeout).
bool board_speaker_start();
void board_speaker_stop();
void board_speaker_write(const int16_t *pcm, size_t samples);
