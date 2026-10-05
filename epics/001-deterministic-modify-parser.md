# Epic 001: Deterministic prompt-to-code `modify` (no LLM)

Crate: `crates/protopie-ui-agent`
Target: TypeScript + SolidJS + Solid Router + CSS modules, using the versions and conventions in `reference/` and its lockfile.
Status: T1-T10 done.

- T1-T8: done.
- T10: done. `Share the <label> [across pages]` shares a state through a Solid context. The agent never invents the four requirements: **scope** (the prompt's `across pages`, otherwise asked; the only supported scope mounts the provider around the router's root layout, in a new owned `providers` region of `src/App.tsx` that the reference template now ships), **state shape** (asked: text, number, yes/no, or text that may be missing), **initial value** (asked and validated per shape; an optional text starts unset by its own definition) and **consumer bindings** (asked: pages created by the agent that display the value, chosen one at a time). Each missing requirement is a persisted blocking question whose continuation (`Continuation::ProvideContext`) carries the draft, so nothing is written until the context is complete; one plan then runs `CreateContext` plus one `AddContextConsumer` per page, generating `src/context/<Name>.tsx` (the Solid 2 context object is the provider; no `useX` wrapper, per the reference cheatsheet), the provider nesting in `App.tsx` and each consumer page re-rendered with `useContext`. Added `context.rs`, `ContextRecord` / `ProjectSnapshot::contexts`, `tests/context.rs`, emitter unit tests, parser tests and a `generated_contexts_type_check_and_build` build test (passes). Not supported yet (asked about or rejected, never guessed): a context with no created page to read it (rejected with advice to create a page), the home page as a consumer, narrower scopes, setters or any writer of the state, object-shaped state, existing projects created before the `providers` region (apply reports a conflict explaining the region to add). Not verified: generated pages were type-checked and built but never run in a browser.
- T9: done. Dry runs now return the plan ID, the base revision and a per-file diff (`Preview.plan_id`, `base_revision`, `diffs`). `modify` / `answer` accept `expected_revision` and `expected_plan_id`; a project that moved on or a plan that differs from the previewed one returns a conflict and writes nothing, and an external edit to owned source conflicts without being overwritten. The GUI previews every project command (diff in chat, Apply changes / Discard bound to the previewed revision and plan) and renders structured question options as buttons that answer through `/projects/answer` (also previewed first). Added `tests/review.rs` (preview/apply/stale/external-edit/answer flows), `tests/hardening.rs` (fixed-seed property tests: no parser panics, valid spans, dry runs never write, rejected requests never change code, applied results keep owned source consistent and leave no journal, determinism), server and API wire tests, GUI review-state unit tests, and `.github/workflows/ci.yml` with a Rust job and a generated-build job (`npm ci` in `reference/`, then `generated_build -- --ignored`). Recovery scenarios were already covered in `apply_tests.rs`. Not verified here: the GUI drawing code was compiled but not run in a window, and the CI workflow has not run on GitHub.

## Outcome and scope

Replace the echo-only `modify(project_path, command)` with a deterministic command interpreter that understands a bounded vocabulary of short UI requests, builds a typed plan, and generates code in the project at the supplied path. The same prompt, conversation state, project snapshot, and policy version must produce the same semantic result. No LLM or network is used to interpret prompts or generate code.

Initial examples:

- “Add top nav” / “Need a top navigation”
- “Add Contact Us to top nav”
- “Add Contact Us” after creating or focusing a navigation
- “Image needs rounded corners”
- “Give the form below hero more padding”

Start with projects generated from `reference/`, with explicitly owned files or regions and a project manifest. Editing arbitrary handwritten TSX, inferring browser geometry, broad natural-language understanding, fuzzy spelling correction, and Solid context generation are outside the initial scope. Context is a later extension with its own clarification requirements.

## Architecture

