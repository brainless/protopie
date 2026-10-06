# MVP multi-file Apply and Vite reload runbook (Epic 003 T6)

Run on macOS with a supported Node/npm installation and a browser that can open
the project's loopback Vite URL. Use the GUI's **preview, then Apply** flow for
every command. Keep the browser developer tools open with **Preserve log** enabled
for Console and Network. In Network, retain the Vite WebSocket frames; save the
frame list and console output after each Apply. Keep the GUI's preview log tail
open. A screen recording with a visible timecode is the simplest way to measure
from the GUI's Applied result to the first correct browser frame.

## Preparation and install measurement

1. Record the date, macOS and browser versions, `node --version`, `npm --version`,
   and the `package-lock.json` revision or hash. Use a clean checkout.
2. Create a fresh project through **New Project**. Confirm its directory has no
   `node_modules`. Start recording, then click **Launch**. Record the times at
   Launch, **Starting** (the end of dependency preparation), **Running**, and
   browser first render. This separates first-launch `npm ci` from Vite startup.
   Record whether npm used a warm cache and save the install output from the
   preview log. Stop the preview before changing projects.
3. For each trial below, create a *new* project and repeat the same prompt and
   answer sequence exactly. Run each scenario three times. Launch it before the
   measured Apply and wait for the initial page to render. Repeated trials must
   not reuse a project because page creation and context commands have different
   outcomes after their first application.

## Route and navigation trial

In each fresh project, submit `Need a top navigation`, review its diff, and
Apply. With the browser at `/`, submit `Add Contact Us`, answer `new page`,
review the resulting multi-file diff, and Apply. The expected final state is a
visible Contact Us navigation link that opens `/contact-us`, and a direct load of
`/contact-us` that renders the new page. Use Back and Forward after clicking.

Record the ordered Vite WebSocket frames and Vite log lines during Apply, any
overlay or console error (including its duration), whether the page performed
HMR or a full reload, and whether the new route ever returned a stale module or
blank view. Record the GUI Applied time and first visible correct link and page
times. If an overlay appears, capture its exact message and the file it names.

## Context and provider trial

In each fresh project, review and Apply `Add a Doctors page`, then `Add a Visits
page`. With `/doctors` open in the browser, submit `Share the selected doctor
across pages` and answer, in order: `text`, `Dr. Rao`, `doctors`, `done`. Review
the multi-file diff and Apply. The expected final state is a provider around the
router layout, a visible shared value on `/doctors`, and a working `/visits`
route, including direct loads and Back/Forward navigation.

Record the same event sequence, overlays, console errors, reload type, and
Applied-to-visible time as in the route trial. Specifically note whether a
consumer imports a context module before it exists or renders before the
provider wraps the route.

## Observation record

Keep one row per trial, including trials without errors:

| Trial | Lockfile, Node/npm, browser | Launch → Starting (install) | Starting → Running | Apply → visible | Vite frames/log sequence | HMR or full reload | Overlay/console and duration | Route/DOM result |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Route 1 | | | | | | | | |
| Route 2 | | | | | | | | |
| Route 3 | | | | | | | | |
| Context 1 | | | | | | | | |
| Context 2 | | | | | | | | |
| Context 3 | | | | | | | | |

Classify a frame as an update, full reload, or error using its actual payload;
preserve the order and timestamps. If an intermediate failure occurs, save the
recording and the affected generated files. Reproduce it before changing the
apply protocol. Any mitigation must preserve `.protopie/journal.json` recovery:
run `env RUSTC_WRAPPER= cargo test -p protopie-ui-agent apply::tests --lib` and
add a regression that exhibits the observed browser failure. A rename-order
change only counts as a fix when the repeated browser trial confirms it.
