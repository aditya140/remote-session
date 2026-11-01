"use strict";
function telemetrySourceLabel(code) {
  const labels = {
    scheduled_rule_evaluation_not_configured: "Not configured",
    audit_reader_unavailable: "The audit source is unavailable.",
    on_demand_metadata_rules: "On-demand audit and coverage review",
    on_demand_inventory_coverage: "On-demand coverage review",
  };
  return labels[code] || "Unavailable";
}
const svgDonut = (segments, center, label) => {
  const total = segments.reduce((n, s) => n + s.value, 0);
  let offset = 0;
  const rings = segments
    .filter((s) => s.value > 0)
    .map((s) => {
      const p = (s.value / (total || 1)) * 100;
      const result = `<circle cx="90" cy="90" r="65" fill="none" stroke="${s.color}" stroke-width="17" pathLength="100" stroke-dasharray="${p} ${100 - p}" stroke-dashoffset="${-offset}"/>`;
      offset += p;
      return result;
    })
    .join("");
  return `<svg viewBox="0 0 180 180" class="av-donut" role="img" aria-label="${escapeHtml(label)}: ${segments.map((s) => escapeHtml(s.name) + ": " + s.value).join(", ")}"><circle cx="90" cy="90" r="65" fill="none" stroke="#e9edf5" stroke-width="17"/><g transform="rotate(-90 90 90)">${rings}</g><text x="90" y="89" text-anchor="middle" class="av-ring-number">${escapeHtml(center)}</text><text x="90" y="110" text-anchor="middle" class="av-ring-label">${escapeHtml(label)}</text></svg>`;
};
const metric = (i, label, value, note, tone) =>
  `<div class="av-metric ${tone}"><span class="av-icon">${icon(i, 22)}</span><span><small>${escapeHtml(label)}</small><strong>${escapeHtml(value)}</strong><em>${escapeHtml(note)}</em></span></div>`;
