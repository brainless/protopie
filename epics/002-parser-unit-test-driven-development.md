# Epic 002: Build the deterministic parser through unit-test-driven development

Crate: `crates/protopie-ui-agent`
Status: in progress; T1–T7 and R1–R3 complete; R4 reviewed but not cleared pending F001-T1.
Related: [Epic 001](001-deterministic-modify-parser.md), specifically its parse stage and T2.

## High-level overview

Build a pure Rust parser for the bounded UI command language in Epic 001 using small test-first increments. Each increment introduces an observable language behavior, a failing unit test, the minimum implementation needed to pass it, and any refactoring supported by the accumulated tests.

The parser accepts only prompt text. It returns a typed parsed request, a linguistic clarification, or an unsupported result. It does not discover what exists in a project or decide how to edit it. For example, `Add Contact Us` preserves an omitted target for the future resolver; it does not choose a navigation from history or ask which existing navigation to use.

Tests also shape the parser's contract. Introduce only the request variants and fields needed by the next small set of tests. Do not design every type from Epic 001 before beginning, and do not write the entire parser before its tests.

This epic refines the implementation approach for parser work in Epic 001. It does not complete that epic's broader contracts, integration, resolution, planning, or generation tasks. Parser unit cases live in Rust initially, rather than requiring the external corpus infrastructure described for the broader pipeline.

## Objectives

1. Make accepted syntax, preserved information, and rejection behavior executable specifications.
2. Preserve original label text and meaningful source spans while matching command words and aliases consistently.
3. Consume complete meaningful input: never silently accept a supported prefix and discard an unsupported instruction.
4. Keep missing project context distinct from ambiguity in the wording itself.
5. Establish a small, deterministic parser interface that future resolution work can consume.
6. Allow multiple coding-agent sessions to extend the implementation without repeatedly redesigning it or losing regression coverage.

## Scope and boundaries

Included:

- Navigation creation requests, labelled additions with explicit or omitted targets, and explicit page requests.
- Quoted and lowercase labels, relation words inside labels, and documented `called` / `named` forms.
- The two style examples from Epic 001, relative padding requests, and full-width requests at the syntax level.
- Role selectors and the `below hero` relational selector needed by the form example.
- Typed outcomes, original-input spans, deterministic diagnostics, and parser unit tests.

Excluded:

- Changes to `modify`, its echo behavior, the server, wire API, or GUI.
- Project fixtures, filesystem assertions, manifests, conversation state, target resolution, and question continuations.
- Plan operations, CSS token arithmetic, actual component generation, routes, and generated-project builds.
- Move commands, component-generation breadth, typo correction, general natural-language understanding, and LLMs.
- A standalone fuzzing setup, external JSON fixture framework, or parser plugin architecture.

Recognizing a style or page request here does not imply that the application can execute it yet. Leave the parser disconnected from `modify` until the corresponding Epic 001 integration work handles capability checks.

## Contract and test conventions

### Minimum public shape

Expose one pure entry point conceptually equivalent to `parse(&str) -> ParseOutcome`. Exact Rust names can be settled in T1 and then recorded as decisions.

Outcomes must distinguish:

- **Parsed:** operation-specific syntactic data, including omissions that a resolver may fill later.
- **NeedsClarification:** ambiguity in the input's interpretation, with a structured reason and relevant span. Include a quoted-form suggestion for ambiguous label boundaries.
- **Unsupported:** an unsupported or malformed input, with a structured reason and relevant span; no partially successful request.

Prefer request variants that make invalid field combinations difficult to construct. A generic `Intent` enum is not a requirement. Do not introduce resolved IDs, plans, persistence versions, or future operation variants just to mirror Epic 001.

A labelled addition with no target must not invent that target's role. Similarly, a navigation target is different from the item's eventual link destination; parsing must not invent a URL or page.

Use half-open UTF-8 byte ranges into the original input for spans. Every span must be ordered, in bounds, and on character boundaries. Document which fields carry spans and whether quotes are included. Labels retain original casing and interior text; normalization applies to recognized syntax, not display labels. Semantic comparisons between aliases may exclude their naturally different spans, but dedicated tests must verify those spans.

### Inline unit cases

Keep tests under the parser module using `#[cfg(test)]`, with prompt strings and expected Rust values directly in the tests. Split into test submodules when useful. Use table-driven tests for aliases and rejection families; use named individual tests for distinct grammar decisions.

Each case should assert all relevant request fields or the structured diagnostic reason and span. Do not assert only success or one intent while ignoring a wrong label, target, or trailing clause. Human-readable diagnostic wording need not be frozen unless the wording itself is the requirement.

Small expected-value builders are acceptable. They must not invoke production parsing or derive expected values using the same recognition logic being tested. No snapshots, template copies, Node installation, network access, mocks of project state, or filesystem fixtures are needed.

### Red, green, refactor

For each behavior:

1. Add a named test with explicit expected output.
2. Add only enough type/function scaffolding for the test to compile if necessary.
3. Run it and observe an assertion failure caused by the missing behavior. A compiler error alone is not the final red evidence.
4. Implement the smallest coherent behavior that passes the test.
5. Run the accumulated parser tests and refactor if useful.
6. Record the behavior, test name, red/green evidence, and any contract decision in the handoff log.

Pair positive grammar cases with nearby negative cases in the same increment. Keep completed work green; do not maintain a large ignored-test backlog or claim a task is complete with placeholder implementations. A temporary always-unsupported parser is acceptable scaffolding for the first red test, not a completed parser.

## Tasks

### T1: Establish the smallest contract and first test-first behavior

Dependencies: none.

- Inspect existing code and this epic before choosing module layout. Keep scaffolding and existing behavior intact.
- Introduce the parser module, outcome types, span type, and only the request data needed for navigation creation.
- Drive `Add top nav` through an observed failing assertion to a parsed navigation-creation request.
- Add empty/whitespace input and unrelated text rejection tests.
- Document the chosen contract and span convention in the decision log below.

Done when the first real parser behavior and rejection cases pass as unit tests, the parser is independent of IO/context, and existing crate tests still pass. Do not implement all future types in this task.

