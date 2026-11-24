# Watch (experimental)

State what you are trying to get done in a project. The console follows the
runs the ledger witnesses and tells you when one of them needs a decision.
The notice leads to the ledger lines that caused it.

This is phase 1 of a larger idea (the console as the place you are told what
needs you). It is an experiment: the module is **off by default** and nothing
in Testigo depends on it.

## Switch it on

Command palette → **Configure Modules** → **Watch**. The panel appears under
**Work → Agents → Watch**, and as the action `nav.watch`.

## What it does today

- **One goal, in your words.** "Close this PR today." A goal belongs to the
  open project and follows that project's ledger from the moment it starts;
  earlier runs are ignored.
- **One condition.** A run fails a recognized test or check: a `check_run`
  ledger line with status `failed`. That is a fact the ledger already
  records (runner, exit code, hash of the output). `unknown`, `empty` and
  `interrupted` checks do not alert. A runner piped into another command
  (`cargo test | tail -20`) ends with that command's exit code, so it is
  `unknown` whatever the tests did: run it bare, or under `set -o pipefail`,
  for a failure to be seen.
- **One alert per run.** The run is the turn. A second failed check in the
  same turn joins the alert as more evidence and notifies nobody.
- **Evidence in a click.** An alert opens the turn's ledger lines (prompt,
  snapshot, tool calls, checks, close) with the triggering lines marked, and
  **Open this run in Proof** takes you to the same turn in the ledger view.
- **Questions with a referent.** "¿Qué pasó en esa ejecución?" / "What
  happened in that run?" is answered from the ledger lines of the alert you
  have open. With several alerts and none open, you are shown the candidates.
  If the ledger lacks something (no snapshot, run still open, exit code not
  reported) the answer says what is missing. Anything else gets "I cannot
  answer that yet". There is no model in this phase.
- **Pause, stop, delete.** Pause keeps the cursor: resuming evaluates what
  was skipped. Stop is final and keeps the alerts for reading.

## Notifications

When an alert is created and the window is not focused, the console sends
one desktop notification. With the window focused there is a toast instead.
The alert is saved as notified **before** the notification is sent, so a
crash or a restart can lose a notification but never repeats one.

## What happens while the app is closed

Nothing is evaluated while the app is closed: there is no background
service. Nothing is lost either.

- Each goal stores the last ledger `seq` it evaluated (`cursorSeq`, in
  `watch.json` under the app's data directory).
- Ledger lines written while the console was closed are on disk like any
  other. In practice that is the hook events of agent sessions that kept
  running, which the console ingests on launch.
- The first pass after launch compares each active goal's cursor with the
  ledger head and evaluates everything in between. Alerts found this way are
  labelled "happened while the app was closed" and arrive as a single
  notification that counts them.
- A run that failed while the app was closed and that the ledger never
  witnessed does not exist for the watch. "No alerts" means "no failed check
  in what the ledger saw", and the panel says so.

## Demo

**Start demo** follows a scripted timeline instead of the ledger: one run
that passes its tests, then one that fails them twice, 30 seconds after you
start. Switch to another window and wait for the notification.

The demo events are a function of the goal's start time and the clock. They
are never written to a ledger, they keep "happening" with the window in the
background, and after a restart the ones already due are simply there.

To check the phase by hand:

1. Start the demo and put the console in the background.
2. About 30 s later one notification arrives. A second failed check lands
   4 s after it and does not notify.
3. Open the alert, read the evidence, ask "¿qué pasó en esa ejecución?".
4. Quit and reopen. The goal, the alert and the thread are there; no new
   notification arrives.

## Limits of this phase

- A goal is linked to every run of its project, not to a PR or a branch.
- No detection of two agents touching the same file yet.
- No voice. No model. No task preparation.
- Switching the module off hides the panel; it does not silence a goal that
  is still active. Pause or stop the goal for that.
- Goals, alerts and the thread are not ledger events: the ledger is not
  written by this feature.
