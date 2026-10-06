# Project diagnostics

The GUI and server append project-specific JSONL events under the repository's ignored `.logs/` directory. Each directory is the SHA-256 digest of the project's canonical path; its `project.json` records that path so you can identify it. Run `rg --no-ignore -n 'canonical_project_path' .logs` from the repository root to list projects with diagnostics.

The current log is `events.jsonl`; `events.1.jsonl` and `events.2.jsonl` are older copies. Each file is limited to 1 MiB. Writes are locked across GUI and server processes and each line is valid JSON. On Unix the project directory is private (`0700`) and new files are private (`0600`). Logging failures do not interrupt edits.

The server records modify and answer request inputs, dry-run and expected plan/revision fields, outcome kind, question IDs, and pending question IDs/options before and after the request. The GUI records the input sent, the response it displayed, and Discard clicks. GUI and server entries share the request ID, including dry runs. Preview text in the log omits code diffs; source files and diff bodies are never logged. Prompts and answers can contain personal or sensitive text, so inspect logs locally before sharing them.

To capture the sharing-question failure:

1. Create or select a project, then `Add a Doctors page` and Apply.
2. Send `Share the selected doctor across pages` and click **Text** when prompted. Continue with the value and page steps if the UI permits.
3. Find the project's `project.json`, then read its `events.jsonl` from top to bottom. Match GUI `request`/`response` and server `modify_request`/`modify_result` or `answer_request`/`answer_result` by `request_id`. The server's `pending_before` and `pending_after` fields show whether the question offered by the GUI was actually stored.
