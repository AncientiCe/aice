# Data protection impact assessment (template)

A starting point for a property's DPIA (UK GDPR / EU GDPR Article 35) for an aice deployment. It records what the software does; the property (the controller) must complete the context, lawful basis, and risk decisions, and have them approved by its data protection officer. This template is not legal advice.

## 1. Processing overview

| Item | aice behaviour | Property to complete |
|------|----------------|----------------------|
| Purpose | Let guests, residents, or patients make spoken requests to staff from their room; page staff; remember per-stay preferences when agreed. | Confirm purpose and scope. |
| Data subjects | People in rooms with a pod; staff using the desk. | Include visitors? |
| Audio | Microphone audio is streamed from the pod only while it is not muted. The bridge forwards speech segments to the backend; speech-to-text runs on-prem. Audio is **not stored** by aice. | Confirm no other system records it. |
| Transcripts and answers | Used to classify the request and create a ticket. Stored in Memory Palace **only** for a stay with memory consent, in that stay's wing. | Decide retention. |
| Tickets | Room, request type, arguments, status, timestamps in `property.sqlite`. | Retention period for tickets. |
| Audit log | Staff actions, logins, pod and stay changes, append-only. | Retention period; who may read it (supervisors). |
| Staff accounts | Username, argon2 password hash, role. | Joiner/leaver process (`user add/remove`). |
| Location | Everything runs on the property's host; no cloud service is required. Optional integrations (property MCP, webhooks) receive ticket data. | List every integration and its processor. |

## 2. Lawful basis and consent

- Voice requests to staff: typically legitimate interests or contract (hotels); care and health settings need the special-category condition for health data where requests reveal health.
- Voice memory across a stay: aice records **consent per stay** (`memory_consent`); without it nothing is remembered. Care homes and wards default to "not agreed"; hotels default to "agreed" through booking terms — change `memory_consent_default` if your terms do not cover it.
- Withdrawal: staff turn memory off for the stay at any time; with `memory_retention` `wipe_on_close` the stay's memory is deleted at checkout.

## 3. Necessity and proportionality

| Control | Where |
|---------|-------|
| Privacy mute (double tap, magenta LED) | Pod firmware |
| Memory scoped to one stay; next occupant cannot hear it | Architecture section 26 |
| Retention: keep / archive N days / wipe at checkout | `memory_retention` |
| Encryption in transit (property CA, TLS everywhere) | Architecture section 24 |
| Encryption at rest | **Operator control:** full-disk encryption on the host (FileVault, BitLocker, LUKS) |
| Access control: staff roles, lockout, audit | Architecture section 24 |
| No manual interpretation of speech outside the LLM classifier | AGENTS rule 10 |

## 4. Risks (fill in likelihood, severity, residual risk, and owner)

| Risk | Mitigations in aice | Property actions |
|------|---------------------|------------------|
| A guest's memory is heard by the next guest | Stay-scoped wings, per-turn stay check, consent gate | Train check-out discipline |
| Unauthorised access to requests | TLS, staff login, service and device tokens | Network segmentation; strong passwords |
| Host theft | — | Full-disk encryption, locked room |
| Pod placed in the wrong room | Supervisor assigns rooms on the desk; audit log | Labelled pods; check after moves |
| Always-listening concern | Mute, LED states, no audio storage | Room notice; opt-out on request |
| Request missed | Alerts, SLA escalation, help button, offline pod tickets | Tested alert channels; staffing |
| Integration leaks data | Integrations are opt-in | DPAs with processors |

## 5. Sign-off

| Role | Name | Decision | Date |
|------|------|----------|------|
| DPO | | | |
| Controller | | | |
