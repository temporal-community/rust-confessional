"use strict";

const POLL_INTERVAL_MS = 700;
const REQUEST_TIMEOUT_MS = 5000;

const elements = {
  form: document.querySelector("#confession-form"),
  input: document.querySelector("#confession-input"),
  submit: document.querySelector("#submit-button"),
  workerPanel: document.querySelector("#worker-panel"),
  workerStatus: document.querySelector("#worker-status"),
  temporalStatus: document.querySelector("#temporal-status"),
  modelMode: document.querySelector("#model-mode"),
  workflowMode: document.querySelector("#workflow-mode"),
  agentMode: document.querySelector("#agent-mode"),
  holdStatus: document.querySelector("#hold-status"),
  holdToggle: document.querySelector("#hold-toggle"),
  modeToggle: document.querySelector("#mode-toggle"),
  agentModeToggle: document.querySelector("#agent-mode-toggle"),
  seedButton: document.querySelector("#seed-button"),
  resetButton: document.querySelector("#reset-button"),
  demoRetry: document.querySelector("#demo-retry-button"),
  connectionState: document.querySelector("#connection-state"),
  connectionLabel: document.querySelector("#connection-label"),
  count: document.querySelector("#submission-count"),
  empty: document.querySelector("#empty-state"),
  list: document.querySelector("#submission-list"),
  received: document.querySelector("#stat-received"),
  waiting: document.querySelector("#stat-waiting"),
  thinking: document.querySelector("#stat-thinking"),
  judged: document.querySelector("#stat-judged"),
  toast: document.querySelector("#toast"),
  screenReaderStatus: document.querySelector("#sr-status"),
};

let knownIds = new Set();
let penanceAnimated = new Set();
let hasRenderedState = false;
let polling = false;
let pollTimer = null;
let toastTimer = null;
let resetConfirmTimer = null;
let lastSuccessfulPoll = 0;
let lastConnectionAnnouncement = "";
let pendingSubmission = null;

const WAITING_STATUSES = new Set([
  "received",
  "queued",
  "pending",
  "waiting",
  "held",
  "reply_pending",
  "accepted",
  "submitted",
]);

const JUDGED_STATUSES = new Set([
  "complete",
  "completed",
  "done",
  "judged",
  "replied",
  "sent",
  "delivered",
]);

const FAILED_STATUSES = new Set(["failed", "error", "cancelled", "canceled"]);

function normalizeStatus(status) {
  return String(status || "received").trim().toLowerCase().replace(/[\s-]+/g, "_");
}

function statusPhase(status) {
  const normalized = normalizeStatus(status);
  if (JUDGED_STATUSES.has(normalized)) return "judged";
  if (FAILED_STATUSES.has(normalized)) return "failed";
  if (WAITING_STATUSES.has(normalized)) return "waiting";
  return "thinking";
}