```text
prompt ──parse──> ParsedRequest
                       │
project + session ──resolve──> ResolvedRequest
                                    │
                         plan ──> Plan + follow-up questions
                                    │
                         prepare ──> File changes + diff
                                    │
                         apply ──> Project + session update

Parse / resolve / plan may instead return a blocking question or Unsupported.
```

- **Parse:** pure text processing. Extract actions, selectors, labels, relations, and requested changes. Preserve unresolved fields and source spans; perform no IO or history lookup.
- **Resolve:** pure evaluation against an explicit project snapshot and conversation. Bind selectors to stable IDs and fill omissions using compatible context. Return blocking questions for unresolved ambiguity.
- **Plan:** pure domain logic. Produce ordered, typed operations, preconditions, and follow-up questions. Resolve relative changes into absolute results here.
- **Prepare:** render the plan against the snapshot into a complete set of proposed file changes and a diff, without writing.
- **Apply:** validate preconditions and persist files, manifest, and conversation changes. This is the only stage that writes.

`modify` orchestrates loading and these stages. Expose planning and preparation independently for tests and previews. A dry run must not change files, conversation focus, or pending questions.

Use separate modules for parsing, resolution, project data, conversation, planning, and emission. Domain planners for components, styles, routing, and eventually context share the same resolver and operation types. Start with enum dispatch; a handler plugin framework is unnecessary for the first slice.

## Parsing: lexicon and grammar

Use a table of canonical concepts and explicit aliases, matched on token boundaries with longest-phrase preference:

| Category | Examples |
| --- | --- |
| Actions | add, need, remove, move, give, increase |
| Element roles | top nav, navigation, hero section, image, form, button |
| Properties | rounded corners, border radius, padding, width |
| Relations | to, inside, above, below, left of, right of |
| Amounts | more, less, full, explicit supported lengths |
| Literal labels | Contact Us, My Doctor, Get a callback |

Lexical recognition does not imply support for every combination. Each accepted grammatical pattern must map to an implemented capability or return an explicit unsupported result.

Initial grammatical patterns:

```text
add / need <element>
add <label> to <target>
add <label>                         # target may need conversation context
add a <label> page
<target> needs <style>
give <target> <amount> <property>
increase / decrease <target> <property>
```

Later patterns include `move <target> <relation> <anchor>` and `add <element> <relation> <anchor>`.

Matching uses normalized text, but labels retain their original spelling and casing. Support quoted labels and explicit `called` / `named` forms. Unquoted lowercase labels must work. Do not use Title Case as a requirement or split unconditionally at every relation word: “Get in Touch”, “Terms of Use”, and “Back to School” are valid labels. Grammar determines where a label ends; genuinely ambiguous boundaries produce clarification with a quoted-form suggestion.

Use explicit plural aliases initially. Defer typo correction; never silently alter labels. Consume the complete meaningful input. Unsupported clauses or trailing instructions must not be discarded while applying a recognized prefix. Negation such as “do not add a nav” must never become an affirmative add operation.

For “Give the form below hero more padding”, parse `below hero` as a constraint on the form selector. It is not a move operation.

## Request, selector, and plan types

Define serializable Rust enums and structs with distinct types for each stage. Suggested concepts:

```text
ParsedRequest:
  action, subject selector or label, destination selector,
  requested change, source spans

Selector:
  optional role, optional label, optional explicit ID,
  optional relation to another selector, scope

ResolvedRequest:
  operation-specific data with concrete element / definition IDs

Plan:
  ID, schema / policy version, base project revision,
  preconditions, ordered operations, follow-up questions
```

Separate a target's relational constraints from the destination of an insertion or move. Represent unresolved navigation destinations explicitly rather than as invented URLs.

Initial operations include `CreateComponent`, `InsertElement`, `AddNavigationItem`, `SetStyle`, `CreatePage`, `RegisterRoute`, and `SetNavigationDestination`. Keep operations semantic; the emitter owns filenames and syntax conventions.

Planning outcomes should distinguish:

- `Ready`: a plan, possibly with follow-up questions.
- `NeedsClarification`: blocking questions; no code changes.
- `NoChange`: already satisfied, with a reason and any still-relevant question.
- `Unsupported`: the unsupported portion and explanation; no code changes.

The application result additionally reports applied changes or a conflict/error. Return structured data through the API, with a human-readable summary for chat. Changing the current `Result<String>` return type requires updating the server, shared wire types, GUI, and echo-based tests together.

## Project model and ownership

Persist a versioned `.protopie/model.json`. Model elements separately from reusable component definitions and their instances. Record stable IDs, roles, labels, page scope, parent relationships, sibling order, generated layout information, source bindings, style bindings, and ownership fingerprints.

Seed the model from `reference/`, including the hero element in `Home.tsx` and its CSS bindings. Give the template explicit insertion regions where necessary. Choose one documented ownership convention: regenerate wholly owned files and replace only explicitly owned regions in shared files. A manifest identifies entities; markers delimit editable regions.

Before preparing or applying changes, verify that owned source still matches its recorded state. Report external-edit conflicts rather than overwriting changes or silently trusting stale metadata. Existing projects without a manifest need an explicit supported initialization/migration path or an actionable unsupported-project result.

Initially, `below` and `above` mean sibling order within a known vertical container; `left of` and `right of` require a known horizontal layout. Source order alone does not establish visual geometry. Unsupported layouts must not be guessed.

Style bindings must identify whether an edit affects one instance or a component definition. An instance-targeted change must not accidentally affect every instance sharing a CSS class.

## Conversation and target resolution

Persist bounded conversation state under `.protopie/`, scoped by project and conversation ID. Keep a default conversation for the simple `modify(path, command)` entry point; let the API identify separate conversations explicitly.

Record resolved focus, recent successful operations, current page/selection when available, and pending questions with stable IDs. Raw prompt history can be retained for diagnostics but is not the resolution algorithm.

Resolution precedence:

1. Explicit reference in the current prompt.
2. Compatible explicit GUI selection, when available.
3. Recent compatible conversational focus.
4. Unique matching element within the current scope.
5. Blocking clarification if a required reference remains ambiguous or missing.

Define compatibility by action and role. A navigation can receive an added item; the most recently edited image cannot automatically receive “Contact Us”. Validate that remembered IDs still exist. Explicit scope changes reset incompatible focus. Use deterministic bounded turn-based history, not wall-clock recency.

```text
“Need a top navigation”
  → create nav_1; focus nav_1 after successful application
“Add Contact Us”
  → add a navigation item to nav_1; ask about destination
```

Rejected requests, previews, and failed applications must not establish successful-edit focus. A blocking question may persist a pending clarification without changing project code. A new independent command may proceed while a follow-up remains pending; a bare answer must never resolve an arbitrary question.

## Questions and navigation behavior

Separate blocking clarification from optional follow-up:

- “Which image?” blocks an edit because its target is unresolved.
- “Where should Contact Us lead?” follows adding the requested navigation item.

“Add Contact Us to top nav” creates only the item. Its destination is `Unresolved`, rendered as noninteractive text until a destination is chosen. It must not create a page, register a route, or invent a broken link.

Return a structured follow-up:

> Where should “Contact Us” lead: a new page, an existing page or section, or an external URL?

Also allow leaving it unlinked. Each answer maps to typed data or a further required question:

- **New page:** propose the label-derived path, check collisions, then create the page, route, and navigation destination in one plan. Ask if a path cannot be determined uniquely.
- **Existing page or section:** select a known destination; ask which one if necessary.
- **External URL:** request and validate the URL before linking.
- **Leave unlinked:** dismiss the question and keep the unresolved item.

Questions carry IDs, target IDs, options, and continuation data. Accept option labels/numbers in unambiguous context and structured answers referencing question IDs. Accept “yes” or “no” only for a single applicable yes/no question. Revalidate the project when answering; stale answers must not target deleted elements or silently override newer changes.