### T2: Navigation aliases, normalization, and complete-input handling

Dependencies: T1.

- Add cases for `Need a top navigation`, `add navigation`, case variations, and documented whitespace handling.
- Introduce token-boundary matching and longest-phrase preference through observable cases. Do not equate lexical recognition with support for arbitrary combinations.
- Add near-match, negation, incomplete-input, and unsupported-tail cases alongside positive cases: for example `do not add a nav`, `Add top nav and delete the hero`, and `Add top nav then move it` must not produce an affirmative or partial request.
- Document punctuation, articles, and the exact navigation aliases supported. Do not silently expand the vocabulary.

Done when aliases have equivalent semantic requests, all meaningful input is accounted for, and these rejection cases have structured outcomes and accurate spans.

### T3: Labelled additions and omitted targets

Dependencies: T2.

- Introduce label and target data through tests for `Add Contact Us to top nav` and `Add Contact Us`.
- Add lowercase labels, such as `add contact us to top nav`, without requiring Title Case.
- Preserve omitted targets explicitly. Parse results depend only on the current string, with no remembered navigation or successful-edit focus.
- Distinguish a container target from a navigation link destination. Do not derive URLs, routes, or identifiers.
- Pair cases with incomplete delimiters and missing-label inputs, following a documented interpretation instead of dropping tokens.

Done when complete typed expectations cover both explicit and absent targets and label spelling is preserved.

### T4: Quoting, label boundaries, and linguistic ambiguity

Dependencies: T3.

- Cover `Add Get in Touch to top nav`, `Add Terms of Use to top nav`, and `Add Back to School to top nav` with exact labels and targets.
- Support quoted labels, including words that would otherwise be command syntax. Add a quoted label containing a conjunction so rejection logic does not blindly reject words inside literal text.
- Specify supported quote delimiters and escaping behavior, then test valid quoting, unmatched quotes, and unsupported escapes as applicable.
- Choose and record exact `called` / `named` forms before implementing them; a suitable narrow starting pair is `Add an item called Contact Us to top nav` and `Add an item named Contact Us to top nav`.
- Define and record at least one concrete unquoted string with two plausible grammatical interpretations. Assert clarification with a quoted-form suggestion; assert that explicit quoting selects the intended interpretation.
- Add Unicode label cases and verify byte spans against the original string after normalization of surrounding syntax.

Done when labels are not split at every relation word, ambiguous boundaries are not guessed, and label/diagnostic spans can safely slice the original input. Grammar decisions must include the actual prompt and expected interpretation, not just a general statement about ambiguity.

### T5: Explicit page request syntax

Dependencies: T4.

- Introduce a page-request variant through `Add a Contact Us page`, including lowercase and quoted-label cases.
- Test its distinction from a labelled addition to navigation and from a quoted label containing the word `page`.
- Preserve display text without slugifying it or inventing route paths.
- Add incomplete and unsupported-tail cases for this pattern.

Done when page requests and labelled additions have distinct exact outputs and all earlier label-boundary cases remain green.

### T6: Style requests and relational selectors

Dependencies: T5.

- Drive `Image needs rounded corners` into a style request with an unresolved role selector and a requested radius concept, not a concrete CSS value.
- Drive `Give the form below hero more padding` into a form selector constrained by its relation to a hero selector, plus a relative padding increase. It must not be a move request.
- Add `Give the form less padding`, `Increase form padding`, `Decrease form padding`, and `Image needs full width` as documented narrow patterns.
- Preserve relative amounts for the planner. Do not read existing styles, choose tokens, calculate values, or validate layout feasibility.
- Pair each pattern with unsupported-property, incomplete-selector, and trailing-instruction cases.
- Explicit numeric lengths, axis-specific changes, plural selectors, and additional relations remain deferred unless added as separately documented test-first scope increments.

Done when the two Epic 001 style examples and the listed narrow variants have exact typed outputs, selector relations remain separate from actions, and no project-dependent behavior has entered the parser.

### T7: Consolidate the regression suite and integration handoff

Dependencies: T6.

- Review coverage across accepted syntax, ambiguity, malformed input, negation, token boundaries, quoting, unsupported remainder, and original-input spans. Add missing regression cases before fixes.
- Add a small deterministic adversarial table: empty strings, punctuation, multibyte text, unmatched quotes, and repeated delimiters. Assert specified outcomes and valid spans; ordinary test execution should expose any panic.
- Assert repeat calls return equal results. Do not assert arbitrary text must always be unsupported; valid supported text may appear in adversarial inputs.
- Consolidate the supported grammar and deferred syntax in the decision log or a linked parser-local document. Update earlier decisions if later tests deliberately supersede them.
- Run all crate library unit tests and document how Epic 001 can consume the parser without claiming integration is complete.

Done when every completed behavior has active passing tests, the grammar is documented, no test is ignored to hide missing behavior, and another session can identify the next integration work without reverse-engineering the parser.

## Multi-session implementation rules

### Review tasks and feedback loop

Review tasks are part of delivery, not optional final cleanup. Run them at the checkpoints below. A review may be performed in a later coding-agent session; a separate agent is not required. Review the actual implementation and tests, rather than only the previous session's summary.

| Review task | When | Focus |
| --- | --- | --- |
| R1: Contract and rejection review | After T2, before T3 | Is the contract minimal and pure? Do tests cover complete-input handling, negation, boundaries, and original spans? |
| R2: Label grammar review | After T4, before T5 | Are label boundaries, quotes, omissions, Unicode, and clarification rules consistent? Are there accepted strings that lose meaningful text? |
| R3: Full grammar review | After T6, before T7 | Do page/style patterns introduce precedence regressions? Are relations distinct from actions? Has any resolution or planning behavior leaked into parsing? |
| R4: Completion review | After T7 | Are all promised cases implemented, findings accounted for, tests active, and decisions/handoffs consistent with the code? |

Each review follows this process:

