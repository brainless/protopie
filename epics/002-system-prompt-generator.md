# Epic 002: System Prompt Generator

**Status:** Not started
**Goal:** Build a deterministic, unit-testable system-prompt generator, and wire `evals record` to call it instead of accepting free-text system prompts, so system-prompt changes are as evaluable as the prompts they wrap.

---

## Introduction

### Work Context

**Problem:** `evals record` currently sends the raw `--prompt` text with no system prompt or stack framing at all (`evals/README.md`), which is why the `top-navigation` recording came back as vanilla HTML/CSS/JS instead of anything SolidJS-shaped — the model was never told what it's generating for. protopie needs a system prompt to fix that, but the prompt itself is not static: the v1 tech stack is fixed (SolidJS + Vite + TS + Tailwind + DaisyUI + Solid Router, unlikely to change soon), but the *feature* context — what the project already has, from recent brain-dumps and the changes they produced — must be surfaced and will keep growing. A generator that assembles both needs the same record-once/test-many discipline epic 001 built for raw LLM calls, or every tweak to it is untested guesswork.

**Goal of this epic:** A pure, deterministic function — no LLM call — that takes structured inputs (the fixed tech-stack description, plus a project's recent-changes history) and returns a system prompt string. Unit/golden-test it directly in Rust, with no fixture files, no database, and no network. Then extend `evals record` to call this generator (instead of taking a free-text system prompt) so every recording exercises the real thing, and add at least one new recording proving the generated system prompt measurably changes model output versus the no-context baseline already checked in (`top-navigation`).

**What we are NOT deciding:**
- **Where recent-changes data actually comes from.** The generator takes already-assembled history structs (e.g. `Vec<BrainDump>`, later `Vec<CommitSummary>`) as plain function arguments; it never reads a file or a database itself. Two future sources feed those structs and are both explicitly deferred:
  - **Brain-dump text** — from SQLite (the `store` crate, per `PRD.md`/`DEVELOP.md`). No checked-in history fixture and no test-only database — the generator's tests construct history structs directly in test code, in memory. Building any part of `store` is out of scope here.
  - **Code-level changes** — from `git log`, which requires a deterministic-commit feature (auto-commit after codegen + a clean `npm run build`) that does not exist yet. Noted here as the intended future source; not built in this epic.
- SQLite/FTS/vector retrieval (`store` crate) — still deferred per `PRD.md` §2/§8.
- Prompt-to-task splitting / the PO agent contract (`PRD.md` §7.2) — still deferred to whichever epic follows this one.
- Any change to the epic-001 recording format (`prompt.toml` / `response.json`) beyond adding a field for the generated system prompt — the v0 shape otherwise stays as-is.
- The `agents` crate's full scope from `DEVELOP.md` — this epic only needs the generator function, not single-shot generation strategies, response parsing, or `llm-sdk` call wiring beyond what `evals record` already has.

**Output:** A generator function living in a small, clearly-scoped module (crate placement TBD at implementation time — likely inside `evals` initially, or a minimal new module, not the full `agents` crate from `DEVELOP.md` §2), unit-tested deterministically with in-memory-constructed history inputs; `evals record` updated to call it and persist the generated system prompt alongside each recording; at least one new recording (e.g. re-running the `top-navigation` prompt through the generator) with `assertions.toml` checks proving the output is SolidJS-shaped (contains signals like `createSignal`, `.tsx`, Solid Router imports) and no longer vanilla HTML (no `<!DOCTYPE html>` / raw DOM-mutating `<script>`).

---

## Reference sources

| System | Local path | Why it matters |
|---|---|---|
| **protopie intent** | `PRD.md`, `DEVELOP.md` | Tech stack is fixed for v1 (§"Stack" in `README.md`); "host assembles context, agent never goes looking for it" is the constraint this epic's generator must respect. |
| **epic 001** | `epics/001-evolution-replay-harness.md` | Recording format, fixture convention, and the `evals record`/`test` CLI this epic extends rather than replaces. |
| **evals crate** | `evals/src/record.rs`, `evals/src/main.rs` | Current `record` implementation — no system prompt support yet; this is what gets extended. |
| **top-navigation recording** | `evals/recordings/top-navigation/` | The no-context baseline this epic's new recording is compared against. |

---

## Design

### Where things live

The generator is a pure function:

```
fn generate_system_prompt(stack: &TechStack, recent_changes: &[ChangeSummary]) -> String
```

- `TechStack` — a fixed constant for v1 describing the scaffold (framework, router, styling, mock-data-proxy convention). Not read from any project file; it's the same for every protopie project until the scaffold itself changes.
- `ChangeSummary` — a plain struct (brain-dump text, and later a commit summary) passed in by the caller. The generator never fetches this itself.

### Testing without fixtures or a database

- **Generator unit tests** — construct `Vec<ChangeSummary>` directly in Rust test code (e.g. `vec![ChangeSummary { text: "added a top nav".into(), .. }]`) and assert on the resulting string. No checked-in history file, no SQLite, no network — same spirit as `DEVELOP.md` §5's rule that deterministic machinery gets deterministic tests.
- **`evals record` integration** — `record` calls `generate_system_prompt` with the fixed `TechStack` and an empty (or small, inline, non-persisted) `recent_changes` list for now, since no real history source exists yet. The generated string is persisted into the recording (e.g. a `system_prompt` field in `prompt.toml`, or a separate `system_prompt.txt`) so it's inspectable and diffable in git like everything else in a recording.
- **Live comparison** — record a new case re-running the `top-navigation` prompt through the generated system prompt, and write `assertions.toml` for it checking for SolidJS-shaped output and absence of vanilla-HTML tells. This is the evidence that the generator is worth having, not just that it produces *a* string.

---

## Tasks

- [ ] Decide and document the generator's exact module/crate placement (likely inside `evals` for now, given `agents` doesn't exist yet — avoid speculatively standing up the full `agents` crate from `DEVELOP.md` §2 for one function).
- [ ] Define `TechStack` as a fixed v1 constant describing the scaffold stack.
- [ ] Define `ChangeSummary` (or similarly named) as a plain struct for one prior brain-dump's text — the minimal shape needed now, extensible later for commit-derived summaries.
- [ ] Implement `generate_system_prompt(stack, recent_changes) -> String` as a pure function.
- [ ] Write unit tests constructing `recent_changes` in memory (empty, one entry, several entries) and asserting on the generated prompt content — no fixture files, no database.
- [ ] Extend `evals record` (and the recording format) to call the generator and persist the generated system prompt alongside `prompt.toml`/`response.json`.
- [ ] Record at least one new case re-running an existing baseline prompt (e.g. `top-navigation`) through the generated system prompt.
- [ ] Write `assertions.toml` for that new recording asserting SolidJS-shaped output and the absence of vanilla-HTML tells; verify `evals test` passes deterministically.
- [ ] Update `evals/README.md` to document system-prompt generation as part of the record workflow (superseding the "record does not add a system prompt" note from epic 001).
- [ ] Note, but do not implement, the two future dependencies for real "recent changes" data: a minimal SQLite-backed brain-dump store, and a deterministic auto-commit-after-build feature feeding `git log`. Leave both as candidates for future epics.