An explicit “Add a Contact Us page” directly authorizes page and route creation. Labels remain display text; identifier/path generation uses separate validated rules and collision checks.

## Styling policy

Start with corner radius, padding, and full width on supported generated layouts. Keep style policy versioned and deterministic.

- Define spacing and radius tokens in the generated project.
- “Rounded corners” selects a documented default radius token.
- “More / less padding” moves each affected axis one step on a fixed spacing scale. For `2rem 1rem`, both vertical and horizontal values advance independently unless an axis is specified.
- Support a small documented set of literal lengths and token references. Define stepping for off-scale supported values as the nearest strictly higher/lower token; clamp at scale limits and report no change there.
- For unsupported computed values, mixed units, or unclear cascade ownership, return an explanation or ask for an explicit value.
- “Full width” maps to a defined style operation only where the known parent layout supports its intended meaning.

The planner emits absolute resulting values. The emitter does not reread a value and increment it during application.

## Application, retries, and consistency

“Add top nav” is an ensure-style command: repeating it in the same scope yields no duplicate. “More padding” is incremental: a new request increases padding again. Reapplying the same plan must not repeat either operation.

Track plan/request identity and project revisions separately. Plans contain expected state and absolute outcomes; retrying an applied plan returns its recorded result. An unapplied plan against a changed project yields a conflict and requires replanning. Use request IDs at the API boundary to distinguish transport retries from deliberate repeated commands.

All project writes, including metadata, must use `resolve_in_project`. Escape labels correctly for generated TSX/CSS and validate derived names, routes, and URLs.

Prepare and validate the complete change set before writing. Serialize applications per project and recheck revisions under the lock. Stage writes and use a documented journal/recovery mechanism so interruption cannot leave code, manifest, and session permanently inconsistent. Per-file atomic replacement alone is not a multi-file transaction. Recovery and normal application must follow the same path checks.

## Testing strategy

Introduce tests with the first slice and grow them alongside every feature:

1. **Parser fixtures:** prompt → exact parsed request, ambiguity, or unsupported result.
2. **Resolver fixtures:** request + project + conversation → concrete IDs or blocking questions.
3. **Planner fixtures:** resolved request + snapshot → exact operations and follow-ups, including absence of unauthorized operations.
4. **Emitter fixtures:** apply plans to temporary copies of `reference/`; compare files and metadata, then run generated-project checks.
5. **Conversation fixtures:** ordered turns, expected results, persisted state, restart behavior, and question answers.

Use structured JSON or equivalent corpus fixtures under `crates/protopie-ui-agent/tests/corpus/`, with expected outcomes and assertions for each turn. Include lowercase and quoted labels, relation words inside labels, explicit targets overriding history, incompatible focus, multiple matching elements, unknown text, negation, and trailing unsupported clauses.

Test repeated commands separately from repeated plan application. Cover stale plans, request retries, external edits, path escapes, label escaping, and interrupted application recovery. Property/fuzz tests must establish no panics and no writes on rejected requests; arbitrary input may also legitimately parse as a supported command.

The first end-to-end navigation fixture must pass the generated project's existing build command (`tsc --noEmit` plus Vite build). Keep Rust unit tests independent of Node installation; provide a dedicated CI job with dependencies installed from the reference lockfile for generation/build verification.

## Tasks and acceptance criteria

### T1: Contracts and test harness (P0)

Define parsed/resolved request types, selectors, plan operations, question/answer types, outcomes, and versioned project/session schemas. Add structured fixture helpers and pure stage entry points. Update `modify` and API callers to carry structured outcomes while unsupported prompts remain nonmutating.

Done when serialization and a parser/resolver/planner fixture run in Rust tests, the workspace callers compile, and the GUI can display an unsupported result or question summary.

### T2: Initial grammar and label handling (P0)

Implement tokenization with source spans, longest-phrase aliases, navigation/page patterns, and the two style example patterns. Preserve omissions for resolution. Reject unsupported remainder and negation safely.