function humanize(value, fallback = "Unknown") {
  const text = String(value || "").trim();
  if (!text) return fallback;
  return text
    .replace(/[_-]+/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function safeText(value, fallback = "") {
  if (value === null || value === undefined) return fallback;
  return String(value);
}

function formatTime(value) {
  if (!value) return "just now";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return safeText(value);
  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

function setText(element, value) {
  const next = String(value);
  if (element.textContent !== next) element.textContent = next;
}

async function request(path, options = {}) {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  const headers = new Headers(options.headers || {});

  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  try {
    const response = await fetch(path, {
      ...options,
      headers,
      cache: "no-store",
      signal: controller.signal,
    });

    if (!response.ok) {
      let detail = "";
      try {
        const body = await response.json();
        detail = body.error || body.message || "";
      } catch (_) {
        // The status code is enough when the backend does not return JSON.
      }
      throw new Error(detail || `Request failed (${response.status})`);
    }

    if (response.status === 204) return null;
    const contentType = response.headers.get("content-type") || "";
    return contentType.includes("application/json") ? response.json() : null;
  } finally {
    window.clearTimeout(timeout);
  }
}

function setConnectionState(mode) {
  elements.connectionState.classList.toggle("is-connecting", mode === "connecting");
  elements.connectionState.classList.toggle("is-stale", mode === "stale");

  const label = mode === "live" ? "Live" : mode === "stale" ? "Reconnecting" : "Connecting";
  setText(elements.connectionLabel, label);

  if (lastConnectionAnnouncement !== label) {
    lastConnectionAnnouncement = label;
    setText(elements.screenReaderStatus, `Dashboard ${label.toLowerCase()}.`);
  }
}

function renderSystem(state) {
  const workerOnline = Boolean(state.worker_online);
  elements.workerPanel.classList.remove("is-unknown", "is-online", "is-offline");
  elements.workerPanel.classList.add(workerOnline ? "is-online" : "is-offline");
  setText(elements.workerStatus, workerOnline ? "Online" : "Offline");

  const temporalConnected = Boolean(state.temporal_connected);
  elements.temporalStatus.classList.remove("is-unknown", "is-connected", "is-disconnected");
  elements.temporalStatus.classList.add(temporalConnected ? "is-connected" : "is-disconnected");
  setText(elements.temporalStatus, `Temporal: ${temporalConnected ? "Connected" : "Disconnected"}`);

  setText(elements.modelMode, `Model: ${humanize(state.model_mode, "unknown")}`);
  // Light up the live model (OpenAI), otherwise it's the canned fixture backend.
  elements.modelMode.classList.toggle("is-active", state.model_mode === "openai");

  const aggregate = state.workflow_mode === "session";
  setText(elements.workflowMode, `Workflow: ${aggregate ? "Aggregate" : "Per confession"}`);
  elements.workflowMode.classList.toggle("is-active", aggregate);
  if (!elements.modeToggle.disabled) elements.modeToggle.checked = aggregate;

  const autonomous = state.agent_mode === "autonomous";
  setText(elements.agentMode, `Agent: ${autonomous ? "Autonomous" : "Linear"}`);
  elements.agentMode.classList.toggle("is-active", autonomous);
  if (!elements.agentModeToggle.disabled) elements.agentModeToggle.checked = autonomous;

  elements.holdStatus.classList.toggle("is-hidden", !state.held);

  if (!elements.holdToggle.disabled) elements.holdToggle.checked = Boolean(state.held);
}

function renderStats(submissions) {
  const counts = submissions.reduce(
    (result, submission) => {
      const phase = statusPhase(submission.status);
      if (phase === "waiting") result.waiting += 1;
      if (phase === "thinking") result.thinking += 1;
      if (phase === "judged") result.judged += 1;
      return result;
    },
    { waiting: 0, thinking: 0, judged: 0 },
  );

  setText(elements.received, submissions.length);
  setText(elements.waiting, counts.waiting);
  setText(elements.thinking, counts.thinking);
  setText(elements.judged, counts.judged);
  setText(elements.count, `${submissions.length} ${submissions.length === 1 ? "submission" : "submissions"}`);
}

function appendResultRow(container, label, value, className = "") {
  const text = safeText(value).trim();
  if (!text) return;

  const labelElement = document.createElement("span");
  labelElement.className = "result-label";
  labelElement.textContent = label;

  const valueElement = document.createElement("p");
  valueElement.className = `result-value ${className}`.trim();
  valueElement.textContent = text;

  container.append(labelElement, valueElement);
}

function submissionCard(submission, isNew) {
  const item = document.createElement("li");
  const phase = statusPhase(submission.status);
  item.className = `submission-card${isNew ? " is-new" : ""}`;
  item.dataset.phase = phase;
  item.dataset.submissionId = safeText(submission.id);

  const meta = document.createElement("div");
  meta.className = "submission-meta";

  const id = document.createElement("span");
  id.className = "submission-id";
  id.textContent = submission.id ? `#${submission.id}` : "#pending";
  id.title = safeText(submission.id, "pending");

  const status = document.createElement("span");
  status.className = "submission-status";
  status.textContent = humanize(submission.status, "Received");

  const time = document.createElement("time");
  time.className = "submission-time";
  time.textContent = formatTime(submission.created_at);
  if (submission.created_at) time.dateTime = safeText(submission.created_at);

  meta.append(id, status, time);

  const confession = document.createElement("p");
  confession.className = "confession-text";
  confession.textContent = safeText(submission.text, "Confession withheld");

  item.append(meta, confession);

  const hasResult = [
    submission.category,
    submission.judgment,
    submission.severity,
    submission.prescription,
    submission.error,
  ].some((value) => safeText(value).trim());

  if (hasResult) {
    const result = document.createElement("div");
    result.className = "result-block";
    appendResultRow(result, "Judgment", submission.judgment, "judgment");
    appendResultRow(result, "Category", submission.category, "category-tag");
    appendResultRow(result, "Severity", submission.severity);
    appendResultRow(result, "Rust can fix that", submission.prescription);
    appendResultRow(result, "Error", submission.error, "error");
    item.append(result);
  }

  // Autonomous confessions carry a step trace; linear ones send none, so the
  // block is skipped and their cards render exactly as before.
  const steps = Array.isArray(submission.agent_steps) ? submission.agent_steps : [];
  if (steps.length > 0) {
    const trace = document.createElement("div");
    trace.className = "agent-trace";

    const label = document.createElement("span");
    label.className = "result-label";
    label.textContent = "Agent trace";

    const crumbs = document.createElement("ol");
    crumbs.className = "trace-steps";
    for (const step of steps) {
      const crumb = document.createElement("li");
      crumb.textContent = safeText(step);
      crumbs.append(crumb);
    }

    trace.append(label, crumbs);
    item.append(trace);
  }

  const penanceText = safeText(submission.penance).trim();
  const penanceLine = safeText(submission.penance_line).trim();
  const penanceReps = Number(submission.penance_reps) || 0;
  if (penanceText && penanceLine && penanceReps > 0) {
    const id = safeText(submission.id);
    // Animate the loop only the first time this submission shows a penance;
    // the feed re-renders every poll, so gate on an id set to avoid replaying.
    const shouldAnimate = Boolean(id) && !penanceAnimated.has(id);
    if (shouldAnimate && id) penanceAnimated.add(id);

    const block = document.createElement("div");
    block.className = `penance-block${shouldAnimate ? " is-typing" : ""}`;

    const label = document.createElement("span");
    label.className = "result-label";
    label.textContent = `Penance · Ferris Level ${penanceReps}`;

    const task = document.createElement("p");
    task.className = "penance-task";
    task.textContent = penanceText;

    const code = document.createElement("pre");
    code.className = "penance-code";

    const open = document.createElement("div");
    open.className = "penance-fixed";
    open.textContent = `for _ in 0..${penanceReps} {`;
    code.append(open);

    for (let i = 0; i < penanceReps; i += 1) {
      const line = document.createElement("div");
      line.className = "penance-line";
      line.style.setProperty("--i", String(i));
      line.textContent = `    println!("${penanceLine}");`;
      code.append(line);
    }

    const close = document.createElement("div");
    close.className = "penance-fixed";
    close.textContent = "}";
    code.append(close);

    block.append(label, task, code);
    item.append(block);
  }

  return item;
}

function submissionTimestamp(submission) {
  const time = new Date(submission.created_at || 0).getTime();
  return Number.isNaN(time) ? 0 : time;
}

function renderSubmissions(submissions) {
  const sorted = [...submissions].sort((left, right) => submissionTimestamp(right) - submissionTimestamp(left));
  const nextIds = new Set(sorted.map((submission) => safeText(submission.id)));
  const fragment = document.createDocumentFragment();

  for (const submission of sorted) {
    const id = safeText(submission.id);
    const isNew = hasRenderedState && id && !knownIds.has(id);
    fragment.append(submissionCard(submission, isNew));
  }

  elements.list.replaceChildren(fragment);
  elements.empty.classList.toggle("is-hidden", sorted.length > 0);
  knownIds = nextIds;
}

function awardText(value, submissions) {
  if (value === null || value === undefined || value === "") return "";

  if (typeof value === "object") {
    return safeText(value.text || value.confession || value.judgment || value.title || value.id).trim();
  }

  const raw = safeText(value).trim();
  const match = submissions.find((submission) => safeText(submission.id) === raw);
  return match ? safeText(match.text).trim() : raw;
}

function renderAwards(awards, submissions) {
  const values = awards && typeof awards === "object" ? awards : {};

  document.querySelectorAll("[data-award]").forEach((card) => {
    const value = awardText(values[card.dataset.award], submissions);
    const output = card.querySelector("p");
    card.classList.toggle("has-winner", Boolean(value));
    setText(output, value || "Awaiting enough evidence…");
  });
}

function renderState(state) {
  const submissions = Array.isArray(state.submissions) ? state.submissions : [];
  renderSystem(state);
  renderStats(submissions);
  renderSubmissions(submissions);
  renderAwards(state.awards, submissions);
  hasRenderedState = true;
}

async function pollState() {
  if (polling) return;
  polling = true;

  try {
    const state = await request("/api/state");
    if (!state || typeof state !== "object") throw new Error("Invalid state response");
    renderState(state);
    lastSuccessfulPoll = Date.now();
    setConnectionState("live");
  } catch (error) {
    const mode = lastSuccessfulPoll ? "stale" : "connecting";
    setConnectionState(mode);
  } finally {
    polling = false;
    window.clearTimeout(pollTimer);
    pollTimer = window.setTimeout(pollState, POLL_INTERVAL_MS);
  }
}

function showToast(message, isError = false) {
  window.clearTimeout(toastTimer);
  elements.toast.textContent = message;
  elements.toast.classList.toggle("is-error", isError);
  elements.toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => elements.toast.classList.remove("is-visible"), 2800);
}

async function runAction(button, path, body, successMessage, headers) {
  button.disabled = true;
  try {
    await request(path, {
      method: "POST",
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    showToast(successMessage);
    await pollState();
    return true;
  } catch (error) {
    showToast(error.name === "AbortError" ? "The request timed out." : error.message, true);
    return false;
  } finally {
    button.disabled = false;
  }
}

function newIdempotencyKey() {
  if (window.crypto && typeof window.crypto.randomUUID === "function") {
    return window.crypto.randomUUID().replace(/-/g, "").slice(0, 8);
  }
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`.slice(0, 8);
}

elements.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const text = elements.input.value.trim();
  if (!text) {
    elements.input.focus();
    return;
  }

  if (!pendingSubmission || pendingSubmission.text !== text) {
    pendingSubmission = { text, key: newIdempotencyKey() };
  }
  const succeeded = await runAction(
    elements.submit,
    "/api/confessions",
    { text },
    "Confession received. Ferris is judging.",
    { "Idempotency-Key": pendingSubmission.key },
  );
  if (succeeded) {
    pendingSubmission = null;
    elements.input.value = "";
    elements.input.focus();
  }
});

elements.input.addEventListener("input", () => {
  if (pendingSubmission && elements.input.value.trim() !== pendingSubmission.text) {
    pendingSubmission = null;
  }
});

// Stage helper: load the confession that triggers the transient rate-limit beat
// (the compose Activity fails retryably on its first two attempts). Populates the
// box rather than submitting, so the audience sees the text before you confess.
elements.demoRetry.addEventListener("click", () => {
  elements.input.value = "My agent keeps getting rate-limited (HTTP 429).";
  elements.input.dispatchEvent(new Event("input", { bubbles: true }));
  elements.input.focus();
});

elements.holdToggle.addEventListener("change", async () => {
  const requested = elements.holdToggle.checked;
  elements.holdToggle.disabled = true;

  try {
    await request("/api/demo/hold", {
      method: "POST",
      body: JSON.stringify({ held: requested }),
    });
    showToast(requested ? "Replies are held. Ready for chaos." : "Replies released.");
    await pollState();
  } catch (error) {
    elements.holdToggle.checked = !requested;
    showToast(error.name === "AbortError" ? "The request timed out." : error.message, true);
  } finally {
    elements.holdToggle.disabled = false;
  }
});

elements.modeToggle.addEventListener("change", async () => {
  const wantsSession = elements.modeToggle.checked;
  const mode = wantsSession ? "session" : "per_confession";
  elements.modeToggle.disabled = true;

  try {
    await request("/api/demo/mode", {
      method: "POST",
      body: JSON.stringify({ mode }),
    });
    showToast(
      wantsSession
        ? "Aggregate workflow mode. Demo reset."
        : "Per-confession mode. Demo reset.",
    );
    // Switching resets the session server-side; drop local render state to match.
    knownIds = new Set();
    penanceAnimated = new Set();
    hasRenderedState = false;
    await pollState();
  } catch (error) {
    elements.modeToggle.checked = !wantsSession;
    showToast(error.name === "AbortError" ? "The request timed out." : error.message, true);
  } finally {
    elements.modeToggle.disabled = false;
  }
});

elements.agentModeToggle.addEventListener("change", async () => {
  const wantsAutonomous = elements.agentModeToggle.checked;
  const agentMode = wantsAutonomous ? "autonomous" : "linear";
  elements.agentModeToggle.disabled = true;

  try {
    await request("/api/demo/agent-mode", {
      method: "POST",
      body: JSON.stringify({ agent_mode: agentMode }),
    });
    showToast(
      wantsAutonomous
        ? "Autonomous agent mode. New confessions run the loop."
        : "Linear pipeline mode.",
    );
    // Agent mode does not reset the session; each confession keeps its own mode.
    await pollState();
  } catch (error) {
    elements.agentModeToggle.checked = !wantsAutonomous;
    showToast(error.name === "AbortError" ? "The request timed out." : error.message, true);
  } finally {
    elements.agentModeToggle.disabled = false;
  }
});

elements.seedButton.addEventListener("click", () => {
  runAction(elements.seedButton, "/api/demo/seed", undefined, "Example confessions seeded.");
});

elements.resetButton.addEventListener("click", async () => {
  if (!elements.resetButton.classList.contains("is-confirming")) {
    elements.resetButton.classList.add("is-confirming");
    elements.resetButton.textContent = "Click again to reset";
    showToast("Click Reset again to clear the demo.");
    window.clearTimeout(resetConfirmTimer);
    resetConfirmTimer = window.setTimeout(() => {
      elements.resetButton.classList.remove("is-confirming");
      elements.resetButton.textContent = "Reset demo";
    }, 4000);
    return;
  }

  window.clearTimeout(resetConfirmTimer);
  const succeeded = await runAction(elements.resetButton, "/api/demo/reset", undefined, "Demo reset.");
  elements.resetButton.classList.remove("is-confirming");
  elements.resetButton.textContent = "Reset demo";
  if (succeeded) {
    knownIds = new Set();
    penanceAnimated = new Set();
    hasRenderedState = false;
  }
});

document.addEventListener("visibilitychange", () => {
  if (!document.hidden) {
    window.clearTimeout(pollTimer);
    pollState();
  }
});

setConnectionState("connecting");
pollState();
