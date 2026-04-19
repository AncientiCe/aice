# M5Stack ATOM Echo (Experimental) - pod deployment

How to build, flash, and operate the ATOM Echo pod for experimental transport testing with Aice.
Signal Pod is the primary target hardware direction.

## Hardware

| Part | Detail |
|------|--------|
| Device | M5Stack ATOM Echo (ESP32-PICO-D4) |
| Microphone | SPM1423 PDM (CLK G33 / DATA G23) |
| Speaker | NS4168 0.8W I2S (DATA G22 / BCLK G19 / LRCK G33) |
| LED | SK6812 RGBW (G27) |
| Button | G39 (active LOW) |

## Prerequisites

1. PlatformIO (`pio --version`)
2. USB serial driver for your OS (if required)
3. Firmware config in `pod-firmware/include/pod_config.h`

## Build and flash

```bash
cd /path/to/aice/pod-firmware
pio run -t upload --upload-port <serial-port>
```

(Optional monitor)

```bash
pio device monitor --project-dir pod-firmware --baud 115200
```

## Runtime verification

1. Start the pod transport alongside the backend:

```bash
cargo aice-gateway
cargo aice-backend
```

2. Confirm pod connects and streams audio.
3. Speak wake word + query and verify TTS playback returns to pod.

## Notes

- Never commit Wi-Fi credentials.
- M5Stack support is experimental; use it to validate transport behavior.
- For networking details see [Wi-Fi configuration](../network/wifi-configuration.md).
