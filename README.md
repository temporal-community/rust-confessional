<div align="center">

[![Rust](https://img.shields.io/badge/Rust-1.88%2B-dea584?logo=rust&logoColor=white)](Cargo.toml)
[![Temporal Rust SDK](https://img.shields.io/badge/Temporal_Rust_SDK-0.5.0-635bff)](https://github.com/temporalio/sdk-rust)
[![Docker Compose](https://img.shields.io/badge/Run_with-Docker_Compose-2496ed?logo=docker&logoColor=white)](compose.yaml)
[![Watch the talk](https://img.shields.io/badge/Watch-the_talk-ff0033?logo=youtube&logoColor=white)](https://youtu.be/_t_Rxf8Z4mU?si=bqvJKqz_hZe2OheV)

</div>

# Wall of Regrets

**An audience-powered AI agent that keeps its place while the Rust Worker comes
and goes.**

People submit a programming confession from a phone or browser. Ferris plans a
response, consults an approved Rust remedy catalog, and returns a judgment,
prescription, and penance. The agent loop runs in Rust. Temporal keeps its
progress durable between tasks.

The live talk used the name **Rust Confessional**. The booth display is the
**Wall of Regrets**. They are the same demo.

> **See it in action:** Watch the
> [Rust meetup talk](https://youtu.be/_t_Rxf8Z4mU?si=bqvJKqz_hZe2OheV), play the
> [10-second deploy-and-resume walkthrough](docs/media/wall-of-regrets-walkthrough.mp4),
> or [download the slide deck](docs/media/rust-can-fix-that-slides.pdf).

![A side-by-side walkthrough of Wall of Regrets and Temporal Web during a Worker redeploy](docs/media/wall-of-regrets-walkthrough.gif)

### Key demo moments

| Moment | What happens | Why it matters |
| --- | --- | --- |
| **Audience input** | Attendees submit confessions from a phone or browser | The audience becomes part of the system, not merely spectators |
| **Visible agent loop** | Ferris decides, uses a skill, folds the result into state, and repeats | The basic AI loop stays small enough to explain on stage |
| **Durable human wait** | Each Workflow parks at `Reply Pending` until an operator releases it | Waiting for a person does not require a dedicated live process |
| **Routine Worker redeploy** | The old Worker stops, a Signal arrives during the gap, and a fresh Worker resumes | Process lifetime is separate from interaction lifetime |
| **Retry recovery** | A simulated rate limit fails twice and succeeds on the third attempt | Rust classifies the error; Temporal owns durable backoff |
| **Booth mode** | A passive wall shows new judgments and Hall of Shame awards | The same durable backend works as an ongoing interactive installation |

### Why this architecture?

> **Process-only approach:** pending work lives in memory. Replace the process
> and the replacement starts empty.
>
> **Durable approach:** Workflow state and Signals live in Temporal history. A
> compatible replacement Worker replays that history and continues.

![Animated comparison of process memory and Temporal history during the same Worker redeploy](docs/media/durable-agent-redeploy.gif)

The animation shows the talk-sized thesis: **Rust runs the work. Temporal
remembers where it was.**

## The problem

An agent loop is straightforward:

1. Observe the current state.
2. Decide what to do next.
3. Run a model call or tool.
4. Fold the result into state.
5. Repeat until the goal or a stop condition is reached.

Everything around that loop is harder. Model APIs rate-limit. Humans reply
later. Workers are redeployed. Networks disappear. If the only copy of the
agent's progress is in process memory, every interruption becomes custom
recovery code.

This demo keeps the loop ordinary Rust and moves the reliability boundary into
Temporal Workflows, Activities, Signals, retries, and event history.

## 60-second quickstart

The repository is intentionally Docker-first. You do not need Rust, Cargo, or a
Temporal server installed on the host.

Prerequisites:

- Docker Engine or Docker Desktop with Docker Compose v2
- Loopback ports `3000`, `7233`, and `8233`
- Network access for the first image pull and Rust dependency build

Start the deterministic fixture version:

```sh
make up
make status
curl -fsS -o /dev/null http://localhost:3000/healthz
```

Open:

| URL | Purpose |
| --- | --- |
| <http://localhost:3000> | Stage dashboard and operator controls |
| <http://localhost:8233> | Temporal Web and Workflow history |
| <http://localhost:3000/?view=wall> | Passive Wall of Regrets booth display |

Stop the stack while retaining both named volumes:

```sh
make down
```

## Run the durable demo beat

The dashboard starts in autonomous, per-confession mode with **Hold before
reply** enabled.

1. Submit a confession and watch the card move through the agent steps.
2. Wait for `Reply Pending`. The Workflow is durable and no Workflow task is
   actively executing while it waits.
3. Stop the old Worker to open a deliberately visible deployment gap:

   ```sh
   make begin-redeploy
   ```

4. Turn **Hold before reply** off while no Worker is polling. Temporal records
   the durable `release` Signal.
5. Start a fresh compatible Worker:

   ```sh
   make finish-redeploy
   ```

The replacement Worker replays Workflow history, sees the Signal, and continues
through `Sending` to `Sent`. The confession is not resubmitted, and completed
Activities are not rerun.

Full presenter cues and fallback paths are in the
[demo runbook](docs/DEMO_RUNBOOK.md).

## How it works

![Architecture showing audience input, the Rust Stage, Temporal history, the Rust Worker loop, and Activities](docs/media/durable-agent-architecture.svg)

### The key insight

The Rust Worker is compute, not the source of truth. The Workflow's inputs,
Activity results, durable state transitions, and Signals are represented in
Temporal history. A Worker can disappear between tasks without taking the
interaction with it.

The three boundaries are intentionally explicit:

| Boundary | Owns | Why |
| --- | --- | --- |
| **Workflow** | Deterministic decisions, status, agent findings, judgment, release state | Replay reconstructs the same durable object |
| **Activity** | Model calls, catalog lookup, projection updates, delivery | Side effects get timeouts, retries, and typed errors |
| **Signal** | Human input that may arrive at any time | Input is recorded even when no Worker is available |

### The agent loop in Rust

The autonomous path is a bounded decide-and-act loop. The production code lives
in [`src/workflows.rs`](src/workflows.rs); this condensed sketch shows its shape:

```rust
for iteration in 0..MAX_AGENT_STEPS {
    let step = ctx
        .start_activity(
            ConfessionalActivities::decide_next_step,
            decide_input(iteration),
            durable_activity_options(30),
        )
        .await?;

    match step {
        AgentStep::Lookup { skill, .. } => run_skill(ctx, skill).await?,
        AgentStep::Compose | AgentStep::Revise { .. } => compose(ctx).await?,
        AgentStep::Finish => break,
    }
}

ctx.wait_condition(|state| state.released).await;
deliver(ctx).await?;
```

The model may choose the next approved step, but it cannot invent arbitrary
tools or run forever. The loop has a deterministic cap and every tool call sits
behind a typed Activity boundary.

## Demo controls and modes

| Control | Default | Use it to show |
| --- | --- | --- |
| **Autonomous agent** | On | A bounded model-driven decide, act, observe loop |
| **Linear agent** | Off | The fixed plan, remedy lookup, compose pipeline |
| **Per confession** | On | One production-shaped Workflow per submission |
| **Aggregate workflow** | Off | One durable session object for stage visualization |
| **Hold before reply** | On | A durable human-in-the-loop checkpoint |
| **Fixture model** | On | Repeatable offline behavior for talks and booths |
| **Show raw confessions** | Off | Stage-safe agent paraphrases instead of raw audience text |

Switching between per-confession and aggregate Workflow modes resets the current
session so the two shapes never interleave.

### Model modes

#### Fixture mode

Fixture mode is the stage-safe default:

```sh
MODEL_PROVIDER=fixture docker compose up --build -d
```

It classifies by keyword, uses the bundled remedy catalog, and adds short
simulated delays so the pipeline remains visible. It is deterministic and makes
no model-provider network calls after the image is built.

#### OpenAI mode

OpenAI mode uses structured Responses API calls for planning, deciding, and
composing. Supply a model your account can access:

```sh
export MODEL_PROVIDER=openai
read -rsp "OpenAI API key: " OPENAI_API_KEY
export OPENAI_API_KEY
export OPENAI_MODEL="YOUR_MODEL_ID"
docker compose up --build -d
```

Requests use strict JSON schemas, a configurable 12-second HTTP timeout, and
`store: false`. The model-backed Activities use durable retry options so
temporary provider outages do not fail the interaction immediately.

To return an existing stack to fixture mode:

```sh
export MODEL_PROVIDER=fixture
unset OPENAI_API_KEY
docker compose up -d --force-recreate worker
```

## Audience and booth experience

The normal dashboard accepts browser submissions and exposes operator controls.
The wall view is a passive display designed for a booth:

![Wall of Regrets booth display with the committed dummy QR and award leaders](docs/media/wall-of-regrets.png)

Recommended booth setup:

```sh
export MODEL_PROVIDER=fixture
export MAX_SUBMISSIONS_PER_SESSION=100
export SHOW_RAW_CONFESSIONS=false
docker compose up --build -d
```

Then:

1. Keep **Per confession** mode enabled.
2. Turn **Hold before reply** off.
3. Put `/?view=wall` full-screen on the public display.
4. Keep `/` open on the operator laptop.
5. Test reset and hold controls before doors open.

The wall is a presentation layout, not an authorization boundary. Keep the
dashboard on loopback and never expose its unauthenticated controls to a LAN or
the internet.

### Optional inbound SMS with Twilio

Twilio input is inbound-only. Audience texts become submissions and results
appear on the wall; the demo does not send an SMS response. You need your own
Twilio account and number. The committed QR contains a placeholder on purpose.

<details>
<summary><strong>Signed webhook setup</strong></summary>

Set the account, auth token, and exact public webhook URL:

```sh
export TWILIO_ACCOUNT_SID="AC..."
read -rsp "Twilio auth token: " TWILIO_AUTH_TOKEN
export TWILIO_AUTH_TOKEN
export TWILIO_WEBHOOK_URL="https://YOUR_PUBLIC_HOST/webhooks/twilio/messages"
export SHOW_RAW_CONFESSIONS=false
export ALLOW_UNMODERATED_TWILIO=false
docker compose up -d --force-recreate stage
```

Configure the number's inbound message webhook to send an HTTP `POST` to the
same URL.

The endpoint validates `X-Twilio-Signature` and `AccountSid`, deduplicates on
`MessageSid`, and ignores STOP, START, and HELP-family messages. It validates
sender and recipient fields but never retains or logs phone numbers.

Expose only `/webhooks/twilio/messages` through a path-restricted HTTPS reverse
proxy. Do not tunnel the whole Stage service.

</details>

<details>
<summary><strong>Outbound-only API polling</strong></summary>

Polling is useful when a locked-down host cannot receive a public webhook. It
only makes outbound HTTPS calls to `api.twilio.com`.

```sh
export TWILIO_ACCOUNT_SID="AC..."
export TWILIO_API_KEY_SID="SK..."
read -rsp "Twilio API key secret: " TWILIO_API_KEY_SECRET
export TWILIO_API_KEY_SECRET
export TWILIO_NUMBER="+15551234567"
export TWILIO_POLL_SECONDS=4
docker compose up -d --force-recreate stage
```

Leave `TWILIO_WEBHOOK_URL` unset for polling-only operation. On its first
successful request the poller baselines the existing backlog, then accepts only
new messages. The latency is up to `TWILIO_POLL_SECONDS`.

</details>

<details>
<summary><strong>Generate the live SMS QR locally</strong></summary>

Never commit a real event number. Public numbers attract spam and inbound
charges, and Git history is permanent.

```sh
pip install segno
python tools/gen_qr.py "+15551234567"
git update-index --skip-worktree static/confess-qr.svg
```

Restore normal tracking later with:

```sh
git update-index --no-skip-worktree static/confess-qr.svg
```

</details>

## Safety and privacy

Treat submissions as public conference content, not secrets.

- Raw confession text is stored in Temporal Workflow history.
- With `SHOW_RAW_CONFESSIONS=false`, the Stage projection stores a neutral
  placeholder followed by the agent's display paraphrase.
- OpenAI mode sends confession text and accumulated agent context to the
  configured model provider.
- Temporal Web may expose raw Workflow and Activity payloads even when the wall
  is in safe display mode. Do not project arbitrary payloads.
- The stage-safe display field reduces accidental raw projection. It is not a
  moderation service or enforceable content policy.
- The demo has a 500-character input limit and a default cap of 20 submissions
  per session, but no identity, authentication, per-client rate limiting,
  moderation queue, or deletion workflow.
- Anyone who can reach the dashboard can view submissions and use its controls.
- Never commit API keys, Twilio credentials, or a live phone number.

<details>
<summary><strong>Trusted-input raw display mode</strong></summary>

For rehearsals with presenter-controlled input:

```sh
export SHOW_RAW_CONFESSIONS=true
docker compose up -d --force-recreate stage
```

Stage strips control characters, collapses whitespace, applies your optional
`MASK_WORDS`, and rejects empty normalized text. These guards do not catch
creative spellings, context, or personal information.

Stage fails closed if Twilio and raw display are enabled together. The
`ALLOW_UNMODERATED_TWILIO=true` escape hatch exists for controlled integration
testing, not public events.

Return to the safe default and clear the projection with:

```sh
export SHOW_RAW_CONFESSIONS=false
export ALLOW_UNMODERATED_TWILIO=false
docker compose up -d --force-recreate stage
make reset-demo
```

Changing the flag does not scrub existing Stage rows or Temporal history.

</details>

## Reliability playbook

### 1. Open with the process-memory contrast

The image also contains a deliberately non-durable agent:

```sh
# Terminal A
make naive-run

# Terminal B, after REPLY PENDING appears
make naive-redeploy

# Terminal A
make naive-restart
```

The replacement reports zero recovered confessions because the pending reply
existed only in the old process.

### 2. Park and redeploy the durable agent

Use the `begin-redeploy` and `finish-redeploy` sequence described in
[Run the durable demo beat](#run-the-durable-demo-beat). A Signal can arrive
during the gap because Temporal, not the Worker process, owns the Workflow
history.

This demonstrates replacement with the same replay-compatible code. It does
not demonstrate arbitrary Workflow code changes or Worker Versioning.

### 3. Recover from a transient dependency failure

Submit "the API keeps rate-limiting my agent" or click **Rate-limit demo**. The
`compose` Activity returns a retryable error twice, then succeeds on its third
attempt. The card remains in `Composing` while Temporal backs off and retries.

### Optional infrastructure variations

Simulate a network partition:

```sh
make partition-worker
make heal-worker
```

The Worker process stays alive but cannot reach Temporal or Stage. Pending tasks
are redelivered when the connection returns.

A hard-crash variation is also available:

```sh
make kill-worker
make restart-worker
```

The routine redeploy story is the recommended talk path because it is more
representative of everyday production operations.

## Design choices worth taking away

- **One Workflow per confession.** Each submission gets a stable, readable
  Workflow ID. Temporal scales by running many Workflows.
- **Deterministic orchestration.** Workflow code decides what happens. Model
  calls, catalog lookup, reporting, and delivery run as Activities.
- **Typed durable state.** Plans, findings, steps, judgments, and release state
  are Rust values reconstructed by replay.
- **Explicit failure semantics.** Retryable and permanent errors are distinct
  Rust types with operation-specific timeouts and retry policies.
- **At-least-once side effects.** Delivery is designed for deduplication by
  submission ID before increasing its attempt budget.
- **Replay-aware upgrades.** All `temporalio-*` crates are pinned to `=0.5.0`.
  Treat SDK and Workflow-code upgrades as deliberate compatibility work.

The Temporal Rust SDK is in Public Preview, so APIs may evolve. Read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design boundaries, failure
behavior, and production gaps.

## Developer reference

### Useful commands

| Command | Purpose |
| --- | --- |
| `make up` | Build and start Temporal, Stage, and Worker |
| `make down` | Stop the stack and retain named volumes |
| `make build` | Build the Docker application image |
| `make test` | Run `cargo test --locked` in the Docker test target |
| `make lint` | Run formatting and Clippy with warnings denied |
| `make logs` | Follow Stage and Worker logs |
| `make status` | Show Compose service state |
| `make reset-demo` | Release unfinished Workflows and begin a fresh session |
| `make begin-redeploy` | Stop the old Worker and open the demo gap |
| `make finish-redeploy` | Start the compatible replacement Worker |
| `make partition-worker` | Disconnect the Worker from the Compose network |
| `make heal-worker` | Reconnect the Worker to the Compose network |

Delete all local demo history and projection data only when you truly want a
clean rehearsal:

```sh
docker compose down -v
docker compose up --build -d
```

### Services and ports

| Service | Host address | Purpose |
| --- | --- | --- |
| Stage | `127.0.0.1:3000` | Dashboard, demo API, health check |
| Temporal | `127.0.0.1:7233` | Temporal gRPC endpoint |
| Temporal Web | `127.0.0.1:8233` | Workflow inspection UI |

Inside Compose, the Worker reports to
`http://stage:3000/api/internal` and both Rust processes reach Temporal at
`http://temporal:7233`.

<details>
<summary><strong>API quick reference</strong></summary>

```text
GET  /healthz
GET  /api/state
POST /api/confessions       {"text":"I fixed the race with a sleep."}
POST /api/demo/hold         {"held":true}
POST /api/demo/mode         {"mode":"session"} or {"mode":"per_confession"}
POST /api/demo/agent-mode   {"agent_mode":"autonomous"} or {"agent_mode":"linear"}
POST /api/demo/seed
POST /api/demo/reset
POST /webhooks/twilio/messages   signed Twilio form, optional
```

`POST /api/confessions` accepts an optional `Idempotency-Key` header. Internal
endpoints require the shared Stage and Worker bearer token.

</details>

### Repository map

```text
src/bin/stage.rs    Rust/Axum stage server
src/bin/worker.rs   Temporal Worker process
src/bin/naive.rs    deliberately non-durable opening contrast
src/workflows.rs    deterministic durable agent orchestration
src/activities.rs   model, tool, report, and delivery side effects
src/agent.rs        fixture and OpenAI agent backends
src/domain.rs       shared serializable domain types
src/stage.rs        API, projection store, Workflow start, and release Signal
src/twilio.rs       signed Twilio webhook handling
src/twilio_poll.rs  outbound-only polling for inbound SMS
static/             stage dashboard and placeholder QR
tools/              QR and README diagram generators
compose.yaml        local Temporal, Stage, and Worker stack
```

### Regenerate the README diagrams

The static architecture SVG and animated redeploy GIF share one small renderer:

```sh
python3 -m pip install Pillow
python3 tools/render_readme_diagrams.py
```

The generated files are committed so GitHub renders the README without a build
step.

## Talk and project materials

- [Rust meetup recording](https://youtu.be/_t_Rxf8Z4mU?si=bqvJKqz_hZe2OheV)
- ["Rust Can Fix That" slide deck](docs/media/rust-can-fix-that-slides.pdf)
- [Deploy-and-resume MP4](docs/media/wall-of-regrets-walkthrough.mp4)
- [Demo runbook](docs/DEMO_RUNBOOK.md)
- [Architecture and production notes](docs/ARCHITECTURE.md)

## Acknowledgments

Built with help from Melissa Herrera, Spencer Judge, Chris Olszewski, Tom
Wheeler, and Shy.