1. Run the existing unit-test suite and inspect coverage against this epic's requirements and recorded grammar decisions.
2. Probe plausible counterexamples with unit tests or local test runs. Review for correctness and missing assertions, not preferred code style. Distinguish observed defects from unverified concerns.
3. Append each actionable finding to the findings backlog with a stable ID such as `F001`. Record the triggering prompt, expected versus actual behavior (or the coverage gap), affected requirement, and evidence. Record a clean review too; do not invent findings to fill the log.
4. Classify the finding as a required correction within this epic, an optional improvement, or deferred scope belonging to Epic 001 or later work. Give a concrete rationale. A correctness defect in promised behavior cannot be silently deferred to declare completion.
5. Create a follow-up task `F001-T1` (and additional subtasks only if needed) with a proposed failing unit test, acceptance criteria, and dependencies. Keep it small enough for another session to resume.
6. Address required corrections before the next implementation task. Use the same red -> green -> refactor process; for a coverage gap where the new test already passes, record that honestly rather than manufacturing a failure.
7. Re-review the changed behavior and adjacent grammar cases. Close the finding only with passing test evidence and a recorded verification result. Add newly discovered issues as new findings and repeat this loop until required corrections are closed.

A review is **complete** when its inspection and findings are recorded. Its checkpoint is **cleared** only when all required follow-up tasks are verified. Optional improvements do not block progress but must have an explicit disposition. If a requirement is genuinely undecidable from the epic and established decisions, record the precise open question and continue independent work; do not silently choose a conflicting contract or restart the parser.

Review tasks may also be reopened after later changes invalidate their conclusions. Preserve earlier findings and verification history. A strategy change proposed during review must follow the continuity rules below and be justified by a finding; the review loop is not permission for repeated rewrites.

### Start of each coding session

1. Read this epic, its decision/handoff logs, and applicable repository instructions.
2. Inspect git status/diff and the existing parser and tests. Preserve unrelated or unfinished work.
3. Run the current crate unit-test baseline and distinguish pre-existing failures from new failures.
4. Check pending reviews and findings. Finish required correction tasks at the current checkpoint before selecting the next implementation task. Otherwise select the earliest incomplete task whose dependencies are satisfied. Continue its existing implementation instead of creating a competing parser.

Use `cargo test -p protopie-ui-agent --lib` as the crate-level gate. During an increment, a test-name filter may shorten the red/green loop. Tests must not require a running server, GUI, generated application, or network. A first Cargo dependency fetch is an environment setup concern, not a test dependency on network access.

### Continuity and strategy changes

- Accepted behavior and its tests are the stable contract. Internal module layout and parsing algorithms can evolve.
- A new session's preference for another parsing style is not sufficient reason to replace working code. First identify a failing requirement, concrete maintenance problem, or measured limitation.
- Before a substantial refactor, record that reason, affected contract, and smallest proposed migration. Preserve behavioral coverage and keep the implementation in one authoritative path.
- If behavior must change, add or update an explicit test and record the old/new interpretation and rationale. Never weaken assertions or delete regression cases solely to make a replacement pass.
- Distinguish scope expansion from refactoring. New phrasing needs its own tests and documented semantics; it must not arrive accidentally through a broad matching rule.
- Avoid parallel legacy/new parsers, speculative frameworks, and wholesale rewrites of completed tasks. No approval checkpoint is required for routine refactoring within this scope.

### End of each coding session

- Update task status and append a concise handoff entry identifying files changed, behavior completed, named red/green examples, commands/results, unresolved decisions, and the next smallest test to write.
- Update review status and finding dispositions. Name the next review or correction task explicitly when one is pending, so another session continues the feedback loop.
- Record actual evidence only. If a red run was not observed, do not claim that it was.
- Prefer a green stopping point. If interrupted while red, label it explicitly and name the failing test and remaining work; do not mark the task done.
- Do not claim Epic 001 API, resolver, generation, or application milestones based on this parser suite.

## Task status

| Task | Status | Completion evidence |
| --- | --- | --- |
| T1: Minimum contract | Done | `add_top_nav_creates_top_navigation` observed red then green; 9 crate library tests pass. |
| T2: Navigation and rejection | Done | Navigation alias, boundary, normalization, negation, incomplete-input, and unsupported-tail tests; 14 crate library tests pass. |
| R1: Contract and rejection review | Cleared | Independent code/test inspection plus `review_probes_preserve_rejection_spans`; 15 crate library tests pass, no required findings. |
| T3: Labels and omissions | Done | Explicit/omitted container, lowercase label, incomplete delimiter, and unsupported-tail tests; 20 crate library tests pass. |
| T4: Boundaries and quoting | Done | Relation-word labels, double-quoted literals and escapes, called/named items, concrete clarification, and Unicode spans; 27 crate library tests pass. |
| R2: Label grammar review | Cleared | Independent inspection plus quote-boundary, repeated-target, and quoted-Unicode probes; 28 crate library tests pass, no required findings. |
| T5: Page syntax | Done | Distinct page request with exact display labels and spans; lowercase, quoted, repeated `page`, incomplete-input, and unsupported-tail tests; 32 crate library tests pass. |
| T5 independent review | Cleared | Page/item precedence, relation-word and quoted page labels, incomplete forms, and post-page tails checked; 33 crate library tests pass, no findings. |
| T6: Styles and selectors | Done | Exact typed image/form style requests, unresolved role and below-hero selectors, relative padding directions, and negative cases; 39 crate library tests pass. |
| R3: Full grammar review | Cleared | Independent page/style precedence, relation, tail, and purity review plus a focused regression probe; 40 crate library tests pass, no findings. |
| T7: Regression and handoff | Done | Deterministic adversarial outcomes, nested UTF-8 span checks, repeat-call equality, and consolidated grammar/handoff; 42 crate library tests pass. |
| R4: Completion review | Reviewed; not cleared | The 42-test library suite passes with no ignored tests, but F001 exposes page syntax falling through to a labelled addition when its final `page` noun has unsupported punctuation. |

## Review findings and follow-up tasks

Append findings rather than replacing this section with only the latest review. Use this format:

```text
Finding ID / originating review:
Classification: required correction | optional improvement | deferred scope
Status: open | in progress | awaiting verification | closed | deferred
Requirement and affected code/tests:
Triggering prompt or coverage gap:
Expected / actual behavior:
Evidence (observed defect or unverified concern):
Follow-up task ID, dependencies, and proposed unit test:
Acceptance criteria:
Resolution or deferral rationale:
Verification command/results and regression test names:
```

Epic completion requires T1–T7 complete, R1–R4 cleared, and no open required corrections. Record optional/deferred findings with their destination and rationale so later work can recover them.

F001 / R4 completion review (2026-10-02):
- Classification: required correction within this epic. Status: open; R4 is not cleared.
- Requirement and affected code/tests: T5 page/item precedence and incomplete or malformed page requests; T7's consolidated rule that only navigation aliases permit an attached final period and punctuation on syntax words is unsupported. `parse_page_creation` returns `None` when the final page noun is `page.`, `page,`, or `page!`, after which `parse_labelled_addition` accepts the whole phrase as an omitted-target item. Existing page rejection tests cover trailing words but not punctuation on the page noun.
- Triggering prompts: `Add a Contact Us page.`, `Add a Contact Us page,`, and `Add a Contact Us page!`.
- Expected / actual behavior: each should return `Unsupported(UnrecognizedInput)` over the meaningful `0..22` input span under the recorded punctuation rule; each actually returns `Parsed(AddLabelledItem)` with label `a Contact Us page.`, `a Contact Us page,`, or `a Contact Us page!` and `target: None`. The comparable quoted form `Add a "Contact Us" page.` already returns `Unsupported(UnrecognizedInput)`.
- Evidence: `cargo test -p protopie-ui-agent --lib` passed 42/42 with zero ignored tests; a standalone local probe linked against a fresh `cargo build -p protopie-ui-agent --lib` printed the outcomes above. This is an observed accepted malformed page command, not an unverified concern.
- Follow-up task F001-T1 (depends on T7): add a named failing parser unit test for those three prompts with exact `Unsupported` reasons and spans; then prevent the malformed page form from falling through to `AddLabelledItem` while keeping valid `Add a Contact Us page` and `Add Contact Us` behavior. Run the full crate library suite and adjacent page/item/quote tests, then re-review F001 and clear R4 only after passing evidence is recorded.
- Resolution rationale: accepting a malformed page command as a different operation breaks the documented precedence and complete-input contract. This is required parser correctness work, not deferred Epic 001 integration.

T1 independent review (2026-10-02): clean; no findings. The parser's exact-match branch accepts `Add top nav` with span `0..11`, while empty or whitespace-only input and unrelated text return structured `Unsupported` outcomes with in-bounds original-input spans. Inspection also confirms that an added instruction such as `Add top nav and delete the hero` cannot be accepted by the current exact-match branch; T2 owns the explicit regression case and broader grammar. `cargo test -p protopie-ui-agent --lib` passed 9/9, and `git diff --check` passed. T1 review is positive; R1 remains scheduled after T2.

R1 independent review (2026-10-02): clean; no findings. The public parser accepts only prompt text and returns operation-specific data or a structured rejection; `modify` still has its original echo behavior. Inspection of the four exact token aliases and the full-input check found no accepted prefix that discards a following instruction. The added `review_probes_preserve_rejection_spans` test verifies that `do not add top nav` is rejected as negated, a whitespace-separated `and delete the hero` tail is rejected with the tail's original byte span, and `navé` does not match `nav` at a token boundary. These probes passed, as did all 15 crate library tests. The R1 checkpoint is cleared; T3 is next.

T3 independent review (2026-10-02): clean; no findings. Inspection confirms that explicit `to top nav` creates a container target with its own span, while `Add Contact Us` retains `target: None` even after parsing a navigation command. Labels keep original casing and interior whitespace; missing labels and incomplete delimiters are rejected, and a second instruction is not absorbed into a label or target. The added `review_probes_label_spans_and_complete_targets` test covers uppercase syntax around an unchanged label, an unknown target, and a `then` tail. `cargo test -p protopie-ui-agent --lib` passed 21/21; parser-local `rustfmt --check` and `git diff --check` passed. T3 review is positive; T4 is next, followed by R2.

R2 independent review (2026-10-02): clean; no findings. Inspection found that omitted targets remain `None`; unquoted labels use the recorded rightmost complete `to top nav` boundary; quotes protect syntax words and conjunctions; unsupported escapes, unmatched quotes, and adjacent text after a closing quote are rejected. The new `review_probes_quoted_boundaries_and_repeated_target_phrase` test verifies quoted Unicode byte spans after uppercase syntax, retention of `Back to top nav` before a final target clause, rejection of a glued closing quote, and rejection of an instruction after a quoted label. The test passed on its first run, and all 28 crate library tests passed. R2 is cleared; T5 is next.

T5 independent review (2026-10-02): clean; no findings. Inspection confirmed that `Add a <label> page` returns a distinct `CreatePage` with only a display label and original-input spans, while `Add Contact Us to top nav` and `Add "Contact Us page" to top nav` remain labelled additions. Missing page parts and text following a completed page noun produce structured unsupported outcomes. The added `review_probes_page_label_boundaries_and_tails` test checks an unquoted relation-word label under uppercase syntax, a quoted label containing `and` and `page`, and a trailing instruction after a quoted page request. It passed on its first run, and `cargo test -p protopie-ui-agent --lib` passed 33/33. T6 is next; R3 remains after T6.

R3 independent review (2026-10-02): clean; no findings. The page branch applies to `Add a <label> page`, while style commands begin with their own exact command words; page labels containing `form below hero` remain page labels. The `Below` relation and unresolved hero anchor are selector data, separate from the relative padding change, and the parser takes only `&str` without project lookup, CSS values, routes, or plan operations. The new `review_probes_page_style_precedence_relations_and_tails` test verifies page/style precedence, mixed-case and whitespace-adjusted relation spans, and rejection of instructions after complete page and style commands. It passed on its first run, and all 40 crate library tests passed. R3 is cleared; T7 is next.