Done when all user examples parse correctly, lowercase labels work, “Get in Touch” remains intact, and ambiguous/unsupported examples produce explicit outcomes. Style parsing may precede style generation.

### T3: Project model and conversation resolution (P0)

Seed the reference manifest and ownership boundaries. Implement project loading, scoped stable IDs, revision/fingerprint checks, bounded sessions, and compatible-focus resolution. Define behavior for projects missing metadata.

Done when fresh projects round-trip, target ambiguity is tested, explicit references override focus, and a simulated navigation conversation resolves “Add Contact Us” to the correct navigation across a reload.

### T4: Preparation and reliable application (P1)

Implement rendering to proposed file changes, dry-run diffs, safe paths, project locking, revision checks, retry identity, and staged writes with recovery. Use a minimal supported operation to test the pipeline.

Done when previews leave disk/state untouched, applying the same plan twice does not duplicate effects, external edits cause a conflict, and an injected write failure can be recovered consistently.

### T5: Navigation and unresolved items (P1)

Generate navigation TSX/CSS, insert it into the reference application layout, and register its elements. Add labelled navigation items with unresolved destinations and persist follow-up questions. Wire the flow through the server and GUI using structured outcomes and conversation/request IDs.

Done when “Need a top navigation” → “Add Contact Us” works through chat and after restart, no page/route is created, the item renders unlinked, duplicate ensure requests do not duplicate navigation, and the generated project builds.

### T6: Answers, pages, and destinations (P1)

Implement question continuations, direct page creation, route registration, linking to existing pages/sections and external URLs, and leaving an item unlinked. Generate page/component identifiers and route paths with collision handling. Keep route syntax aligned with the checked-in template and lockfile.

Done when choosing a new page creates the page, route, and link together; alternatives create no page; ambiguous bare answers and stale targets are rejected; and the complete navigation conversation passes fixtures and the generated build.

### T7: Styling and relational selectors (P2)

Implement spacing/radius policy and style binding resolution. Add fixtures containing multiple images/forms, known vertical containers, and reusable component instances. Resolve “the form below hero” structurally and apply absolute style operations.

Done when both style examples generate the intended scoped CSS, ambiguous images trigger a question, shared instances are unaffected by an instance-only request, two fresh “more padding” requests advance twice, and retrying either plan does not advance again.

### T8: More components and positioning (P2)

Add hero, image, form, footer, and button generation, plus supported insertion/move patterns. Track parent layout and distinguish selector relations from positioning requests. Ask for required content or destinations instead of inventing behavior.

Done when “add a form below hero” works in a supported vertical container, horizontal placement works only with known layout semantics, and generated projects build.

### T9: Review surface and regression hardening (P2)

Add GUI plan/diff previews using the existing dry-run path and structured question controls. Expand corpus and property/fuzz coverage from encountered failures. Complete CI generation/build checks and application recovery scenarios as release gates.

Done when users can inspect a prepared change, apply it against its recorded revision, answer a specific question, and see conflicts without losing edits. Required unit, conversation, recovery, and generated-build checks pass in CI.

### T10: Solid context extension (P3, optional)

Design requests such as “share the selected doctor across pages” around explicit state shape, initial value, provider scope, and consumer bindings. Add typed context operations only after those requirements can be resolved or asked about.

Done when an agreed context fixture generates a provider and consumers with the correct scope and passes the generated build. This task does not block the initial epic delivery.

## Delivery order

Implement T1–T4 with minimal operations, then complete T5–T6 as the first usable vertical slice: navigation → item → destination question → page/route. Ship its tests and basic chat interaction together. Continue with T7 styling, T8 component breadth, and T9 richer review/regression coverage. T10 remains optional.

Each supported phrasing and conversation rule lands with its tests. Add a regression fixture before fixing a failed real-world phrasing. Expand vocabulary only when its resolution, code generation, and uncertainty behavior are defined.
