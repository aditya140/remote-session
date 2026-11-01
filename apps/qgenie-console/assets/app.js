"use strict";
const $ = (selector) => document.querySelector(selector);
const escapeHtml = (value) =>
  String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
const paths = {
  grid: "M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z",
  users:
    "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8M22 21v-2a4 4 0 0 0-3-3.87",
  server: "M3 3h18v7H3zM3 14h18v7H3zM7 6.5h.01M7 17.5h.01M11 6.5h6M11 17.5h6",
  laptop: "M4 4h16v12H4zM2 20h20M8 16v4M16 16v4",
  agent: "M8 3h8M12 1v2M5 7h14v13H5zM2 11h3M19 11h3M8 11h.01M16 11h.01M9 16h6",
  activity: "M3 12h4l3-8 4 16 3-8h4",
  chart: "M3 3v18h18M7 15v-4M12 15V7M17 15V5",
  shield: "M12 3l8 3v6c0 5-8 9-8 9s-8-4-8-9V6zM8 12l3 3 5-6",
  file: "M6 3h8l5 5v13H6zM14 3v6h5M9 13h7M9 17h5",
  terminal: "M4 5h16v14H4zM7 9l3 3-3 3M13 15h4",
  arrow: "M5 12h14M14 7l5 5-5 5",
  clock: "M12 8v5l3 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0",
  model: "M12 2l9 5v10l-9 5-9-5V7zM3 7l9 5 9-5M12 12v10",
};
const icon = (name, size = 18) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${paths[name] || paths.grid}"/></svg>`;
const chip = (label, tone = "") =>
  `<span class="pill ${tone}">${escapeHtml(label)}</span>`;
const card = (title, body, right = "") =>
  `<section class="card"><div class="cardhead"><h2>${escapeHtml(title)}</h2>${right}</div>${body}</section>`;
const button = (label, attribute, value, cls = "") =>
  `<button class="${cls}" ${attribute}="${escapeHtml(value)}">${escapeHtml(label)}</button>`;
let auth = null,
  view = "workspace",
  page = "Overview",
  selectedUser = null,
  selectedDevice = null,
  observedAt = null,
  coverage = null;
let inventoryUsers = [],
  inventoryDevices = [],
  inventoryRuns = [],
  telemetrySeries = null,
  telemetrySignals = null,
  telemetryRuns = null,
  telemetryStatus = null,
  lastError = null;
let loadController = null,
  loading = false;
const stopResults = new Map();
const names = {
  Overview: "grid",
  Users: "users",
  Fleet: "server",
  "Telemetry & signals": "chart",
  "System health": "activity",
  Audit: "file",
  Governance: "shield",
  "My devices": "laptop",
  "Session history": "file",
};
const currentUser = () => auth?.user;
const ownedDevices = (id) =>
  inventoryDevices.filter((device) => device.owner === id);
const deviceRuns = (id) => inventoryRuns.filter((run) => run.device === id);
const userRuns = (id) => inventoryRuns.filter((run) => run.owner === id);
const stateOf = (run) =>
  stopResults.has(run.id) ? stopResults.get(run.id).state : run.state;
const activeRuns = (runs) =>
  runs.filter((run) => ["Running", "Idle"].includes(stateOf(run)));
const unknownRuns = (runs) =>
  runs.filter((run) =>
    ["Unknown", "Stop unknown", "Pending confirmation"].includes(stateOf(run)),
  );
