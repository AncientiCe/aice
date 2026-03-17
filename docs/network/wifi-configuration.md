# Wi‑Fi and network configuration

How pod devices connect to your local Wi-Fi and to the pod gateway.
Signal Pod is the target device path; M5Stack configuration here is experimental.

## Requirements

- **Same LAN:** The M5Stack pod and the machine running **pod-gateway** must be on the same local network (same Wi‑Fi or same wired LAN). The pod connects to the gateway over TCP (WebSocket) using the gateway host’s IP and port.
- **Gateway reachable:** The host running `pod-gateway` must bind on an address the pod can reach. Default is `0.0.0.0:8765` (all interfaces, port 8765). Firewall rules must allow inbound TCP on that port from the LAN.

## Gateway side (`config.json`)

On the machine that runs the gateway, `config.json` includes:

```json
"pod_bind": "0.0.0.0:8765"
```

- **`0.0.0.0`** — Listen on all interfaces so the pod can connect using the host’s LAN IP.
- **Port** — Use a single port (e.g. `8765`). The pod will connect to `GATEWAY_HOST:8765`.

If the gateway runs on `192.168.1.10`, the pod should be configured to connect to `192.168.1.10:8765` (or whatever port you set).

## Pod side (firmware)

The pod firmware must be configured with:

1. **Wi‑Fi:** Your LAN’s **SSID** and **password** so the device joins the same network as the gateway.
2. **Gateway address:** The **host** (IP or hostname) and **port** of the machine running `pod-gateway` (e.g. `192.168.1.10` and `8765`).

How you set these depends on the firmware:

- **Compile-time:** #defines or build flags (SSID, password, gateway host, port).
- **Runtime:** Config file on SD card, or a provisioning flow (e.g. captive portal, BLE, or serial commands).

Avoid committing real Wi‑Fi passwords to the repo; use env vars, local config, or a secure provisioning flow.

## Finding the gateway host

On the machine running the gateway:

- Run `ip route get 1` and use the source IP shown for your LAN interface.
- If `ip` is unavailable, use `hostname -I` and pick the first non-loopback IPv4 address.

Use that IP as the gateway host in the pod configuration (e.g. `192.168.1.10`).

## Validation checklist

- [ ] Gateway runs and binds: `cargo aice-gateway`.
- [ ] Gateway health endpoint responds: `curl -fsS http://127.0.0.1:8780/healthz` (adjust host/port if changed).
- [ ] `config.json` has `pod_bind` set (e.g. `0.0.0.0:8765`).
- [ ] Pod firmware has correct Wi‑Fi SSID/password and gateway host/port.
- [ ] Pod and gateway host are on the same LAN.
- [ ] Firewall allows inbound TCP on the gateway port from the LAN.
- [ ] After flashing/reset, pod joins Wi‑Fi and opens a WebSocket to the gateway; gateway receives `hello/identify`.
- [ ] Pod keepalive works (`ping` from pod, `pong` from gateway).

## Troubleshooting

- **Pod never connects** — Verify SSID/password; ping the gateway host from another device on the same Wi‑Fi; ensure the gateway is running and listening on `0.0.0.0:8765` (or your chosen port).
- **Connection refused** — Port not open: check firewall and that `pod_bind` matches (host and port).
- **Wrong gateway IP** — If the host has multiple IPs, use the one on the same subnet as the pod (usually the LAN interface). Avoid `127.0.0.1` for the pod; that is local to the host only.

For building and flashing the pod, see [M5Stack pod deployment](../deployment/m5stack-pod.md).
