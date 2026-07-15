# Demo runbook

This runbook is optimized for a ten-minute meetup or conference slot. The
reliable stage path uses the fixture backend and a controlled Workflow
checkpoint, so the failure happens at a legible moment every time.

## The story in one sentence

Process memory forgets; Temporal does not. The naïve process loses its pending
reply, while Temporal retains the durable agent's event history and a release
Signal sent with no Worker online.

## Roles of the three services

- **Stage** remains running and serves the dashboard at port `3000`.
- **Temporal** remains running and stores Workflow history at port `7233`.
- **Worker** is the only service killed during the durable demonstration.

The opening `naive` binary runs in a separate temporary container. It is not a
fourth service and never connects to Stage or Temporal.

Do not kill the Stage or Temporal container for the main recovery beat. That
would demonstrate a different failure mode and make the dashboard harder to
interpret.

## Before event day

Build, verify, and rehearse from the exact commit that will be used on stage:

```sh
make test
make lint
make build
docker compose down -v
make up
make status
```

The `down -v` above is deliberately destructive: it clears old Temporal and
Stage volumes so Temporal Web starts clean. Use it for rehearsal preparation,
not when history must be preserved.

Verify both pages:

- <http://localhost:3000>
- <http://localhost:8233>

Perform the entire kill/restart sequence at least once on the presentation
machine. Then build the final images before leaving reliable internet:

```sh
MODEL_PROVIDER=fixture docker compose up --build -d
docker compose images
```

Fixture mode needs no external model connection after the images exist.
Rehearse the naïve opening from two terminals as well; `make naive-run` is
supposed to block until the other terminal kills it.

## Presentation setup

Use two terminal tabs, two browser tabs, and optionally a third terminal for
logs.

Terminal A — naïve process first, then durable status/restart commands:

```sh
make status
```

Terminal B — kill commands. Optional Terminal C — logs:

```sh
make logs
```

Browser tabs:

1. Stage dashboard at <http://localhost:3000>
2. Temporal Web at <http://localhost:8233>

Start or confirm the fixture stack:

```sh
export MODEL_PROVIDER=fixture
export SHOW_RAW_CONFESSIONS=false
export ALLOW_UNMODERATED_TWILIO=false
unset OPENAI_API_KEY
make up
curl -fsS -o /dev/null http://localhost:3000/healthz
make reset-demo
```

Before clearing its projection, reset releases unfinished Workflows from the old
session and waits up to 12 seconds for them to reach `Sent` or `Failed`. Only
then does it create a new session with no displayed submissions and **Hold
before reply** enabled. Stage prevents new admissions from interleaving with
that drain. Reset does not delete old histories from Temporal; use
`docker compose down -v` before the event if a clean Temporal UI matters.

Confirm the status strip shows:

- Worker: **Online**
- Temporal: **connected**
- Model: **Fixture**
- Replies held
- No red **Raw input mode** banner

The Worker heartbeat is sent once per second and considered offline after three
seconds without a heartbeat.

Keep `SHOW_RAW_CONFESSIONS=false` for public input, especially Twilio. In this
mode incoming cards use a neutral placeholder until the agent returns a
stage-safe paraphrase. Set the flag to `true` only for presenter-controlled,
rehearsed text that is already safe to display; it immediately stores and serves
normalized incoming text and does not replace it with the paraphrase. A red
warning banner remains visible across the dashboard while raw mode is active.

For a trusted-input rehearsal that specifically needs text to appear at intake:

```sh
export SHOW_RAW_CONFESSIONS=true
docker compose up -d --force-recreate stage
```

If Twilio variables are configured, Stage refuses to start in raw mode unless
`ALLOW_UNMODERATED_TWILIO=true` is also explicit. Do not use that override for a
public event. For trusted rehearsals, disable Twilio instead of bypassing the
guard.

Before admitting public input, set it back to `false`, recreate Stage, and reset
the projection. Changing the flag alone does not remove raw rows already stored.

## Opening beat: show process memory fail

This takes about one minute and uses the known-safe seeded confession. It does
not touch the dashboard or the durable Workflows.

In Terminal A:

```sh
make naive-run
```

The fixture agent prints `RECEIVED`, `PLANNING`, `TOOL`, and `COMPOSING`, then
stops here:

```text
REPLY PENDING  memory only — kill this container now
Pending confessions in this process: 1
```

While that command remains attached, run in Terminal B:

```sh
make naive-forget
```

Terminal A exits non-zero because `SIGKILL` is intentional. Then, in Terminal A:

```sh
make naive-restart
```

The fresh process has no input or external history and prints:

```text
Recovered pending confessions: 0
Nothing to resume—the process memory is empty.
```

Speaker line:

> The agent did the work, but its only memory died with its process.

Now move to the dashboard. The agent loop remains recognizable, but Temporal
owns its progress and the Worker becomes disposable.

