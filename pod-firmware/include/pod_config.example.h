// Example pod configuration. Copy to pod_config.h (git-ignored) and fill in.
//
// Values can also come from platformio.ini build_flags, e.g.
//   build_flags = -DWIFI_SSID='"MyNetwork"' -DWIFI_PASSWORD='"secret"'

#pragma once

// ── Wi-Fi ────────────────────────────────────────────────────────────────────
#ifndef WIFI_SSID
#define WIFI_SSID "CHANGE_ME"
#endif

#ifndef WIFI_PASSWORD
#define WIFI_PASSWORD "CHANGE_ME"
#endif

// ── Room bridge (pod-gateway) ────────────────────────────────────────────────
// Host name must match a name in the bridge's certificate (pod_gateway.tls).
#ifndef GATEWAY_HOST
#define GATEWAY_HOST "voice.property.local"
#endif

#ifndef GATEWAY_PORT
#define GATEWAY_PORT 8765
#endif

#ifndef GATEWAY_PATH
#define GATEWAY_PATH "/"
#endif

// ── Identity ─────────────────────────────────────────────────────────────────
// Leave empty to use "pod-<wifi mac>". The room is set by a supervisor on the
// staff desk, never on the pod.
#ifndef DEVICE_ID
#define DEVICE_ID ""
#endif