R4 independent completion review (2026-10-02): inspection confirmed active tests for the T1–T7 accepted phrases, ambiguity, omitted targets, roles and relations, negation, malformed quotes, unsupported tails, original-input byte spans, and repeat-call equality; `modify` still echoes commands. `cargo test -p protopie-ui-agent --lib` passed 42/42 with zero ignored tests, and `git diff --check` passed before this review entry. A local counterexample probe found F001: punctuation on the final unquoted page noun silently changes a page-looking command into a labelled addition. R4 inspection is recorded but its checkpoint remains uncleared until F001-T1 is implemented and independently verified.

## Decision log

Initial constraints: pure parser; inline Rust unit cases; incremental operation-specific types; original-input UTF-8 byte spans; no application integration in this epic.

Append decisions here with task ID, chosen rule, representative input/output, and rationale. Mark superseded decisions explicitly instead of leaving contradictory guidance.

- T1: Public entry point is `parser::parse(&str) -> ParseOutcome`. `Parsed(Request::CreateNavigation(NavigationCreation { position: Top, span }))` represents `Add top nav`; `NeedsClarification` and `Unsupported` carry structured reasons and spans. No project data enters this interface.
- T1: `Span { start, end }` is a half-open UTF-8 byte range into the original prompt. A recognized navigation request's span covers its complete command text and excludes surrounding whitespace if that becomes accepted. Diagnostic spans cover the rejected original input, including whitespace for empty input. No T1 field represents a quoted label. The sole accepted T1 phrase is exactly `Add top nav`; T2 defines aliases, case, and whitespace normalization.
- T2: The complete navigation-creation aliases are `Add top nav`, `Add top navigation`, `Add navigation`, and `Need a top navigation`, matched case-insensitively at token boundaries. All four produce `CreateNavigation { position: Top }`; position `Top` is the currently supported navigation default when the word `top` is absent. No other article or role aliases are implied: `Add a navigation` and `Need top navigation` are unsupported. Longer phrases are checked before shorter ones, so `top navigation` is treated as a single specific role phrase. Whitespace may surround the command or separate words, including newlines and tabs. One attached final period is accepted and included in the request span; other punctuation is unsupported. The request span covers the complete meaningful command, excluding surrounding whitespace. `UnrecognizedInput`, `IncompleteInput`, and `NegatedRequest` cover the complete meaningful input; `UnsupportedTail` covers the first extra token through the last, excluding separator whitespace. Empty-input diagnostics retain the full input span. ASCII case normalization applies only to syntax words; no label behavior is defined yet.
- T2: A command is accepted only when all tokens match one alias. An exact alias prefix followed by further words yields `UnsupportedTail`, with no partial request. `do not add ...` yields `NegatedRequest`. Prefixes of supported aliases such as `Add top` yield `IncompleteInput`; glued near matches such as `navbar` yield `UnrecognizedInput`.
- T3: `Add Contact Us to top nav` produces `AddLabelledItem { label: "Contact Us", target: Some(TopNavigation) }`; `Add Contact Us` produces the same label with `target: None`, even after another parse call. The target is the container that receives the item, not the item's link destination; no destination, route, URL, or ID is represented. ASCII case folding applies to `Add`, `to`, `top`, and `nav` only. The label is copied exactly from the original input between its first and last tokens, preserving casing and interior whitespace. The request span covers the complete meaningful command, the label span covers only that interior text, and the explicit target span covers `top nav` without `to`; all exclude surrounding whitespace.
- T3: The narrow unquoted delimiter is `to top nav`. `Add to top nav` and `Add to` lack a label; `Add Contact Us to` and `Add Contact Us to top` lack a complete target. All yield `IncompleteInput` over the meaningful command. An unknown target yields `UnrecognizedInput`; words after a complete `to top nav` target yield `UnsupportedTail` starting at the first extra token. An unquoted `and` or `then` in a bare label likewise starts an `UnsupportedTail`, preventing a second instruction from being swallowed. Navigation command starts and their T2 near matches keep their existing interpretation. T4 owns quoted labels and more complex label boundaries, including relation words inside labels.
- T4: Supersedes T3's first-`to` boundary selection. For an unquoted addition, the rightmost complete `to top nav` is the container boundary. Thus `Add Get in Touch to top nav`, `Add Terms of Use to top nav`, and `Add Back to School to top nav` keep `Get in Touch`, `Terms of Use`, and `Back to School` as exact labels. If no complete boundary exists, the first `to` retains T3's incomplete/unknown-target diagnostics. Bare `and` and `then` remain unsupported instruction boundaries.
- T4: ASCII double quotes delimit a label immediately after `Add` or after `Add an item called` / `Add an item named`. The label span excludes the quote characters and covers the raw interior bytes. The label text decodes only `\"` and `\\` escapes; other backslash escapes are `UnrecognizedInput` on the escape bytes, and an unmatched quote is `IncompleteInput` over the meaningful command. Quotes permit `to`, `and`, `then`, and navigation syntax as literal label words. Opening single and typographic quotes are unsupported, as are stray double quotes or backslashes in a bare label. `Add "top nav"` is an item with an omitted target. `Add "Back to School and navigation" to top nav` is one item with an explicit target.
- T4: The two introduced forms are exactly `Add an item called <label> [to top nav]` and `Add an item named <label> [to top nav]`, with the same bare or double-quoted label rules; the introducer is syntax and excluded from the label span. `Add an item called Contact Us to top nav` produces label `Contact Us` with a top-navigation container. Incomplete introduced forms are unsupported.
- T4: `Add navigation to top nav` has two plausible readings: create a navigation with an attachment clause, or add an item literally named `navigation` to the existing top navigation. It produces `NeedsClarification(AmbiguousLabelBoundary)` on the `navigation` bytes with suggestion `Add "navigation" to top nav`. That quoted form selects the labelled item. The four exact T2 navigation creation aliases remain unchanged. This is a deliberately narrow ambiguity rule; it does not make every unquoted `to top nav` addition ambiguous.
- T4: ASCII case folding still applies only to syntax. In `  ADD Café ☕ TO TOP NAV  `, the label is exactly `Café ☕`, its span is bytes `6..15`, the target is `19..26`, and the complete request is `2..26`; all safely slice the original UTF-8 input.
- T5: `Add a <label> page` produces `CreatePage(PageCreation { label, span })`, distinct from `AddLabelledItem`. `Add a Contact Us page` keeps display text `Contact Us` and its original-input byte span; no route, slug, or destination field exists. Syntax words are ASCII case-insensitive, and a double-quoted label uses T4's escaping and raw-interior span rules. `Add a "Café page" page` and `Add a Help page page` both retain `page` inside their labels. An unquoted final `page` is the page noun; when the command continues after an earlier `page`, the continuation is an `UnsupportedTail`. `Add a` and `Add a page` are incomplete, as is a quoted label without a following `page`. `Add Contact Us to top nav` and `Add "Contact Us page" to top nav` remain labelled additions. The established `Add a navigation` near match remains unrecognized.
- T6: The exact style forms are `Image needs rounded corners`, `Image needs full width`, `Give the form below hero more padding`, `Give the form less padding`, `Increase form padding`, and `Decrease form padding`. Syntax words use ASCII case-insensitive matching and whitespace token boundaries. They produce `Request::Style(StyleRequest { selector, change, span })`. `RoundedCorners` and `FullWidth` are requested concepts without CSS values; `Padding(Increase|Decrease)` preserves relative direction without calculating an amount. `Image`, `Form`, and `Hero` are unresolved roles. The form selector in the below-hero form carries a `Below` relation whose anchor is a separate unresolved hero selector; it is not a move action. The request span covers the complete meaningful command, the selector span covers the role and any relation, and a relation span covers `below hero`; all are half-open original-input byte ranges. Exact-prefix omissions yield `IncompleteInput`, recognized complete commands with further tokens yield `UnsupportedTail`, and unsupported properties/selectors yield `UnrecognizedInput`. Numeric lengths, axis-specific changes, plural roles, further relations, and other style phrases remain deferred.
- T7: Consolidated supported grammar, with ASCII case-insensitive syntax words and whitespace between words:

  ```text
  Add top nav | Add top navigation | Add navigation | Need a top navigation
  Add <label> [to top nav]
  Add an item called <label> [to top nav]
  Add an item named <label> [to top nav]
  Add a <label> page
  Image needs rounded corners | Image needs full width
  Give the form below hero more padding | Give the form less padding
  Increase form padding | Decrease form padding
  ```

  `<label>` is a nonempty unquoted sequence or an ASCII double-quoted literal. Unquoted labels retain original casing and interior whitespace; the final complete `to top nav` phrase marks an explicit container, so earlier such words stay in the label. Bare `and` and `then` mark unsupported continuation. A quoted label may contain syntax words and conjunctions and decodes only `\"` and `\\`; its source span excludes the quotes and retains the raw interior bytes. The bare `Add navigation to top nav` boundary remains a `NeedsClarification` case with the suggestion `Add "navigation" to top nav`. The four navigation aliases alone permit one attached final period. The parser returns `Unsupported` for malformed or unmatched quotes, unsupported punctuation on syntax words, unsupported targets/properties, negation, and extra instructions; it never returns a partially parsed request. Exact reason and span rules remain in the task-specific decisions above and active tests.

