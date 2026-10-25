# Pod network configuration

How a room pod joins the property network and reaches the room bridge and the staff desk. Building, flashing and enrolling the pod is covered in the [room pod guide](../deployment/pod.md).

## What the pod connects to

| Connection | From → to | Default | Notes |
|------------|-----------|---------|-------|
| Enrolment, firmware updates | Pod → desk (`aice-hotels`) | `https://<desk host>:8791` | URL compiled into `property_trust.h` by `firmware pod-header` |
| Room audio | Pod → bridge (`pod-gateway`) | `wss://<GATEWAY_HOST>:8765/` | `pod_bind` on the bridge; `GATEWAY_HOST`, `GATEWAY_PORT`, `GATEWAY_PATH` in `pod_config.h` |
| Clock | Pod → NTP | `pool.ntp.org`, `time.google.com` | `NTP_SERVER_1`, `NTP_SERVER_2`. The time is needed before any TLS connection |
| Name lookup | Pod → property DNS or mDNS | — | `.local` names resolve over mDNS |

## Requirements

- **One network for pods and the host.** Put the pods, the desk PCs and the host on one VLAN, with no inbound access from the guest Wi-Fi or the internet ([pilot runbook](../runbooks/property-pilot.md)).
- **Host names, not IP addresses.** The pod's TLS library checks certificates against the host name only. Use a DNS name, or the host's mDNS name (`<computer name>.local`), in `GATEWAY_HOST`, in the desk URL and in the certificates (`tls issue bridge.pem bridge.key voice.hotel.local`).
- **TLS on the bridge.** With a desk configured, `pod-gateway` refuses plain WebSocket on a network bind. Set `pod_gateway.tls` to the bridge certificate.
- **Firewall.** Allow inbound TCP from the pod network to the bridge port (`8765`) and the desk port (`8791`).
- **NTP.** If the pod network has no internet access, point `NTP_SERVER_1` and `NTP_SERVER_2` at the property's NTP server.

## Bridge side (`config.json`)

```json
"pod_bind": "0.0.0.0:8765",
"pod_gateway": {
  "tls": { "cert_file": "tls/bridge.pem", "key_file": "tls/bridge.key" },
  "backend_url": "wss://voice.hotel.local:8781/turns/stream",
  "backend_ca_file": "tls/ca.pem"
}
```

## Pod side (`pod-firmware/include/pod_config.h`)

Copy `pod_config.example.h` (`pod_config.h` is git-ignored) and set:

```c
#define WIFI_SSID     "property-iot"
#define WIFI_PASSWORD "…"
#define GATEWAY_HOST  "voice.hotel.local"   // a name in the bridge certificate
#define GATEWAY_PORT  8765
```

Leave `DEVICE_ID` empty so the pod uses `pod-<wifi mac>`. Do not set a room in firmware: a supervisor assigns it on the desk.

## Validation checklist

- [ ] The bridge starts without a "refusing plain WebSocket" error and listens on `pod_bind`.
- [ ] From a laptop on the pod network, `GATEWAY_HOST` resolves and `openssl s_client -connect <GATEWAY_HOST>:8765` shows the bridge certificate signed by the property CA.
- [ ] The pod's serial log shows Wi-Fi joined, the clock set, then enrolment (slow amber blink).
- [ ] The pod appears as `pending` under **Room devices** on the desk. After **Assign room**, it turns green within 10 s.
- [ ] `pod_connections_total` rises on the bridge metrics (`:9766`).

## Troubleshooting

- **Stays on blue blink (connecting):** the bridge is unreachable or TLS failed. Check the firewall, `pod_bind`, that the host name resolves on the pod network, that NTP is reachable (certificates are checked against the date), and that `GATEWAY_HOST` matches a name in the certificate. The serial log says which.
- **Fast red blink:** the audio hardware did not start, the pod was revoked, or it enrolled with another nonce ([room pod guide](../deployment/pod.md)).
- **Stays on slow amber blink:** enrolled but no room yet. Assign it on the desk.
- **Pod disappears every few minutes:** weak Wi-Fi. Each drop opens a `device_offline` ticket after 2 minutes, so fix coverage before go-live.
