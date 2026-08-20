# Epic 001: Evolution Replay Harness

**Status:** Not started
**Goal:** Build the smallest reproducible harness that can attempt three successive brain-dumps against one SolidJS fixture through a live local LLM, record the complete attempt, and replay the same responses without an LLM.

---

## Introduction

### Work Context

**Problem:** The architecture in `PRD.md` and `DEVELOP.md` contains several plausible but unproven choices: a PO task planner, concern-specific generators, tree-sitter-based edits, and compiler-error ownership. Testing those pieces separately would establish that each mechanism can work, but not that protopie can evolve one application coherently. Before choosing those mechanisms, the repo needs a repeatable way to expose failures in cumulative generation and compare later strategies against the same requests.

**Goal of this epic:** Create one minimal checked-in SolidJS fixture, a three-step ordered evolution scenario, a live runner that calls the local llama.cpp model through `llm-sdk`, and a deterministic replay runner that consumes a recorded raw-response trace. The live and replay runs must reach the same terminal project contents and validation outcomes, including when a response is rejected and the sequence stops early. The first prompt strategy may be deliberately simple; its quality is evidence, not an architectural commitment.

**What we are NOT deciding:** the final PO contract, generator labels or boundaries, tree-sitter versus another editing mechanism, mock-data proxy design, automatic repair, browser automation, SQLite retrieval, the production crate layout, the `gui`, or a praxis-equivalent vocabulary. This epic does not need to prove that the full protopie thesis works. It only establishes trustworthy experiment machinery and records what the first small sequence does.

**Output:** A reproducible evaluation harness with one three-step scenario, one recorded live-model trace, deterministic replay coverage, a run report describing pass/fail results, and `epics/002-*.md` scoped from the observed failures. Epic 001 succeeds when the evidence is reproducible, even if the generated application fails one or more scenario assertions.

---

## Reference sources

| System | Local path | Why it matters |
|---|---|---|
| **protopie intent** | `PRD.md`, `DEVELOP.md` | Defines the product constraints the harness must preserve without assuming the final implementation. |
| **nocodo** | `~/Projects/nocodo/agents/DESIGN.md` | RustEngineer's prompt/raw/extracted-output capture is the reference for transparent LLM experiments. |
| **llm-sdk** | `~/Projects/llm-sdk/` | The live runner calls the local OpenAI-compatible llama.cpp endpoint through this crate. Read `README.md` and `src/` before implementing the adapter. |
| **rustysolid** | `~/Projects/rustysolid/` | Reference for the smallest credible SolidJS + Vite + TypeScript project fixture. |

---

## Scenario

The fixture begins as a minimal running application. The same working directory evolves through these ordered brain-dumps:

1. Add a Work Items screen with navigation, a heading, and an empty state.
2. Rename the Work Items concept to Cases, including its route, navigation label, component name, and user-visible copy.
3. Replace the Cases empty state with a two-pane case workspace while preserving the route and navigation introduced by the prior step.

Assertions describe observable project outcomes and superseded behavior, not which agent or editing mechanism must be used. Later epics may extend this sequence or add held-out domains; Epic 001 contains only these three steps.

---

## Tasks

- [ ] Define a versioned trace format before building the runner. Each live step records the scenario step, brain-dump, prompt version, model identifier and llama.cpp settings, assembled prompt, raw response, parsed response, proposed file operations, validation results, latency, and token counts when available.
- [ ] Add one minimal checked-in SolidJS + Vite + TypeScript fixture and document the exact install/build commands. This is an evaluation fixture, not the production `scaffold` crate.
- [ ] Define the three ordered scenario steps and their assertions, including positive requirements, behavior that must survive from earlier steps, and concepts that must disappear after being superseded.
- [ ] Implement one intentionally simple, single-shot prompt strategy. It receives only host-selected context, has no tools or filesystem access, and returns a strictly validated set of file operations limited to the fixture directory.
- [ ] Implement live mode through `llm-sdk` against the local llama.cpp endpoint. Live mode applies each accepted response to the evolving fixture, runs the declared validation commands after every step, and writes the complete trace and run report.
- [ ] Implement replay mode with no LLM or network access. Replay mode starts from a clean copy of the same fixture, feeds each recorded raw response through the same parser and validator, and performs the same accepted file operations and validations.
- [ ] Verify that live and replay modes produce the same source-tree hash, excluding dependency/build directories, and the same scenario pass/fail outcomes.
- [ ] Capture one live attempt of the ordered scenario and its replay, through all three steps or the first response that cannot be applied safely. Do not hide rejected responses or failed assertions; failures are the primary input to the next epic.
- [ ] Write a short decision report covering only what the run demonstrated about prompt contracts, cumulative change, trace sufficiency, and validation gaps. Avoid selecting PO roles, generator boundaries, or tree-sitter unless the evidence directly requires a decision.
- [ ] Write `epics/002-*.md` around the smallest observed failure or missing capability that prevents the scenario from becoming more representative.