- T7: Deferred syntax includes other navigation aliases/articles, arbitrary element roles or container targets, destinations and routes, move/remove commands, further selector relations, plural roles, numeric or axis-specific style values, extra properties, other quote delimiters or escapes, typo correction, and general natural-language phrasing. These need explicit syntax tests and capability checks before acceptance. Repeated `to top nav` phrases follow the T4 rightmost-boundary rule: `Add Contact Us to top nav to top nav` parses as the label `Contact Us to top nav` with the final phrase as its target. The T7 adversarial table specifies that outcome alongside empty, punctuation, multibyte, and malformed-quote inputs. Every returned span in that table is checked for bounds and UTF-8 character boundaries.

- T7/Epic 001 handoff: The pure entry point is `protopie_ui_agent::parser::parse(&str) -> ParseOutcome`, exported through `lib.rs`. Epic 001's contracts and integration work can translate `Request` variants to its parsed/resolved-stage types, pass omitted `target: None` to the resolver, and map `NeedsClarification`/`Unsupported` to structured application outcomes. It must gate execution by supported planner/emitter capabilities before changing `modify`; syntactic recognition here does not authorize page, style, or navigation edits. Resolution, conversation focus, destination questions, CSS values, routes, file writes, server/wire/GUI changes, and generated-project builds remain Epic 001 work. The current `modify` still echoes commands.

## Session handoff log

Suggested entry format:

```text
Session/date:
Task and status:
Files changed:
Behavior and named tests added:
Observed red -> green evidence:
Validation command/results:
Contract decisions or superseded rules:
Review performed, findings added/closed, pending checkpoint:
Known failures/blockers:
Next smallest test:
```

Session/2026-10-02:
Task and status: T1 done.
Files changed: `crates/protopie-ui-agent/src/lib.rs`, new `src/parser.rs`, this epic.
Behavior and named tests added: `add_top_nav_creates_top_navigation`, `empty_and_whitespace_inputs_are_unsupported`, `unrelated_text_is_unsupported`.
Observed red -> green evidence: filtered navigation test failed an assertion with `Unsupported(UnrecognizedInput)` versus expected `Parsed(CreateNavigation)`; after the exact phrase rule, the full crate library suite passed 9/9.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 9/9; `git diff --check` passed. Workspace `cargo fmt --check` reports pre-existing formatting differences in unrelated files; only the new parser module was formatted.
Contract decisions or superseded rules: see T1 decisions above.
Review performed, findings added/closed, pending checkpoint: T1 implementation awaits the requested independent review; R1 remains after T2.
Known failures/blockers: none in crate library tests.
Next smallest test: T2 `Need a top navigation` should parse as the same top navigation creation semantic request, with its own original-input span.

