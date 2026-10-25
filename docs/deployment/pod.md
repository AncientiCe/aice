# Room pod (Waveshare ESP32-S3-AUDIO-Board)

How to build, provision, update, and operate a room pod: a microphone and speaker in each room, connected to the room bridge (`pod-gateway`) and trusted through the property facilitator.

> **Talking over an answer works, with limits.** The pod keeps its mic on while it speaks and sends its speaker signal along; the bridge cancels the echo and stops the answer when the guest speaks ([architecture 32](../architecture/README.md#32-spoken-barge-in-full-duplex-pods)). The board's small speaker limits echo cancellation, so the guest may need to speak up a little; the button always stops an answer.

> **Tested on hardware:** enrolment, room assignment, TLS to the facilitator and the bridge, spoken turns, hotel requests reaching the desk, and talking over an answer (11–14 dB of echo removed, no self-interruption over 27 s of answers, normal speech caught within a second about 60% of the time; see [Signal Pod](../hardware/signal-pod.md)). **Not yet tested on hardware:** signed updates, lost-token recovery, and factory reset.

## Hardware

| Part | Detail |
|------|--------|
| Board | Waveshare ESP32-S3-AUDIO-Board (ESP32-S3R8: 8 MB octal PSRAM, 16 MB flash, native USB) |
| Speaker DAC | ES8311 on I²S (DOUT G16) at -20 dB, so the small speaker distorts less (its echo cancels better); the amp is enabled by TCA9555 EXIO8 |
| Microphones | ES7210 4-channel ADC on I²S (DIN G15): frame position 1 is the mic (input MIC1, 37.5 dB), position 0 is the playback reference (18 dB so it never clips), positions 2–3 unused |
| I²S clocks | MCLK G12 / BCLK G13 / LRCK G14 (one port, capture and playback at the same time) |
| Control bus | I²C SDA G11 / SCL G10: ES8311 (0x18), ES7210 (0x40), TCA9555 I/O expander (0x20) |
| Buttons | TCA9555 EXIO9–11, active low (any of the three) |
| LEDs | 7 × WS2812 on G38 |

The pin map lives in `pod-firmware/src/board.cpp`. The ES8311 and ES7210 register drivers are vendored from Espressif's Apache-2.0 `esp_codec_dev` in `pod-firmware/lib/esp_codec_dev`.

## 1. Prepare the property (once)

On the facilitator host (the pack runs with `tls.mode: auto`, so it has a property CA):

```bash
aice-hotels property.json firmware keygen firmware_signing.pk8
aice-hotels property.json firmware pod-header firmware_signing.pk8 https://desk.hotel.local:8791 > pod-firmware/include/property_trust.h
aice-hotels property.json tls issue bridge.pem bridge.key voice.hotel.local
```

- `firmware_signing.pk8` signs every image. Keep it offline or in a vault; anyone with it can update every pod.
- `property_trust.h` carries the property CA, the firmware public key, and the facilitator URL. It is git-ignored.
- `bridge.pem` / `bridge.key` go under `pod_gateway.tls` in the bridge's `config.json`, with `pod_gateway.backend_url` and `pod_gateway.backend_ca_file` pointing at the backend.
- **Use host names, not IP addresses,** in the facilitator URL, `GATEWAY_HOST`, and the certificates. The pod's TLS library checks the certificate against the host name only. Without property DNS, use the host's mDNS name (`<computer name>.local`): the pod resolves `.local` names over mDNS.
- **The pod needs the time.** It sets its clock over NTP (`NTP_SERVER_1`, `NTP_SERVER_2`, default `pool.ntp.org` and `time.google.com`) before any TLS connection, because certificates are checked against the date. Point these at the property's NTP server when the pod network has no internet access.

## 2. Build and flash

```bash
cp pod-firmware/include/pod_config.example.h pod-firmware/include/pod_config.h   # set Wi-Fi and GATEWAY_HOST
python -m platformio run -d pod-firmware -t upload --upload-port <serial-port>
```

`GATEWAY_HOST` must be a name in the bridge certificate. The room is **not** set in firmware. Leave `DEVICE_ID` empty so the pod uses `pod-<wifi mac>`.

## 3. Provision in the room

1. Power the pod in its room. It joins Wi-Fi, sets its clock, creates a secret nonce on first boot, and enrols with the facilitator. The LEDs blink slowly amber.
2. A supervisor opens the desk, finds the pod under **Room devices** as `pending`, types the room, and presses **Assign room** (or runs `aice-hotels property.json device assign <id> <room>`).
3. Within 10 s the pod receives its device token, connects to the bridge over WSS, and turns green (listening).

LED states: dim white booting, blue blink connecting, slow amber blink waiting for a room, green listening, magenta muted, amber thinking, blue speaking, fast red blink error (audio hardware did not start, revoked, or enrolled with another nonce).

## Operating

- Speak a request; the bridge detects speech, the backend answers, and the pod speaks it. Talk over an answer to interrupt it, or press a button.
- **Buttons:** tap stops the answer, double tap toggles the privacy mute (magenta), a press of 1.5 s calls staff (`help_button` ticket).
- The backend reports each connected pod every 30 s. A pod silent for `device_offline_after_secs` (default 120) opens an escalated `device_offline` ticket for its room.
- **Move a pod:** assign it a new room on the desk; the next turn uses the new room.
- **Retire a pod:** press **Revoke**. The pod is refused immediately.
- **Lost token** (for example after a flash erase that kept the nonce partition): the pod asks for a new token with its nonce; the old token stops working.
- **Factory reset:** hold a button for 5 s while powering on. The pod forgets its nonce and token; revoke its old record on the desk, then assign it again.

## Updates

```bash
python -m platformio run -d pod-firmware          # bump FIRMWARE_VERSION first
aice-hotels property.json firmware publish firmware_signing.pk8 pod-firmware/.pio/build/waveshare_audio/firmware.bin 2.0.1 10
aice-hotels property.json firmware rollout 2.0.1 100    # after the 10% canary looks healthy
```

Idle pods check every 6 hours and at boot. A pod is offered the newest release when its stable cohort (hash of device id and version) is inside the rollout percentage. It downloads over HTTPS into the second app slot (the 16 MB flash holds two 6.5 MB slots), hashes while writing, and installs only if the ECDSA P-256 signature verifies; otherwise it keeps running the current image. Publishing an older build again under a new version rolls the fleet back.

Metrics: `fleet_firmware_manifest_total{result}`, `fleet_firmware_downloads_total{version}`, `fleet_devices{status}`.
