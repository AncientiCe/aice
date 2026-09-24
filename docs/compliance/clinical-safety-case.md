# Clinical safety case and hazard log (starter)

For NHS deployments, DCB0129 (manufacturer) and DCB0160 (deploying organisation) require a clinical risk management process, a hazard log, and a safety case approved by a registered Clinical Safety Officer (CSO). This page records aice's design facts and a starting hazard log. **It is not a safety case and aice is not assessed or certified.** Do not deploy on a ward until the manufacturer's and the trust's CSOs have completed and signed their documents. Check whether the intended use makes aice a medical device in your jurisdiction (UK MDR 2002, EU MDR); intended use below is chosen to stay non-clinical, but that judgement belongs to the regulatory lead.

## Intended use (proposed)

Non-clinical requests from a patient bed space to ward staff: wayfinding, appointment times, porter, wheelchair, meal logistics, and asking for a nurse. aice gives no clinical advice, takes no clinical decisions, and is **not a nurse call system**; the existing nurse call remains the primary means of summoning help.

## Design facts relevant to safety

- The ward pack allows only non-clinical tools; anything else is denied by `core-policy` and escalated to staff.
- `get_a_nurse` is acknowledged within 120 s by default or escalated tier by tier; configure paging for every tier.
- The pod's long-press help button raises an escalated ticket without speech recognition or the LLM.
- A silent pod raises `device_offline` within 120 s by default.
- Ward memory defaults to "no consent" and `wipe_on_close`.
- All staff actions are audited.

## Hazard log (starter)

| ID | Hazard | Cause | Existing controls | Residual risk | Further actions |
|----|--------|-------|-------------------|---------------|-----------------|
| H1 | Patient's call for help is not heard | Pod offline, STT/LLM failure, mute on | Offline alerts, help button, LED states, nurse call unchanged | | CSO to assess; signage that nurse call is primary |
| H2 | Request is heard but not acted on | Alert channel down, staff busy | Tiered SLA escalation, desk | | Test channels each shift |
| H3 | Request misclassified | LLM error | Non-clinical allow-list, all requests become staff tickets | | Review tickets weekly |
| H4 | Patient relies on aice instead of nurse call in an emergency | Expectation | Proposed intended use and signage | | Patient information leaflet |
| H5 | Information given is wrong (wayfinding, times) | Stale data | Answers come from property systems | | Data owner per tool |
| H6 | Wrong bed space attributed | Pod in wrong location | Supervisor room assignment, audit | | Check after moves |
| H7 | Confidential information overheard | Speaker volume, shared bays | Stay-scoped memory, consent, mute | | Assess shared bays |

## Sign-off

| Role | Name | Registration | Decision | Date |
|------|------|--------------|----------|------|
| Manufacturer CSO | | | | |
| Deploying organisation CSO | | | | |