Session/2026-10-02:
Task and status: T2 done; R1 pending.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `navigation_aliases_and_case_share_the_top_navigation_request`, `navigation_whitespace_and_terminal_period_preserve_original_span`, `navigation_near_matches_and_incomplete_commands_are_rejected`, `negated_navigation_command_is_rejected`, and `navigation_commands_reject_unsupported_tails`.
Observed red -> green evidence: filtered alias test failed an assertion for `Need a top navigation` (`Unsupported(UnrecognizedInput)` versus expected `Parsed(CreateNavigation)`), then passed after token matching and full-input checks were implemented.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 14/14; `rustfmt --edition 2021 --check crates/protopie-ui-agent/src/parser.rs` and `git diff --check` passed.
Contract decisions or superseded rules: see T2 decisions above. T1's exact-phrase acceptance is extended by these four aliases.
Review performed, findings added/closed, pending checkpoint: R1 is the next checkpoint and must be cleared before T3.
Known failures/blockers: none in crate library tests.
Next smallest test: R1 should probe `Add top nav and delete the hero` and a token-boundary near match, then T3 should start with `Add Contact Us to top nav`.

Session/2026-10-02:
Task and status: R1 cleared; T2 independently reviewed.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `review_probes_preserve_rejection_spans` covers negation, whitespace-separated unsupported tail, and a multibyte near match with exact original-input spans.
Observed red -> green evidence: none; all review probes passed on their first run. This review introduced regression coverage, not a behavior correction.
Validation command/results: filtered review probe passed 1/1; full crate library suite passed 15/15.
Contract decisions or superseded rules: none.
Review performed, findings added/closed, pending checkpoint: R1 clean and cleared; no findings. R2 follows T4.
Known failures/blockers: none in crate library tests.
Next smallest test: T3 `Add Contact Us to top nav` should produce a labelled-addition request with exact label and explicit container target.

Session/2026-10-02:
Task and status: T3 done.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `labelled_addition_preserves_explicit_container_and_label`, `labelled_addition_preserves_omitted_container`, `lowercase_label_spelling_is_preserved`, `labelled_addition_rejects_missing_label_and_incomplete_delimiters`, and `labelled_addition_rejects_unsupported_target_tails`.
Observed red -> green evidence: filtered explicit-container test failed an assertion with `Unsupported(UnrecognizedInput)` versus expected `Parsed(AddLabelledItem)`; after implementation, the full crate library suite passed 20/20. The other T3 cases were added in the same increment and passed in the full run; no separate red run is claimed for them.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 20/20; parser-local `rustfmt --check` and `git diff --check` passed.
Contract decisions or superseded rules: see T3 decisions above. T2 navigation starts retain precedence over labelled additions.
Review performed, findings added/closed, pending checkpoint: no T3 review performed in this session; R2 follows T4.
Known failures/blockers: none in crate library tests.
Next smallest test: T4 `Add Get in Touch to top nav` should retain the full label and explicit target, then a quoted label should select a literal interpretation.

Session/2026-10-02:
Task and status: T3 independent review complete and positive; no findings.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `review_probes_label_spans_and_complete_targets` verifies exact label/target spans, an unknown target rejection, and a trailing instruction rejection.
Observed red -> green evidence: no implementation correction was needed; the review probe passed on its first run.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 21/21; parser-local `rustfmt --check` and `git diff --check` passed.
Contract decisions or superseded rules: none.
Review performed, findings added/closed, pending checkpoint: T3 review clean; no findings. R2 follows T4.
Known failures/blockers: none in crate library tests.
Next smallest test: T4 `Add Get in Touch to top nav` should retain the full label and explicit target.

Session/2026-10-02:
Task and status: T4 done; R2 pending.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `relation_words_inside_unquoted_labels_keep_the_final_container_boundary`, `quoted_labels_keep_syntax_words_and_conjunctions_literal`, `double_quote_escapes_are_decoded_and_other_escapes_rejected`, `called_and_named_item_forms_parse_the_same_labelled_addition`, `navigation_word_is_ambiguous_unquoted_and_literal_when_quoted`, and `unicode_label_and_surrounding_syntax_use_original_byte_spans`.
Observed red -> green evidence: the filtered relation-word test failed for `Add Back to School to top nav` with `Unsupported(UnrecognizedInput)` instead of the expected full label and target. After selecting the final complete target delimiter and adding quoted/introduced-form handling, all 27 crate library tests passed. Other T4 cases were added in the same increment; no separate red runs are claimed for them.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 27/27; parser-local `rustfmt` and `git diff --check` passed.
Contract decisions or superseded rules: see T4 entries above; T3's first-`to` selection is superseded.
Review performed, findings added/closed, pending checkpoint: R2 must review this implementation before T5. No T4 review findings have been recorded yet.
Known failures/blockers: none in crate library tests.
Next smallest test: R2 should probe malformed quote boundaries and multiple `to top nav` phrases; after a cleared R2, T5 should start with `Add a Contact Us page`.

Session/2026-10-02:
Task and status: R2 cleared; T4 independently reviewed.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `review_probes_quoted_boundaries_and_repeated_target_phrase` covers quoted Unicode spans, repeated target phrases, malformed quote adjacency, and an unsupported instruction after a quoted label.
Observed red -> green evidence: none; the review probe passed on its first run.
Validation command/results: filtered review probe passed 1/1; full crate library suite passed 28/28.
Contract decisions or superseded rules: none.
Review performed, findings added/closed, pending checkpoint: R2 clean and cleared; no findings. R3 follows T6.
Known failures/blockers: none in crate library tests.
Next smallest test: T5 `Add a Contact Us page` should produce a distinct page request with the exact display label and no invented route.

Session/2026-10-02:
Task and status: T5 done; independent T5 review pending.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `page_creation_preserves_display_label_and_original_spans`, `lowercase_and_quoted_page_labels_preserve_display_text`, `page_requests_are_distinct_from_navigation_additions_and_literal_page_words`, and `page_requests_reject_missing_parts_and_unsupported_tails`.
Observed red -> green evidence: the initial page tests failed because `Add a Contact Us page` returned `AddLabelledItem` and `Add a` returned a bare labelled addition; after the page branch, the tests passed. A later `Add a Help page page` case failed as `UnsupportedTail` and passed after selecting a final page noun.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 32/32; parser-local `rustfmt --check` and `git diff --check` passed.
Contract decisions or superseded rules: see T5 decision above; the `Add a <label> page` form narrows the former generic labelled-addition interpretation for that exact syntax.
Review performed, findings added/closed, pending checkpoint: independent T5 review is next; R3 remains after T6.
Known failures/blockers: none in crate library tests.
Next smallest test: T6 `Image needs rounded corners` should yield a style request with a role selector and a radius concept.

