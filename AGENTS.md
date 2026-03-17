# Agent Rules

When working on this codebase, follow these rules on every task.

---

## 1. Test-Driven Development (TDD)

- **Write behavioural tests first.** Define the expected behaviour in tests before implementing.
- **See them fail.** Run the test suite and confirm the new tests fail (red).
- **Implement.** Write the minimum code to make the tests pass.
- **See them pass.** Run the test suite and confirm all tests pass (green).

Do not implement behaviour without a failing test that defines it.

---

## 2. Quality Gates on Every Task

Before considering a task done, ensure all of the following pass:

- **`cargo fmt`** — code is formatted.
- **`cargo clippy`** — no clippy warnings or errors.
- **`cargo audit`** — no known security advisories in dependencies.
- **Tests** — full test suite passes (e.g. `cargo test`).
- **Pipelines pass** - check pipeline steps that can be run locally and make sure they pass

Fix any failure before marking the task complete.

---

## 3. Architecture Documentation for User-Facing Features

- Every **new feature that affects user behaviour** must have a diagram in **`docs/architecture/README.md`**.
- Add a Mermaid diagram (or equivalent) that shows the flow, components, or context of the feature.
- Include a short **Purpose** and **Notes** (inputs, outputs, failure paths) for the new section.
- Follow the existing style in `docs/architecture/README.md` (numbered sections, flowchart/sequenceDiagram, behaviour ownership where relevant).

---

## 4. No Plan Markdown Files

- **Do not create `.md` files for plans** (e.g. `PLAN.md`, `TODO.md`, task plans).
- Create markdown only for **documentation** (architecture, API, runbooks, etc.) when necessary.
- Keep planning in conversation, tickets, or code comments—not as standalone plan documents in the repo.

---



## 5. Whole-System Awareness

- This is a **complete system**; every change can have consequences elsewhere.
- Before changing behaviour, consider: callers, storage, API contracts, sync, outbox, and observability.
- When adding or modifying endpoints, types, or flows, check for impact on:
  - Northbound (POS/MPOS), southbound (HQ), local storage, and print.
- Update `docs/architecture/README.md` and related docs when you add or change user-facing behaviour.

---

## 6. Observability and Metrics

- **Every new feature must include metrics.** Instrument new code paths with counters, gauges, histograms, or timers as appropriate.
- **If you touch an existing feature that lacks metrics, add them.** Do not leave touched code paths unobserved.
- At minimum, instrument:
  - **Counts** — how many times an operation is invoked (e.g. requests, commands, events processed).
  - **Errors** — how many times an operation fails, labelled by error kind where possible.
  - **Latency** — how long critical operations take (e.g. database queries, external calls, sync steps).
- Use the existing metrics infrastructure in the codebase (e.g. `metrics` crate, Prometheus counters/histograms). Do not invent a parallel mechanism.
- Metric names must follow the existing naming convention: `<subsystem>_<operation>_<unit>` (e.g. `sync_push_duration_seconds`, `orders_created_total`).
- Do not merge a feature without verifiable metric emission — confirm in tests or by inspection that the metric is recorded on the relevant code path.

---

## 7. No Unused Variables or Dead Code

- **No unused variables.** Every declared variable must be used; remove or replace with `_` if intentionally unused in Rust.
- **No dead code.** Remove unreachable functions, branches, types, and imports — do not leave them commented out or hidden behind `#[allow(dead_code)]`.
- Treat compiler warnings for unused items as errors: they must be resolved before a task is complete.

---

## 8. No Mocks

- **Do not use mocks** in tests (e.g. mock objects, mock servers, or mock crates).
- Prefer **real implementations**, **integration tests** with real services, or **test doubles** (fakes, stubs) that are explicit and minimal.
- Tests must exercise real behaviour where practical; avoid substituting dependencies with mocks that hide integration or behaviour.

---

## 9. No Placeholders

- **No placeholders. Ever.** Do not leave `todo!()`, `unimplemented!()`, stub returns, or “coming soon” code in the codebase.
- Deliver **only real implementations**. Every code path that is committed must do the real work or explicitly fail in a defined way (e.g. return `Err`, not panic with “unimplemented”).
- If a feature is not ready, do not merge it; do not merge placeholder code.

---

## Quick Reference

| Rule | Action |
|------|--------|
| TDD | Tests first → see fail → implement → see pass |
| Quality | `cargo fmt` \| `cargo clippy` \| `cargo audit` \| `cargo test` |
| Docs | New user behaviour → diagram in `docs/architecture/README.md` |
| No plan files | No `.md` for plans; only real documentation |
| Observability | New feature → add metrics; touched feature without metrics → add them |
| No dead code | No unused variables, dead code, or `#[allow(dead_code)]` |
| No mocks | No mocks; use real impls, integration tests, or explicit test doubles |
| No placeholders | No placeholders ever; only real implementations |
| System impact | Consider callers, storage, API, sync, outbox, observability |
