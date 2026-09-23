# M5Stack ATOM Echo room pod

How to build, provision, update, and operate an ATOM Echo as a room pod: a microphone and speaker in each room, connected to the room bridge (`pod-gateway`) and trusted through the property facilitator.

> The firmware compiles in this repository (`python -m platformio run -d pod-firmware`) but the provisioning, TLS, and update paths have not yet been exercised on hardware. Test on one pod before a rollout.

## Hardware

| Part | Detail |
|------|--------|
| Device | M5Stack ATOM Echo (ESP32-PICO-D4) |
| Microphone | SPM1423 PDM (CLK G33 / DATA G23) |
| Speaker | NS4168 0.8W I2S (DATA G22 / BCLK G19 / LRCK G33) |
| LED | SK6812 RGBW (G27) |
| Button | G39 (active LOW) |

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

## 2. Build and flash

```bash
cp pod-firmware/include/pod_config.example.h pod-firmware/include/pod_config.h   # set Wi-Fi and GATEWAY_HOST
python -m platformio run -d pod-firmware -t upload --upload-port <serial-port>
```

`GATEWAY_HOST` must be a name in the bridge certificate. The room is **not** set in firmware.

## 3. Provision in the room

1. Power the pod in its room. It joins Wi-Fi, creates a secret nonce on first boot, and enrols with the facilitator. The LED blinks slowly amber.
2. A supervisor opens the desk, finds the pod under **Room devices** as `pending` (its id is `pod-<wifi mac>`), types the room, and presses **Assign room** (or runs `aice-hotels property.json device assign <id> <room>`).
3. Within 10 s the pod receives its device token, connects to the bridge over WSS, and turns green (listening).

LED states: dim white booting, blue blink connecting, slow amber blink waiting for a room, green listening, amber thinking, blue speaking, fast red blink error (revoked or enrolled with another nonce).

## Operating

- Speak a request; the bridge detects speech, the backend answers, and the pod speaks it. The mic is off while the pod speaks; press the button to stop an answer.
- The backend reports each connected pod every 30 s. A pod silent for `device_offline_after_secs` (default 120) opens an escalated `device_offline` ticket for its room.
- **Move a pod:** assign it a new room on the desk; the next turn uses the new room.
- **Retire a pod:** press **Revoke**. The pod is refused immediately.
- **Lost token** (for example after a flash erase that kept the nonce partition): the pod asks for a new token with its nonce; the old token stops working.
- **Factory reset:** hold the button for 5 s while powering on. The pod forgets its nonce and token; revoke its old record on the desk, then assign it again.

## Updates

```bash
python -m platformio run -d pod-firmware          # bump FIRMWARE_VERSION first
aice-hotels property.json firmware publish firmware_signing.pk8 pod-firmware/.pio/build/atom_echo/firmware.bin 2.0.1 10
aice-hotels property.json firmware rollout 2.0.1 100    # after the 10% canary looks healthy
```

Idle pods check every 6 hours and at boot. A pod is offered the newest release when its stable cohort (hash of device id and version) is inside the rollout percentage. It downloads over HTTPS, hashes while writing, and installs only if the ECDSA P-256 signature verifies; otherwise it keeps running the current image. Publishing an older build again under a new version rolls the fleet back.

Metrics: `fleet_firmware_manifest_total{result}`, `fleet_firmware_downloads_total{version}`, `fleet_devices{status}`.