Session/2026-10-02:
Task and status: T6 done; R3 pending.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `image_style_requests_preserve_unresolved_role_and_concept`, `form_below_hero_keeps_relation_separate_from_relative_padding`, `relative_form_padding_variants_keep_direction_and_source_spans`, `style_requests_reject_unsupported_properties_and_selectors`, `style_requests_reject_incomplete_phrases`, and `style_requests_reject_every_trailing_instruction`.
Observed red -> green evidence: the filtered image-style test first failed because `Image needs rounded corners` returned `Unsupported(UnrecognizedInput)` instead of a typed style request; after the style parser branch, that test and the full crate library suite passed. The negative cases were added afterward and passed on their first run; no separate red run is claimed for them.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 39/39. The parser module was formatted with `rustfmt --edition 2021`.
Contract decisions or superseded rules: see T6 decision above; no earlier rule superseded.
Review performed, findings added/closed, pending checkpoint: R3 must review the full grammar before T7. No T6 review findings have been recorded yet.
Known failures/blockers: none in crate library tests.
Next smallest test: R3 should probe style/page precedence, role relation spans, and supported-command tails, then record its review findings or a clean result.

Session/2026-10-02:
Task and status: R3 cleared; T6 independently reviewed.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `review_probes_page_style_precedence_relations_and_tails` covers page labels containing style words, original-input relation spans with mixed case and surrounding whitespace, and page/style unsupported tails.
Observed red -> green evidence: none; the review probe passed on its first run.
Validation command/results: filtered review probe passed 1/1; full crate library suite passed 40/40.
Contract decisions or superseded rules: none.
Review performed, findings added/closed, pending checkpoint: R3 clean and cleared; no findings. R4 follows T7.
Known failures/blockers: none in crate library tests.
Next smallest test: T7 should add a deterministic adversarial table and repeat-call equality checks before finalizing the parser integration handoff.

Session/2026-10-02:
Task and status: T7 done; R4 completion review pending.
Files changed: `crates/protopie-ui-agent/src/parser.rs`, this epic.
Behavior and named tests added: `adversarial_inputs_have_specified_outcomes_and_valid_byte_spans` specifies empty, punctuation, multibyte, unmatched-quote, adjacent-quote, and repeated-delimiter outcomes while checking every returned span; `repeated_calls_are_equal_across_outcome_kinds` checks stable parsed, clarification, and unsupported results after an interleaved parse. Existing named tests cover the accepted navigation, label, page, and style forms plus ambiguity, negation, malformed input, token boundaries, quotes, unsupported remainders, and exact source spans.
Observed red -> green evidence: none; the new adversarial test passed on its first run and no parser correction was required.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 42/42 with no ignored tests; `rustfmt --edition 2021 --check crates/protopie-ui-agent/src/parser.rs` and `git diff --check` passed.
Contract decisions or superseded rules: the consolidated grammar and deferred syntax above restate T1–T6 decisions; the rightmost `to top nav` rule from T4 remains authoritative. No earlier decision was newly superseded.
Review performed, findings added/closed, pending checkpoint: R4 is next; no T7 review finding is claimed before that review.
Known failures/blockers: none in crate library tests.
Next smallest test: R4 should inspect all promised syntax and spans, probe any uncovered counterexample, and record a clean review or a stable-ID finding with its follow-up task.

Session/2026-10-02:
Task and status: R4 completion review performed; checkpoint not cleared due to required finding F001.
Files changed: this epic only; parser implementation and tests unchanged.
Behavior and named tests added: none; the focused local probe demonstrated `Add a Contact Us page.`, `page,`, and `page!` being parsed as omitted-target labelled additions.
Observed red -> green evidence: no correction attempted in review; F001-T1 proposes the failing unit test.
Validation command/results: `cargo test -p protopie-ui-agent --lib` passed 42/42 with zero ignored tests; `cargo build -p protopie-ui-agent --lib` passed; local executable probe printed the wrong parsed outcomes and a quoted-form rejection.
Contract decisions or superseded rules: none; the recorded T7 punctuation rule already determines expected rejection.
Review performed, findings added/closed, pending checkpoint: R4 inspection complete; F001 open and required; R4 clearance awaits F001-T1 correction and re-review.
Known failures/blockers: F001 page punctuation fallthrough.
Next smallest test: F001-T1 `page_creation_rejects_punctuation_on_page_noun` should assert exact `Unsupported(UnrecognizedInput)` and `0..22` spans for `Add a Contact Us page.`, `page,`, and `page!`.

F001-T1 resolution (2026-10-03): Added `page_creation_rejects_punctuation_on_page_noun`; `parse_page_creation` now returns `Unsupported(UnrecognizedInput)` over the full command when an unquoted final token is `page` plus ASCII punctuation. No red run was observed (fix and test were written together). `cargo test --workspace` passes (59 parser/crate lib tests). F001 closed pending independent R4 re-review.

F001-T1 follow-up (2026-10-03, T2 review findings): the first fix was too narrow. Added `page_creation_rejects_repeated_punctuation_missing_labels_and_plural_noun` first and observed it red (`Add a Contact Us page..` parsed as AddLabelledItem). Fix: `parse_page_creation` now strips all trailing ASCII punctuation from the final token and returns `Unsupported(UnrecognizedInput)` over the full input when the stem is `page` with any punctuation (drops the `tokens.len() > 3` guard, so `Add a page.` / `Add a Page.` are rejected rather than becoming a label `a page.`) or is the plural `pages` (with or without punctuation), consistent with the complete-input rule. Green: `cargo test --workspace` passes (60 crate lib tests); existing tests unchanged. Still awaiting independent R4 re-review.
