// Defaults for settings a pod_config.h may leave out.

#pragma once

#ifndef DEVICE_ID
#define DEVICE_ID ""
#endif

// Reported to the facilitator; bump for every published image.
#ifndef FIRMWARE_VERSION
#define FIRMWARE_VERSION "2.0.0"
#endif

#ifndef PROTOCOL_VERSION
#define PROTOCOL_VERSION 1
#endif

#ifndef PDM_SAMPLE_RATE
#define PDM_SAMPLE_RATE 16000
#endif

#ifndef AUDIO_FRAME_MS
#define AUDIO_FRAME_MS 40
#endif

#ifndef WIFI_TIMEOUT_MS
#define WIFI_TIMEOUT_MS 20000
#endif

#ifndef PING_INTERVAL_MS
#define PING_INTERVAL_MS 5000
#endif

#ifndef RECONNECT_BACKOFF_MS
#define RECONNECT_BACKOFF_MS 3000
#endif

// How often to ask the facilitator whether a room has been assigned.
#ifndef ENROLL_POLL_MS
#define ENROLL_POLL_MS 10000
#endif

// How often an idle pod checks for a firmware update.
#ifndef OTA_CHECK_INTERVAL_MS
#define OTA_CHECK_INTERVAL_MS (6UL * 60UL * 60UL * 1000UL)
#endif

// Hold the button this long while booting to forget the pod's identity.
#ifndef FACTORY_RESET_HOLD_MS
#define FACTORY_RESET_HOLD_MS 5000
#endif