function heading(kicker, title, subtitle, right = "") {
  return `<div class="inventory-heading"><div><div class="eyebrow">${escapeHtml(kicker)}</div><h1>${escapeHtml(title)}</h1><p>${escapeHtml(subtitle)}</p></div>${right}</div>`;
}
function inventoryMetrics(devices, runs, users = null) {
  return `<div class="av-metrics">${metric(users ? "users" : "laptop", users ? "Users" : "Devices", users ? users.length : devices.length, users ? "Authorized inventory scope" : `${devices.filter((d) => d.online).length} connected`, "av-blue")}${metric("server", "Enrolled servers", devices.filter((d) => d.kind === "Server").length, `${devices.filter((d) => d.kind === "Server" && d.online).length} connected · device type is reported`, "av-purple")}${metric("activity", "Active native sessions", activeRuns(runs).length, `${unknownRuns(runs).length} current states unknown`, "av-teal")}${metric("agent", "Running agent processes", activeRuns(runs).filter((r) => r.kind === "agent").length, "Installed inventory is not execution", "av-amber")}</div>`;
}
function empty(title, message) {
  return `<div class="empty"><h3>${escapeHtml(title)}</h3><p>${escapeHtml(message)}</p></div>`;
}
function coverageCard(devices, runs) {
  const active = activeRuns(runs).length,
    missing = unknownRuns(runs).length;
  return card(
    "Observation coverage",
    `<div class="av-coverage">${svgDonut(
      [
        { name: "Confirmed active", value: active, color: "#0caa91" },
        { name: "Unknown native state", value: missing, color: "#edac42" },
      ],
      active + missing,
      "native records",
    )}<div class="av-coverage-key"><div><i style="background:#0caa91"></i><span>Confirmed active<b>${active}</b></span></div><div><i style="background:#edac42"></i><span>Unknown state<b>${missing}</b></span></div><p>${devices.filter((d) => d.online).length} of ${devices.length} displayed hosts are connected. An offline host can still run work locally.</p></div></div>`,
    chip(
      missing ? "Partial coverage" : "Current projection",
      missing ? "warn" : "",
    ),
  );
}
function ownerCards() {
  return `<div class="av-people-grid">${inventoryUsers
    .map((u, index) => {
      const ds = ownedDevices(u.id),
        runs = userRuns(u.id);
      return `<button class="av-person searchable" data-user="${escapeHtml(u.id)}" aria-label="Open ${escapeHtml(u.name)}" style="--av-person:${["#4878f5", "#9863e4", "#13a78c"][index % 3]}"><span class="av-person-head"><span class="av-avatar">${escapeHtml(u.initials)}</span><span class="av-person-title"><b>${escapeHtml(u.name)}</b><small>${escapeHtml(u.id)}</small></span>${icon("arrow", 15)}</span><span class="av-person-stats"><span><b>${ds.length}</b><small>devices</small></span><span><b>${activeRuns(runs).length}</b><small>active sessions</small></span><span><b>${unknownRuns(runs).length}</b><small>unknown</small></span></span><span class="av-device-track">${ds.map((d) => `<span class="${d.online ? "is-online" : "is-stale"}">${icon(d.kind === "Server" ? "server" : "laptop", 17)}<i></i></span>`).join("")}<small>${ds.filter((d) => d.online).length} connected</small></span></button>`;
    })
    .join("")}</div>`;
}
function overviewView() {
  return `<div class="av-root">${heading("Platform workspace", "A pulse on your people.", "Trace devices, harnesses, sessions, and incomplete observations.", button("Explore users", "data-page", "Users", "primary"))}${inventoryMetrics(inventoryDevices, inventoryRuns, inventoryUsers)}<div class="av-plot-grid">${card("Execution by owner", ownerBars())}${coverageCard(inventoryDevices, inventoryRuns)}</div><div class="av-section-head"><div><h2>People & their connected world</h2><p>Profile display names are shown only when verified data is available.</p></div>${button("View directory", "data-page", "Users")}</div>${inventoryUsers.length ? ownerCards() : empty("No users in this scope", "The current authorized inventory contains no user records.")}<div class="links-row">${button("Inspect telemetry", "data-page", "Telemetry & signals")}${button("Read audit metadata", "data-page", "Audit")}</div></div>`;
}
function ownerBars() {
  const maximum = Math.max(
    1,
    ...inventoryUsers.map(
      (u) =>
        userRuns(u.id).filter((r) => !["Exited", "Failed"].includes(stateOf(r)))
          .length,
    ),
  );
  return inventoryUsers.length
    ? `<div class="tv-owner-chart">${inventoryUsers
        .map((u) => {
          const rs = userRuns(u.id),
            active = activeRuns(rs).length,
            missing = unknownRuns(rs).length;
          return `<div class="tv-owner-row"><button data-user="${escapeHtml(u.id)}"><span class="tv-avatar">${escapeHtml(u.initials)}</span>${escapeHtml(u.name)}</button><div class="tv-owner-track" role="img" aria-label="${active} confirmed active, ${missing} unknown"><i style="width:${(100 * active) / maximum}%" class="tv-segment-mint"></i><i style="width:${(100 * missing) / maximum}%" class="tv-segment-amber"></i></div><span><b>${active}</b> <small>+${missing} ?</small></span></div>`;
        })
        .join(
          "",
        )}<div class="tv-owner-axis"><span>0</span><span>${maximum} sessions</span></div></div><footer class="tv-chart-foot"><span><i class="tv-key tv-mint"></i>Confirmed active</span><span><i class="tv-key tv-amber"></i>Unknown</span></footer>`
    : empty(
        "No session observations",
        "Counts will appear when enrolled hosts report native work.",
      );
}
function usersView() {
  if (selectedDevice) return deviceView(false);
  if (!selectedUser)
    return `<div class="av-root">${heading("Administration / people", "People, devices, and native work.", "Only metadata is shown. Recording access uses a separate protected API.")}${inventoryMetrics(inventoryDevices, inventoryRuns, inventoryUsers)}${ownerCards()}</div>`;
  const u = inventoryUsers.find((u) => u.id === selectedUser);
  if (!u)
    return empty(
      "User is outside the current snapshot",
      "Refresh or return to the user directory.",
    );
  const ds = ownedDevices(u.id),
    rs = userRuns(u.id);
  return `<div class="av-root">${heading("People / user detail", u.name, u.id, button("All users", "data-back", "users"))}${inventoryMetrics(ds, rs)}${deviceCards(ds, false)}<div style="margin-top:22px">${card("Agents & native sessions", executionTable(rs, false), chip("Metadata only", "neutral"))}</div></div>`;
}
function fleetView() {
  return `<div class="av-root">${heading("Administration / fleet", "Every device, with clear context.", "Device type is explicitly reported. Unknown classifications remain unknown.")}${selectedDevice ? deviceView(false) : deviceCards(inventoryDevices, false)}</div>`;
}
function deviceCards(devices, owner) {
  return devices.length
    ? `<div class="uv-device-grid">${devices
        .map((d, index) => {
          const runs = deviceRuns(d.id),
            active = activeRuns(runs),
            harnesses = [...new Set(runs.map((r) => r.harness))],
            tone = d.online ? ["blue", "teal", "pink"][index % 3] : "amber";
          return `<article class="uv-device uv-device-${tone} searchable"><div class="uv-device-top"><span class="uv-device-icon uv-${tone}">${icon(d.kind === "Server" ? "server" : "laptop", 28)}</span>${chip(d.online ? "Connected" : d.disabled ? "Access disabled" : "Offline", d.online ? "" : "warn")}</div><h3>${escapeHtml(d.name)}</h3><p>${escapeHtml(d.os)} / ${escapeHtml(d.kind)}</p><div class="uv-device-count"><strong>${active.length}</strong><span>confirmed active sessions<small>${unknownRuns(runs).length} unknown</small></span></div><div class="uv-device-rail">${harnesses.map((h) => `<span>${icon(h === "PTY shell" ? "terminal" : "agent", 13)}${escapeHtml(h)}</span>`).join("") || "<span>No reported sessions</span>"}</div><div class="uv-device-detail"><span>Last seen<b>${escapeHtml(d.lastSeen)}</b></span><span>Last connected<b>${escapeHtml(d.lastConnected)}</b></span></div><button class="uv-device-open" data-device="${escapeHtml(d.id)}">View sessions ${icon("arrow", 15)}</button></article>`;
        })
        .join("")}</div>`
    : empty(
        "No devices reported",
        "Enroll a host through the QGenie CLI to make its authorized metadata available here.",
      );
}
function executionTable(runs, owner) {
  return runs.length
    ? `<div class="scroll"><table><thead><tr><th>Session / harness</th><th>Device</th><th>Model</th><th>Native process / task</th><th>Action</th></tr></thead><tbody>${runs
        .map((r) => {
          const d = inventoryDevices.find((d) => d.id === r.device),
            state = stateOf(r);
          return `<tr class="searchable" data-session-row="${escapeHtml(r.id)}"><td><b>${escapeHtml(r.name)}</b><small>${escapeHtml(r.harness)} · ${escapeHtml(r.version)}</small><small>${escapeHtml(r.id)}</small></td><td><b>${escapeHtml(d?.name || r.device)}</b><small>${escapeHtml(d?.lastSeen || "Device outside current page")}</small></td><td><b>${escapeHtml(r.kind === "pty" ? "Not applicable" : r.model || "Not reported")}</b><small>${escapeHtml(r.kind === "pty" ? "Terminal session" : r.requested_model ? "Configured: " + r.requested_model : r.source)}</small></td><td>${chip(state, ["Running", "Idle"].includes(state) ? "" : ["Unknown", "Stop unknown", "Pending confirmation"].includes(state) ? "warn" : "neutral")}<small>${escapeHtml(r.task)}</small></td><td>${owner ? stopButton(r, d) : button("History metadata", "data-history", r.id)}</td></tr>`;
        })
        .join("")}</tbody></table></div>`
    : empty(
        "No sessions reported",
        "The host has not reported any session in this inventory scope.",
      );
}
function stopButton(run, device) {
  const result = stopResults.get(run.id);
  if (result)
    return `<button data-reconcile="${escapeHtml(run.id)}">${icon("clock", 13)} Check outcome</button>`;
  const permitted =
    view === "workspace" &&
    run.owner === currentUser() &&
    device?.online &&
    run.terminate &&
    ["Running", "Idle"].includes(run.state);
  return `<button class="uv-stop" data-stop="${escapeHtml(run.id)}" ${permitted ? "" : "disabled"}>${icon("shield", 13)}${["Exited", "Failed", "Lost"].includes(run.state) ? "Ended" : !device?.online ? "Host offline" : !run.terminate ? "Stop unavailable" : "Stop session"}</button>`;
}
function sessionCards(runs) {
  return runs
    .map((r) => {
      const d = inventoryDevices.find((d) => d.id === r.device),
        state = stateOf(r);
      return `<article class="uv-session searchable" data-session-row="${escapeHtml(r.id)}"><div class="uv-session-main"><span class="uv-session-icon uv-${r.kind === "pty" ? "teal" : "blue"}">${icon(r.kind === "pty" ? "terminal" : "agent", 25)}</span><div class="uv-session-title"><h3>${escapeHtml(r.name)}</h3><p>${icon(d?.kind === "Server" ? "server" : "laptop", 13)} ${escapeHtml(d?.name || r.device)} <span>${escapeHtml(r.harness)} · ${escapeHtml(r.version)}</span></p></div>${chip(state, ["Running", "Idle"].includes(state) ? "" : "warn")}</div><div class="uv-session-meta"><div><label>Model</label><strong>${escapeHtml(r.kind === "pty" ? "Not applicable" : r.model || "Not reported")}</strong><small>${escapeHtml(r.requested_model ? "Configured: " + r.requested_model : r.source)}</small></div><div><label>Task state</label><strong>${escapeHtml(r.task)}</strong><small>Separate from native process state</small></div><div><label>Started</label><strong>${escapeHtml(r.started)}</strong><small>${escapeHtml(r.id)}</small></div></div>${r.recording_warning ? `<div class="banner warn">Recording warning: ${escapeHtml(r.recording_warning)}</div>` : ""}<div class="uv-session-actions"><span>${icon("clock", 13)} ${escapeHtml(d?.lastSeen || "Observation unavailable")}</span><div class="inline-actions">${button("History", "data-history", r.id)}${stopButton(r, d)}</div></div>${stopResults.has(r.id) ? `<div class="banner warn">${escapeHtml(stopResults.get(r.id).message)}</div>` : ""}</article>`;
    })
    .join("");
}
function ownerView() {
  if (selectedDevice) return deviceView(true);
  const devices = ownedDevices(currentUser()),
    runs = userRuns(currentUser()),
    active = activeRuns(runs),
    agents = active.filter((r) => r.kind === "agent"),
    models = agents.filter((r) => r.model);
  return `<div class="uv-root">${heading("Personal workspace / device control", "Your agents. Your devices. One clear picture.", "Current authorized host observations. A disconnected host can keep running work locally.")}${inventoryMetrics(devices, runs)}<div class="uv-insight-grid"><section class="uv-panel"><div class="uv-panel-title"><h2>${icon("laptop", 17)} Connection coverage</h2></div><div class="uv-chart-body">${svgDonut(
    [
      {
        name: "Connected",
        value: devices.filter((d) => d.online).length,
        color: "#4878f5",
      },
      {
        name: "Offline or disabled",
        value: devices.filter((d) => !d.online).length,
        color: "#edac42",
      },
    ],
    devices.filter((d) => d.online).length + "/" + devices.length,
    "hosts current",
  )}<p>${devices.length} enrolled devices.<br>Offline does not mean native work stopped.</p></div></section><section class="uv-panel"><div class="uv-panel-title"><h2>${icon("chart", 17)} Where sessions run</h2></div><div class="pad">${deviceBars(devices)}</div></section><section class="uv-panel"><div class="uv-panel-title"><h2>${icon("model", 17)} Model visibility</h2></div><div class="uv-chart-body">${svgDonut(
    [
      { name: "Reported", value: models.length, color: "#e46aab" },
      {
        name: "Not reported",
        value: agents.length - models.length,
        color: "#97a5ba",
      },
    ],
    models.length + "/" + agents.length,
    "models reported",
  )}<p>Terminal sessions are excluded.<br>${agents.filter((r) => r.requested_model).length} have configured model metadata. Effective models are never inferred from configuration or terminal output.</p></div></section></div><div class="uv-section-title"><div><div class="uv-kicker">YOUR ENROLLED HOSTS</div><h2>A home for every session.</h2></div></div>${deviceCards(devices, true)}<div class="uv-section-title"><div><div class="uv-kicker">NATIVE EXECUTION</div><h2>Agents and terminal sessions <span>${runs.length}</span></h2></div></div><div class="uv-session-layout"><div class="uv-session-list">${runs.length ? sessionCards(runs) : empty("No native sessions reported", "Start a session with the CLI on an enrolled host, then refresh.")}</div><div class="uv-context"><section class="uv-permission-note">${icon("shield", 25)}<h3>Stopping is an explicit decision.</h3><p>A confirmed stop targets one native session. Other sessions and device access continue.</p><p>A missing reply remains unknown. Check the original session state instead of sending a new stop.</p></section><section class="uv-usage-card"><div class="uv-kicker">USAGE REPORTING</div><h3>Not collected</h3><p>The current native projection does not report normalized token or billing data. No estimate is substituted.</p></section></div></div></div>`;
}
function deviceBars(devices) {
  const max = Math.max(
    1,
    ...devices.map((d) => activeRuns(deviceRuns(d.id)).length),
  );
  return devices.length
    ? devices
        .map((d) => {
          const n = activeRuns(deviceRuns(d.id)).length;
          return `<div style="margin-bottom:15px"><div class="minihead" style="padding-bottom:6px"><span class="subtle">${escapeHtml(d.name)}</span><b>${n}</b></div><div class="distribution-bar" style="height:6px;background:var(--bg);border-radius:5px"><i style="display:block;width:${(100 * n) / max}%;height:100%;background:${d.online ? "#4878f5" : "#edac42"};border-radius:5px"></i></div><small class="subtle">${unknownRuns(deviceRuns(d.id)).length} unknown</small></div>`;
        })
        .join("")
    : "<p>No enrolled devices.</p>";
}
function deviceView(owner) {
  const d = inventoryDevices.find((d) => d.id === selectedDevice);
  if (!d || (owner && d.owner !== currentUser()))
    return empty(
      "Device is outside your current scope",
      "Return to the inventory and refresh current authorization.",
    );
  return `<div class="av-root">${heading("Device / execution detail", d.name, `${d.kind} · ${d.os}`, button("Back to inventory", "data-back", "device"))}${inventoryMetrics([d], deviceRuns(d.id))}<div class="banner ${d.online ? "" : "warn"}">Last connected: ${escapeHtml(d.lastConnected)} · Last seen: ${escapeHtml(d.lastSeen)}${d.online ? "" : " · Current native work is unconfirmed."}</div>${card("Native sessions", executionTable(deviceRuns(d.id), owner))}</div>`;
}
function telemetryView() {
  const active = activeRuns(inventoryRuns),
    agents = active.filter((r) => r.kind === "agent"),
    missing = agents.filter((r) => !r.model).length,
    signals = telemetrySignals?.items || [];
  return `<div class="tv-dashboard">${heading("Administration / telemetry", "A pulse on every connection.", "Inventory and audited event aggregates. Missing sources stay explicit.")}${inventoryMetrics(inventoryDevices, inventoryRuns, inventoryUsers)}${telemetryStatus ? `<div class="banner warn">${escapeHtml(telemetryStatus)}</div>` : ""}<div class="tv-top-grid">${card("Accepted session starts", eventPlot(), chip("Audit source", "neutral"))}${coverageCard(inventoryDevices, inventoryRuns)}</div><div class="tv-mid-grid">${card(
    "Harness × model",
    `<div class="tv-model-layout">${svgDonut(
      [
        {
          name: "Model reported",
          value: agents.length - missing,
          color: "#7762ed",
        },
        { name: "Not reported", value: missing, color: "#edac42" },
      ],
      agents.length,
      "agent processes",
    )}<div class="tv-model-legend">${[...new Set(agents.map((r) => r.harness))].map((h) => `<div><span>${escapeHtml(h)}</span><b>${agents.filter((r) => r.harness === h).length}</b></div>`).join("")}<hr><div><span>Model not reported</span><b>${missing}</b></div><p>${agents.filter((r) => r.requested_model).length} have configured/requested models. Effective runtime model identity remains unreported.</p></div></div>`,
  )}${card("Execution by owner", ownerBars())}${card("Usage reporting", `<div class="pad"><div class="tv-usage-number">Unavailable</div><p>Normalized native token usage is not collected. This console does not infer billing or substitute sample data.</p>${chip("Not collected", "neutral")}</div>`)}</div><div style="margin-top:20px">${card("Collection and rule evaluation", `<div class="pad"><p>Inventory observation: ${escapeHtml(time(observedAt))}</p><p>Activity evaluation: ${escapeHtml(telemetrySourceLabel(telemetrySignals?.evaluation))}</p><p>Scheduled runs: ${telemetryRuns?.available === false ? escapeHtml(telemetrySourceLabel(telemetryRuns.reason)) : telemetryRuns?.available ? String(telemetryRuns.items?.length || 0) + " returned" : "Unavailable"}</p>${telemetrySignals?.audit_coverage_complete === false ? "<p>Audit coverage is partial. Event counts are lower bounds.</p>" : ""}</div>`)}</div><div class="tv-section-title"><div><h2>Activity and coverage signals</h2><p>No automatic revocation, stop, or abuse classification.</p></div></div><div class="tv-signal-grid">${signals.map((s, i) => `<article class="tv-signal-card searchable"><div class="tv-signal-top"><span class="tv-signal-icon">${icon("activity", 22)}</span>${chip(s.severity || "Coverage", "warn")}</div><h3>${escapeHtml(s.title)}</h3><p>${escapeHtml(s.machine)}</p><p>${escapeHtml(s.explanation)}</p><button data-signal="${escapeHtml(s.id)}">Inspect evidence ${icon("arrow", 14)}</button></article>`).join("") || empty("No coverage signals in this snapshot", "A clear snapshot is not a security assessment.")}</div><div class="banner" style="margin-top:20px">Statistical anomaly baselines are not established. Review signals use explicit audit thresholds or observation coverage; they do not establish misuse.</div></div>`;
}
function eventPlot() {
  if (!telemetrySeries?.available)
    return `<div class="no-data-plot"><div>${icon("chart", 30)}<h3>Event history is unavailable.</h3><p>${escapeHtml(telemetrySeries?.reason ? telemetrySourceLabel(telemetrySeries.reason) : "The audit source could not be read. No sample history is substituted.")}</p></div></div>`;
  const bins = telemetrySeries.items || [],
    max = Math.max(1, ...bins.map((b) => Number(b.value) || 0)),
    total = bins.reduce((sum, b) => sum + (Number(b.value) || 0), 0);
  return `<div class="time-series"><div class="chart-summary"><strong>${total}</strong><span>audited accepted start events</span></div><svg viewBox="0 0 620 230" role="img" aria-label="Audited accepted session start events in ${bins.length} time buckets"><path d="M30 190H610" stroke="var(--line)"/>${bins
    .map((b, i) => {
      const x = 35 + (i * 570) / Math.max(1, bins.length),
        h = (Math.max(0, Number(b.value) || 0) / max) * 145;
      return `<g><title>${escapeHtml(time(b.start_ms))}: ${Number(b.value) || 0} accepted events</title><rect x="${x}" y="${190 - h}" width="${Math.max(2, 520 / Math.max(1, bins.length))}" height="${h}" rx="3" fill="#7762ed"/><text x="${x}" y="${180 - h}">${Number(b.value) || 0}</text></g>`;
    })
    .join(
      "",
    )}<text x="30" y="216">${escapeHtml(time(telemetrySeries.start_ms))}</text><text x="610" y="216" text-anchor="end">${escapeHtml(time(telemetrySeries.end_ms))}</text></svg><p>${escapeHtml(telemetrySeries.unit)} per bucket · All users · ${telemetrySeries.complete ? "Complete source scan" : "Partial source scan; counts are lower bounds"}</p><p>Session-start event counts differ from currently running process counts.</p></div>`;
}
function auditView() {
  if (auditError)
    return heading(
      "Administration / audit",
      "Audit access is unavailable.",
      auditError,
    );
  const items = Array.isArray(auditData) ? auditData : [];
  return (
    heading(
      "Administration / audit",
      "A clear chain of evidence.",
      "Each audit read is itself audited. This view contains metadata, not transcript text.",
    ) +
    card(
      "Latest audit events",
      items.length
        ? `<div class="audit-events">${items.map((e) => `<article class="audit-event searchable"><h3>${escapeHtml(e.action)}</h3><p>Actor: ${escapeHtml(e.actor)} · ${escapeHtml(time(e.at || e.created_at || e.timestamp))}</p><p>${escapeHtml(e.machine || "")} ${escapeHtml(e.session || "")}</p><small class="subtle">Audit ID ${escapeHtml(e.id)}</small></article>`).join("")}</div>`
        : empty(
            "No audit events returned",
            "The query returned no events in this bounded page.",
          ),
    )
  );
}
function governanceView() {
  return (
    heading(
      "Administration / governance",
      "Make the boundary explicit.",
      "Existing durable actions retain their current authorization and audit contracts.",
    ) +
    `<div class="two">${card("Remote access revocation", `<div class="pad"><h3>Block remote access.</h3><p>Choose a user, machine, or global scope. Native work keeps running. A committed denial is separate from verified propagation.</p><button data-governance="revoke" class="primary">Review revocation</button></div>`)}${card("Legal hold", `<div class="pad"><h3>Preserve eligible records.</h3><p>Platform admin only. Place, update, or release a revisioned hold with a reason and optional case reference. Held records stay invisible to users.</p><button data-governance="hold">Change a legal hold</button></div>`)}</div><div class="card pad" style="margin-top:20px"><h2>Organization policy management</h2><p>A durable policy editing API is not configured. Host-side restrictions remain enforced by the native owner. This console offers no simulated policy save.</p><button disabled>Policy editing unavailable</button></div>`
  );
}
let historyTarget = null,
  historyData = null,
  historyError = null;