const initials = (name) =>
  String(name || "Q")
    .split(/\s+/)
    .map((s) => s[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
function timestamp(value) {
  if (value == null) return null;
  const n =
    typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  const date = new Date(n);
  return Number.isFinite(date.valueOf()) ? date : null;
}
function time(value) {
  const date = timestamp(value);
  return date
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
        timeZone: "UTC",
      }).format(date) + " UTC"
    : "Not recorded";
}
function age(value) {
  const date = timestamp(value);
  if (!date) return "Not reported";
  const seconds = Math.max(0, Math.floor((Date.now() - date) / 1000));
  return seconds < 60
    ? `${seconds}s ago`
    : seconds < 3600
      ? `${Math.floor(seconds / 60)}m ago`
      : time(value);
}
function notify(message) {
  $("#toast").textContent = message;
  $("#toast").classList.remove("hide");
  setTimeout(() => $("#toast").classList.add("hide"), 6500);
}
async function api(url, options = {}) {
  const controller = options.signal ? null : new AbortController();
  const timeout = controller
    ? setTimeout(() => controller.abort(), 12000)
    : null;
  try {
    const response = await fetch(url, {
      credentials: "same-origin",
      cache: "no-store",
      ...options,
      signal: options.signal || controller.signal,
      headers: {
        Accept: "application/json",
        ...(options.body ? { "Content-Type": "application/json" } : {}),
        ...(options.method && options.method !== "GET"
          ? { "X-CSRF-Token": auth?.csrf_token || "" }
          : {}),
        ...options.headers,
      },
    });
    const text = await response.text();
    if (text.length > 8 * 1024 * 1024)
      throw new Error(
        "Response exceeded the console read limit. Narrow the scope.",
      );
    let data = {};
    try {
      data = text ? JSON.parse(text) : {};
    } catch {
      throw new Error("The gateway returned an invalid response.");
    }
    if (!response.ok) {
      const error = new Error(
        data.message ||
          data.error?.message ||
          `Gateway request failed (${response.status}).`,
      );
      error.status = response.status;
      error.code = data.code;
      throw error;
    }
    return data;
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}
function normalize(snapshot) {
  if (
    !Array.isArray(snapshot.users) ||
    !Array.isArray(snapshot.machines) ||
    !Array.isArray(snapshot.sessions)
  )
    throw new Error("Inventory response is incomplete.");
  inventoryUsers = snapshot.users.map((u) => ({
    ...u,
    name: u.display_name || u.id,
    initials: initials(u.display_name || u.id),
  }));
  inventoryDevices = snapshot.machines.map((d) => ({
    ...d,
    name: d.name || d.hostname || d.id,
    kind:
      d.device_class === "server"
        ? "Server"
        : d.device_class === "workstation"
          ? "Workstation"
          : "Unknown type",
    online: d.connected === true,
    lastSeen: age(d.last_seen_ms),
    lastConnected: time(d.last_connected_ms),
  }));
  inventoryRuns = snapshot.sessions.map((s) => ({
    ...s,
    device: s.machine,
    name: s.name || s.id,
    harness: s.harness || (s.kind === "pty" ? "PTY shell" : "Not reported"),
    version: s.harness_version || "Version not reported",
    source:
      s.model_source === "not_reported"
        ? "Not reported"
        : s.model_source || "Not reported",
    state:
      {
        running: "Running",
        idle: "Idle",
        exited: "Exited",
        failed: "Failed",
        lost: "Lost",
      }[s.state] || "Unknown",
    task:
      {
        running: "Turn running",
        completed: "Turn completed",
        failed: "Turn failed",
        interrupted: "Interrupted",
        idle: "Idle",
      }[s.task_state] || "Task not reported",
    started: time(s.created_at_ms),
    model: s.model || null,
    tokens: null,
    terminate: s.terminate === true,
  }));
  observedAt = snapshot.observed_at_ms;
  coverage = snapshot.coverage;
  for (const run of inventoryRuns) {
    if (["Exited", "Failed", "Lost"].includes(run.state))
      stopResults.delete(run.id);
  }
}
async function loadData({ quiet = false } = {}) {
  if (loadController) loadController.abort();
  loadController = new AbortController();
  const controller = loadController;
  const timer = setTimeout(() => controller.abort(), 15000);
  if (!quiet) {
    loading = true;
    render();
  }
  try {
    const snapshot = await api(
      view === "admin" ? "/v1/admin/overview" : "/v1/me/overview",
      { signal: controller.signal },
    );
    if (loadController !== controller) return;
    normalize(snapshot);
    lastError = null;
    if (view === "admin" && page === "Telemetry & signals") {
      await loadTelemetry(controller.signal);
    }
    if (page === "Audit" && view === "admin")
      await loadAudit(controller.signal);
    if (page === "System health" && view === "admin")
      await loadHealth(controller.signal);
    loading = false;
    render();
  } catch (error) {
    if (loadController !== controller) return;
    loading = false;
    if (error.status === 401) {
      auth = null;
      renderLogin("Your sign-in expired. Sign in again to continue.");
      return;
    }
    lastError =
      error.name === "AbortError"
        ? "The read timed out. The last received inventory is shown below."
        : error.message;
    render();
  } finally {
    clearTimeout(timer);
    if (loadController === controller) loadController = null;
  }
}
async function loadTelemetry(signal) {
  const results = await Promise.allSettled([
    api("/v1/admin/telemetry/series", { signal }),
    api("/v1/admin/activity-signals", { signal }),
    api("/v1/admin/telemetry/runs", { signal }),
  ]);
  telemetrySeries = results[0].status === "fulfilled" ? results[0].value : null;
  telemetrySignals =
    results[1].status === "fulfilled" ? results[1].value : null;
  telemetryRuns = results[2].status === "fulfilled" ? results[2].value : null;
  telemetryStatus = results
    .filter((r) => r.status === "rejected")
    .map((r) => r.reason.message)
    .join(" ");
}
let healthData = null,
  healthError = null;
async function loadHealth(signal) {
  try {
    healthData = await api("/v1/admin/health", { signal });
    healthError = null;
  } catch (e) {
    healthData = null;
    healthError = e.message;
  }
}
let auditData = null,
  auditError = null;
async function loadAudit(signal) {
  try {
    auditData = await api("/v1/admin/audit?limit=50", { signal });
    auditError = null;
  } catch (e) {
    auditData = null;
    auditError = e.message;
  }
}
function renderLogin(
  message = "Sign in to view your devices and current native work.",
) {
  document.body.dataset.theme = "workspace";
  $("#scope").textContent = "Authentication required";
  $("#actor").textContent = "Not signed in";
  $("#role").textContent = "";
  $("#nav").innerHTML = "";
  $("#logout").classList.add("hide");
  $("#content").innerHTML =
    `<section class="card error-card"><div class="icon">${icon("shield", 24)}</div><h1>Your workspace starts here.</h1><p>${escapeHtml(message)}</p><a class="login-button" href="/console/auth/login">Sign in with your organization</a></section>`;
}
function breadcrumb() {
  return view === "admin" ? "Platform administration" : "My workspace";
}
function render() {
  if (!auth) return;
  const entries =
    view === "admin"
      ? [
          "Overview",
          "Users",
          "Fleet",
          "Telemetry & signals",
          "System health",
          "Audit",
          "Governance",
        ]
      : ["My devices", "Session history"];
  $("#actor").textContent = currentUser();
  $("#initials").textContent = initials(currentUser());
  $("#role").textContent =
    view === "admin" ? "Platform admin" : "Session owner";
  $("#scope").textContent =
    view === "admin" ? "Administrative metadata" : "Your authorized resources";
  $("#logout").classList.remove("hide");
  $("#crumb").textContent = breadcrumb() + " / " + page;
  $("#nav").innerHTML =
    (auth.platform_admin
      ? `<div class="role-view"><button data-view="workspace" class="${view === "workspace" ? "active" : ""}">My workspace</button><button data-view="admin" class="${view === "admin" ? "active" : ""}">Admin</button></div>`
      : "") +
    entries
      .map(
        (name) =>
          `<button data-page="${name}" class="${page === name ? "active" : ""}">${icon(names[name], 17)}${escapeHtml(name)}</button>`,
      )
      .join("");
  if (loading && !observedAt) {
    $("#content").innerHTML =
      '<section class="card pad"><h1>Loading your inventory…</h1><p>Waiting for authorized gateway data.</p></section>';
    return;
  }
  if (lastError && !observedAt) {
    $("#content").innerHTML =
      `<section class="card error-card"><h1>Inventory is unavailable.</h1><p>${escapeHtml(lastError)}</p><button id="retry-load">Try again</button></section>`;
    return;
  }
  const content =
    page === "My devices"
      ? ownerView()
      : page === "Users"
        ? usersView()
        : page === "Fleet"
          ? fleetView()
          : page === "Telemetry & signals"
            ? telemetryView()
            : page === "System health"
              ? healthView()
              : page === "Audit"
                ? auditView()
                : page === "Governance"
                  ? governanceView()
                  : page === "Session history"
                    ? historyView()
                    : overviewView();
  $("#content").innerHTML =
    (lastError
      ? `<div class="banner warn" role="alert">${escapeHtml(lastError)}</div>`
      : "") +
    (coverage?.complete === false
      ? '<div class="banner warn">Partial inventory: this bounded snapshot does not include every record. Totals below describe the displayed scope.</div>'
      : "") +
    content +
    `<div class="bottom"><span>Gateway observation: ${escapeHtml(time(observedAt))}</span><span>${view === "admin" ? "Metadata visibility does not grant native control" : "Native control remains explicitly authorized"}</span></div>`;
  $("#content").setAttribute("aria-busy", loading ? "true" : "false");
  filterVisible();
  if (typeof paintHistoryText === "function") paintHistoryText();
}
function filterVisible() {
  const query = $("#search").value.trim().toLowerCase();
  document
    .querySelectorAll(".searchable")
    .forEach((row) =>
      row.classList.toggle(
        "hide",
        !row.textContent.toLowerCase().includes(query),
      ),
    );
}