## Main durable recovery sequence

### 1. Build the pending work

Keep **Hold before reply** on. Submit one confession manually, then either invite
more submissions through the form or click **Seed examples**. With the required
public-event setting `SHOW_RAW_CONFESSIONS=false`, raw form input is not echoed
onto the feed: the card shows a neutral placeholder until the agent returns its
stage-safe `display_confession`. The seed button adds:

- “I fixed the race condition with a sleep.”
- “I clone everything until it compiles.”
- “I wrote a Python script that now runs the company.”

Wait until each visible item reaches **Reply Pending**. At this point the
Workflow has durably recorded its plan and judgment and is waiting for a
Signal. It has not run the delivery Activity, and the Hall of Shame still has no
eligible winners because awards consider only `Sent` rows.

If helpful, open Temporal Web and search for the `rust-confession-` Workflow ID
prefix. Temporal Web is outside the dashboard privacy guard: Workflow input and
the plan/compose Activity payloads contain raw confession text. On the projector,
open only a seeded or speaker-owned safe Workflow, and pre-open the exact event
you intend to show. Never click into an arbitrary audience payload live.
Lookup receives only the typed plan and delivery receives only the submission
ID, so neither of those Activity payloads redundantly contains the confession.

### 2. Kill only the Worker

In Terminal B:

```sh
make kill-worker
```

This sends `SIGKILL` to the `worker` container. It is intentionally not a
graceful shutdown. Confirm:

```sh
make status
```

Within roughly three seconds the dashboard's Worker indicator changes to
**Offline**. The confession cards remain because the Stage projection is stored
separately, and the authoritative Workflow history remains in Temporal.

### 3. Release while the Worker is offline

Turn **Hold before reply** off in the dashboard while the Worker is still dead.

This action sends a `release` Signal to every current, unfinished Workflow.
Temporal accepts and records the Signals. The cards stay at **Reply Pending**
because no Worker is available to process the new Workflow Tasks.

Speaker line:

> The command arrived while there was no Rust process available to hear it.

### 4. Restart the Worker

In Terminal A:

```sh
make restart-worker
```

Watch the dashboard. The heartbeat returns, and the Workflows replay their
history, observe the already-recorded Signal, and move through **Sending** to
**Sent**. No confession is resubmitted and no plan is reconstructed from the
dashboard. As rows become `Sent`, the Hall of Shame winners appear; this ties the
award reveal directly to successful recovery and delivery.

If the dashboard transitions are too fast, use the preselected seeded or
speaker-owned Workflow in Temporal Web to point out the Signal and the
post-restart Activity. Do not open arbitrary audience payloads on the projector.

### 5. Land the explanation

The concepts map cleanly to what the audience just saw:

1. The **Client** in Stage started each Workflow and sent the release Signal.
2. The **Workflow** retained typed state and controlled the deterministic loop.
3. **Activities** performed model, catalog, projection, and delivery work.
4. The **Worker** executed those definitions but owned none of the durable
   progress.

Call attention to the three score-derived awards after completion. They were
hidden while the Workflows waited and are selected only from `Sent` rows. They
require no second interaction or final model request; the scores were returned
with the judgments already processed.

To repeat the sequence without resetting, turn **Hold before reply** back on
before accepting more submissions. Otherwise new Workflows begin in the
released state and proceed directly to `Sent`.

## Suggested ten-minute pacing

| Time | Beat |
| ---: | --- |
| `0:00` | Run the naïve agent to `Reply Pending` |
| `0:40` | Kill it, restart it, and show zero recovered |
| `1:15` | Open the durable dashboard and submit the first confession |
| `2:00` | Explain the loop; name Client, Workflow, Activity, and Worker |
| `3:15` | Seed or accept more confessions; wait for `Reply Pending` |
| `4:30` | Kill the Worker and let the offline indicator land |
| `5:15` | Release replies while it is offline |
| `6:00` | Restart and watch the same Workflows finish |
| `7:15` | Inspect the preselected safe Temporal history |
| `8:15` | Read the Hall of Shame awards |
| `9:15` | Recap and leave buffer |

## Direct API controls

The browser is preferred on stage, but the same actions can be driven from a
terminal.

Submit one confession:

```sh
curl -fsS -X POST http://localhost:3000/api/confessions \
  -H 'content-type: application/json' \
  -d '{"text":"I fixed the race condition with a sleep."}'
```

Enable the hold:

```sh
curl -fsS -X POST http://localhost:3000/api/demo/hold \
  -H 'content-type: application/json' \
  -d '{"held":true}'
```

Release all current unfinished Workflows:

```sh
curl -fsS -X POST http://localhost:3000/api/demo/hold \
  -H 'content-type: application/json' \
  -d '{"held":false}'
```

Seed examples:

