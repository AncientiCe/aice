# Property pilot runbook

How to install, run, and roll back a pilot: one pod per room in a hotel wing (first), then a care home unit, then a hospital ward. Read the whole page before going on site.

Order matters. **Hotels first** (non-clinical, guests can phone reception); **care homes** only after the hotel pilot's metrics are healthy and the [DPIA](../compliance/dpia-template.md) is signed; **wards** only after the [clinical safety case](../compliance/clinical-safety-case.md) is approved by the trust's Clinical Safety Officer.

## 1. Host

| Item | Requirement |
|------|-------------|
| Machine | Mac mini (Apple silicon) or Linux x86_64 / aarch64 box; one host runs the pack, backend, and bridge for a pilot wing. |
| Disk encryption | **Required.** FileVault, BitLocker, or LUKS on the volume that holds `property.sqlite`, the Memory Palace database, and `tls/`. The databases are plain SQLite files; the application does not encrypt them yet. |
| Network | Pods, desk PCs, and the host on one VLAN; no inbound access from the guest Wi-Fi or the internet. |
| Accounts | A service account for the processes; staff never log in to the host. |
| Time | NTP enabled (tokens, audit times, and SLA timers depend on it). |

## 2. Install

1. Download `aice-<version>-<platform>.tar.gz` and its `.sha256` from the release page and verify: `shasum -a 256 -c aice-<version>-<platform>.tar.gz.sha256`.
2. Create `/srv/aice/property.json` from `apps/aice-hotels/property.example.json`: set `bind` (for example `0.0.0.0:8791`), `tls.hostnames` (the desk's DNS name), `extensions`, `memory_retention`, `memory_consent_default`, and `alerts` (section 5).
3. Start the pack once: `./aice-hotels /srv/aice/property.json`. It creates `service.token`, `tls/ca.pem`, `tls/server.pem`, and `property.sqlite` beside the config. Stop it.
4. Create supervisors and staff (passwords are prompted, 12+ characters): `./aice-hotels /srv/aice/property.json user add duty.manager supervisor`, then `user add <name> staff` for each desk user.
5. Issue certificates for the backend and the room bridge: `tls issue backend.pem backend.key voice.hotel.local` and `tls issue bridge.pem bridge.key voice.hotel.local`.
6. Backend `config.json`: `property.facilitator_url` = `https://desk.hotel.local:8791/mcp`, `property.service_token_file`, `property.ca_file` = `tls/ca.pem`, `service.tls` = the backend certificate, `stt.workers`, `service.max_concurrent_turns`, and `ollama_urls` from the capacity run (section 6).
7. Bridge settings in the same `config.json`: `pod_bind`, `pod_gateway.tls` = the bridge certificate, `pod_gateway.backend_url` = `wss://voice.hotel.local:8781/turns/stream`, `pod_gateway.backend_ca_file` = `tls/ca.pem`, and `tts.piper_model_path`.
8. Run all three as services (launchd or systemd) with restart on failure: `aice-hotels`, `aice-backend`, `pod-gateway`. Scrape their metrics endpoints with Prometheus ([local observability](local-observability.md)).

## 3. Pods

Follow [the pod deployment guide](../deployment/m5stack-pod.md): generate `property_trust.h`, build, flash, place one pod per room, and assign each pod its room on the desk. Label each pod with its device id and room.

## 4. Daily operation (front desk)

- **Check-in:** Stays → room, tick "Agreed to voice memory" only if the guest agreed, Check in. Returning guest: enter their earlier stay id to continue its memory (only while it still exists).
- **Check-out:** Check out on the stay. `memory_retention` decides what happens to the memory.
- **Requests:** acknowledge when someone takes a ticket, mark done when finished. Escalate if you cannot handle it.
- **Pods:** an `OFFLINE` pod or a `device_offline` ticket means a room cannot call by voice; check power and Wi-Fi now.

## 5. Alerts

Configure at least one channel for the first tier before going live, for example a webhook to the duty phone's paging gateway, and a second-tier channel for the duty manager. Test each by leaving a ticket unacknowledged past its window. Defaults: hotels 900 s for every request. Tighten for requests that matter (`request_maintenance`, `report_complaint`).

## 6. Capacity check (before go-live)

Record a typical request as raw PCM16 16 kHz mono and run, from the host:

```bash
./room-loadtest --bridge wss://voice.hotel.local:8765/ --ca /srv/aice/tls/ca.pem \
  --facilitator https://desk.hotel.local:8791 --db /srv/aice/property.sqlite \
  --rooms <pilot rooms + 50%> --turns 3 --pcm request.raw
```

Pass: no failures and p95 under 4 s. Otherwise raise `stt.workers` (memory permitting), add an Ollama host to `ollama_urls`, or pilot fewer rooms. Revoke the `loadtest-NNNN` pods afterwards (`device revoke`).

## 7. Staff training checklist

- [ ] Log in, log out, what the lockout means.
- [ ] Acknowledge, done, escalate; what the auto-escalation (actor `sla`) means.
- [ ] Check in with and without memory consent; turn memory off on request; check out.
- [ ] Recognise `help_button` and `device_offline` tickets and respond at once.
- [ ] Assign, move, and revoke a pod (supervisors).
- [ ] Where the audit log is and who may read it (supervisors).
- [ ] What guests are told: the pod listens only when the LED is green, magenta means muted, a long press calls staff.

## 8. Go-live checklist

- [ ] Disk encryption on; backups of `property.sqlite` and the palace database (encrypted, access-controlled).
- [ ] Every pilot room: pod assigned, green LED, a spoken request creates a ticket on the desk, a long press creates a `help_button` ticket.
- [ ] Alert channels tested for every tier.
- [ ] Capacity check passed.
- [ ] Guest notice in each room (what is recorded, how to mute, how to call staff, who to contact about data).
- [ ] Rollback owner named.

## 9. Pilot success metrics (review weekly)

| Metric | Target |
|--------|--------|
| `property_ticket_ack_duration_seconds` p90 | within the rule window |
| `property_sla_breaches_total` / tickets | < 5% |
| `pod_bridge_turns_total{result="answered"}` / all turns | > 95% |
| `backend_queue_rejections_total` | 0 during normal hours |
| `fleet_devices{status="offline"}` | 0 for more than 10 minutes |
| `fleet_heartbeat_missed_total` | investigated every time |

## 10. Rollback

1. Tell staff to go back to phones/call buttons only.
2. Stop `pod-gateway` (pods go blue-blinking; nothing is heard).
3. Stop `aice-backend` and the pack. Tickets, audit log, and stays remain in `property.sqlite` for review.
4. To remove all guest memory: stop the backend and delete the palace database file (it is recreated empty). To keep it, leave it on the encrypted volume under the retention you configured.
5. To retire pods: `device revoke` each id; hold the button for 5 s at power-on to wipe a pod before reuse.
