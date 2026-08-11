# Twilio input

Rust Confessional can accept audience confessions by SMS. Twilio input is
inbound-only: message bodies become submissions and judgments appear on the
dashboard or Wall of Regrets. The demo does not send an SMS response.

You need your own Twilio account and number. The QR committed to the repository
contains a placeholder on purpose.

## Choose an ingress mode

| Mode | Best for | Network requirement |
| --- | --- | --- |
| **Signed webhook** | A host with a carefully restricted public endpoint | Twilio must reach one HTTPS path |
| **Outbound polling** | A locked-down host that cannot receive inbound traffic | The Stage makes HTTPS requests to `api.twilio.com` |

Use only one mode at a time.

## Public-event safeguards

- Keep `SHOW_RAW_CONFESSIONS=false`.
- Never expose the full Stage service or its operator controls to a LAN or the
  internet.
- Never commit Twilio credentials or a live event number.
- Treat audience submissions as public conference content, not secrets.
- Open only seeded or presenter-owned Workflow payloads in Temporal Web.

Stage fails closed if Twilio and raw display are enabled together. The
`ALLOW_UNMODERATED_TWILIO=true` override is for controlled integration testing,
not public events.

## Option A: signed webhook

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

Configure the number's inbound message webhook to send an HTTP `POST` to that
same URL.

Publish only `/webhooks/twilio/messages` through a path-restricted HTTPS reverse
proxy. A general tunnel to port `3000` would also expose the unauthenticated
dashboard, reset, seed, and hold controls.

The endpoint:

- validates `X-Twilio-Signature` using the exact configured public URL
- verifies `AccountSid`
- requires and deduplicates on `MessageSid`
- validates but does not retain or log sender and recipient phone numbers
- ignores STOP, START, and HELP-family compliance messages
- returns empty TwiML and never sends an outbound SMS

## Option B: outbound polling

Polling is useful when the presentation host cannot receive a public webhook.
It makes outbound HTTPS requests only.

```sh
export TWILIO_ACCOUNT_SID="AC..."
export TWILIO_API_KEY_SID="SK..."
read -rsp "Twilio API key secret: " TWILIO_API_KEY_SECRET
export TWILIO_API_KEY_SECRET
export TWILIO_NUMBER="+15551234567"
export TWILIO_POLL_SECONDS=4
export SHOW_RAW_CONFESSIONS=false
docker compose up -d --force-recreate stage
```

Leave `TWILIO_WEBHOOK_URL` unset for polling-only operation. On its first
successful request the poller baselines the existing backlog, then accepts only
new messages. Inbound latency is up to `TWILIO_POLL_SECONDS`.

## Generate the live SMS QR locally

Never commit a real event number. Public numbers attract spam and inbound
charges, and Git history is permanent.

```sh
python3 -m pip install segno
python3 tools/gen_qr.py "+15551234567"
git update-index --skip-worktree static/confess-qr.svg
```

Restore normal tracking after the event:

```sh
git update-index --no-skip-worktree static/confess-qr.svg
```

## Event checklist

1. Start in fixture mode with `SHOW_RAW_CONFESSIONS=false`.
2. Send one real test message before doors open.
3. Confirm a placeholder card appears and becomes a stage-safe judgment.
4. Put `/?view=wall` on the public display and keep `/` on the operator laptop.
5. Keep a seeded browser-submission fallback ready.
6. After the event, stop the public proxy and clear credentials from the shell.

```sh
unset TWILIO_AUTH_TOKEN TWILIO_ACCOUNT_SID TWILIO_WEBHOOK_URL
unset TWILIO_API_KEY_SID TWILIO_API_KEY_SECRET TWILIO_NUMBER
unset SHOW_RAW_CONFESSIONS ALLOW_UNMODERATED_TWILIO
```

For the complete trust model and stored-data implications, see
[Architecture: optional Twilio inbound boundary](ARCHITECTURE.md#optional-twilio-inbound-boundary)
and [Data and privacy implications](ARCHITECTURE.md#data-and-privacy-implications).