```sh
curl -fsS -X POST http://localhost:3000/api/demo/seed
```

Inspect public state (`jq` is optional):

```sh
curl -fsS http://localhost:3000/api/state | jq
```

Start a fresh dashboard session:

```sh
make reset-demo
```

## Audience access

Compose deliberately publishes Stage only at `127.0.0.1:3000`. Other audience
devices cannot open the dashboard directly, and Temporal's ports are likewise
loopback-only. Use the presentation machine for browser-form submissions.

The dashboard includes unauthenticated reset, seed, and hold controls. Do not
change the mapping to `0.0.0.0:3000` or create a tunnel for the whole Stage
service merely to collect audience input; doing so would expose those operator
controls as well.

For audience SMS, configure the optional signed Twilio webhook as described in
[the README](../README.md#optional-inbound-sms-twilio). Put a path-restricted
HTTPS reverse proxy in front of only `/webhooks/twilio/messages`; a normal tunnel
to port `3000` exposes the entire Stage service and is not suitable.

Explicitly keep `SHOW_RAW_CONFESSIONS=false` for public SMS input. Enabling raw
display would persist incoming message bodies in the Stage volume and expose
them through `/api/state` before the agent can produce its safe paraphrase.
Stage enforces this combination: with Twilio configured, raw mode fails startup
unless the dangerous `ALLOW_UNMODERATED_TWILIO=true` override is also set.

Send a real test message before doors open and confirm that a placeholder card
appears, then becomes a stage-safe judgment. The integration accepts inbound
message bodies but sends no SMS response; judgments remain dashboard output.
Phone numbers are validated but not retained. STOP/START/HELP-family compliance
commands are acknowledged without appearing as confessions.

## Fallbacks

### Audience input is quiet or networking fails

Click **Seed examples**. The demo remains complete with the three bundled
confessions. Fixture mode and localhost do not depend on venue networking after
the Docker images are built. This is also the fallback if the Twilio number,
public proxy, or venue cellular service is unavailable.

### OpenAI is slow, unavailable, or produces a failed Workflow

Switch the Worker to fixture mode:

```sh
export MODEL_PROVIDER=fixture
unset OPENAI_API_KEY
docker compose up -d --force-recreate worker
```

Then reset the dashboard and seed new examples:

```sh
make reset-demo
curl -fsS -X POST http://localhost:3000/api/demo/seed
```

A model Activity has at most three attempts. Replacing the Worker backend can
help retries that remain open, but it does not revive a Workflow execution that
has already failed.

### Worker does not show offline

Wait three seconds, then verify the container is stopped:

```sh
make status
docker compose logs --tail=100 worker
```

Do not stop the entire Compose project; the Stage needs to keep polling and
Temporal needs to record the offline Signal.

### Worker does not recover

```sh
make restart-worker
make status
docker compose logs --tail=100 worker temporal
```

Confirm the task queue is `rust-confessional` in both containers and that
Temporal remains reachable. If time is short, keep the offline screen visible
and explain the pending Signal verbally rather than debugging live. Use Temporal
Web only if the preselected seeded or speaker-owned event is already open.

### Dashboard is stale

Refresh the page and check the backing API:

```sh
curl -fsS http://localhost:3000/api/state
docker compose logs --tail=100 stage
```

The browser polls approximately every 700 milliseconds. A Stage restart reloads
its JSON projection from the `stage-data` volume, but status reports missed while
Stage was unavailable are not rebuilt automatically from Workflow history.

### Total presentation failure

Keep a short screen recording or two screenshots from the final rehearsal:

1. Confessions at `Reply Pending` with Worker offline
2. A seeded or speaker-owned Workflow history after restart, with submissions
   at `Sent`

The code walkthrough and those two states still tell the durability story.

## Reset and cleanup semantics

`make reset-demo` sends a release Signal to each current, unfinished Workflow
and waits up to 12 seconds for every current row to reach `Sent` or `Failed`.
Only then does it clear the Stage projection and create a new session ID. If a
Signal fails, Temporal is unavailable, the Worker is offline, or the drain times
out, reset returns an error and preserves the current dashboard session; restore
the Worker and retry. An admission lock prevents browser submissions, seed
requests, hold changes, and Twilio submissions from interleaving with the drain.
Do not intentionally submit while resetting. Old Temporal histories remain
after a successful reset.

`make down` removes containers and the Compose network but retains named
volumes. Starting again restores both Temporal history and Stage data.

The following is the true clean slate and cannot be undone:

```sh
docker compose down -v
```

After the talk, stop any public webhook proxy and remove credentials from the
shell environment:

```sh
unset OPENAI_API_KEY TWILIO_AUTH_TOKEN TWILIO_ACCOUNT_SID TWILIO_WEBHOOK_URL
unset SHOW_RAW_CONFESSIONS ALLOW_UNMODERATED_TWILIO
```