function historyView() {
  if (!historyTarget)
    return (
      heading(
        "Session recordings",
        "Protected history metadata",
        "Select a session to read its protected recording stream metadata.",
      ) +
      card(
        "Authorized sessions",
        `<div class="audit-events">${inventoryRuns.map((r) => `<div class="audit-event searchable"><h3>${escapeHtml(r.name)}</h3><p>${escapeHtml(r.id)} · ${escapeHtml(r.started)}</p>${button("Read stream metadata", "data-history", r.id)}</div>`).join("") || "<p>No sessions in this inventory scope.</p>"}</div>`,
      )
    );
  return (
    heading(
      "Protected recording metadata",
      historyTarget.name,
      "Recording reads are authorized and audited by the gateway.",
      button("All recordings", "data-back", "history"),
    ) +
    card(
      "Recording streams",
      `<div class="pad">${view === "admin" ? '<form id="history-reason-form" class="history-form"><label for="history-reason">Investigation reason</label><textarea id="history-reason" required maxlength="512"></textarea><label for="history-case">Case reference (required for held content)</label><input id="history-case" maxlength="256"><button class="primary">Read protected stream metadata</button></form>' : ""}${historyError ? `<p role="alert">${escapeHtml(historyError)}</p>` : ""}${historyData ? `<p>${historyData.streams?.length || 0} recording streams returned.</p>${(historyData.streams || []).map((s) => `<div class="audit-event"><h3>${escapeHtml(s.stream)}</h3><p>${escapeHtml(s.stream_epoch)}</p></div>`).join("")}${historyData.next ? "<p>Additional streams exist. Use the CLI for complete protected history.</p>" : ""}` : ""}<div class="banner" style="margin-top:20px">Recorded chunks use protected compressed formats. Readable playback remains available in the CLI. This view does not claim that capture is complete or decode unverified chunks.</div></div>`,
    )
  );
}
function healthView() {
  if (healthError || !healthData)
    return heading(
      "Administration / system health",
      "Health observations are unavailable.",
      healthError || "No current observation was returned.",
    );
  const components = healthData.components || {};
  const definitions = [
    ["database", "Database", "server", "Schema compatibility probe"],
    ["routing", "Routing broker", "activity", "Bounded broker observation"],
    [
      "identity",
      "Identity verification",
      "shield",
      "Gateway identity configuration",
    ],
    [
      "directory",
      "Directory freshness",
      "users",
      "Authoritative verification coverage",
    ],
    [
      "recording",
      "Recording services",
      "file",
      "Recording service configuration",
    ],
    ["audit_reader", "Audit reader", "file", "Privileged audited read service"],
    [
      "recording_lifecycle",
      "Recording lifecycle",
      "clock",
      "Preservation and retention service",
    ],
  ];
  const tone = (status) =>
    ["ready", "observed_healthy"].includes(status)
      ? ""
      : ["degraded", "unavailable"].includes(status)
        ? "warn"
        : "neutral";
  return `<div class="av-root">${heading("Administration / system health", "The accepting gateway, observed.", "Current process and dependency observations. This is not a fleet-wide health claim.")}<div class="banner">Observed ${escapeHtml(time(healthData.observed_at_ms))} · Scope: ${escapeHtml(healthData.scope)} · ${chip(healthData.status, tone(healthData.status))}</div><div class="av-people-grid">${definitions
    .map(([key, label, i, description]) => {
      const c = components[key] || {};
      return `<section class="card pad"><div class="inline-actions" style="align-items:center;justify-content:space-between"><span class="icon">${icon(i, 22)}</span>${chip(c.status || "Not reported", tone(c.status))}</div><h2 style="margin-top:18px">${label}</h2><p>${description}</p>${c.configured !== undefined ? `<p>Configured: ${c.configured ? "Yes" : "No"}</p>` : ""}${key === "directory" && c.active_subjects != null ? `<p>${Number(c.active_subjects)} tracked subjects · ${Number(c.stale_subjects || 0)} stale</p>` : ""}${key === "recording" ? `<p>Commit durability: ${escapeHtml(c.commit_durability || "Not probed")}</p>` : ""}</section>`;
    })
    .join("")}</div><div style="margin-top:22px">${card(
    "Connection observations",
    healthData.connections
      ? `<div class="pad"><div class="metric-strip">${[
          ["Clients", healthData.connections.clients],
          ["Hosts", healthData.connections.hosts],
          ["Pending requests", healthData.connections.pending],
          ["Cross-node messages", healthData.connections.cross_node_messages],
        ]
          .map(
            ([label, value]) =>
              `<div class="metric"><div class="label">${escapeHtml(label)}</div><div class="number">${Number.isFinite(value) ? value : "Not reported"}</div></div>`,
          )
          .join("")}</div></div>`
      : empty(
          "Routing counts are unavailable",
          "The current gateway did not return connection observations.",
        ),
  )}</div><div class="banner" style="margin-top:20px">Private scrape and OTel endpoints remain private. Database replication, recording commit durability, historical latency, and other gateways need their own verified monitoring sources.</div></div>`;
}
