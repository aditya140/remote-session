"use strict";
function evidence(title, html) {
  $("#evidence-title").textContent = title;
  $("#evidence-body").innerHTML = html;
  $("#evidence-dialog").showModal();
}
async function showSignal(id) {
  const signal = telemetrySignals?.items?.find((s) => s.id === id);
  if (!signal) return;
  const e = signal.evidence || {};
  const rows = [];
  if (signal.machine) rows.push(["Machine", signal.machine]);
  for (const [key, label] of [
    ["last_seen_ms", "Last seen"],
    ["window_start_ms", "Window start"],
    ["window_end_ms", "Window end"],
  ])
    if (e[key] != null) rows.push([label, time(e[key])]);
  if (e.freshness_ms != null)
    rows.push([
      "Freshness threshold",
      Number(e.freshness_ms) / 1000 + " seconds",
    ]);
  if (e.threshold != null) rows.push(["Review threshold", e.threshold]);
  if (e.distinct_resources != null)
    rows.push(["Distinct resources", e.distinct_resources]);
  if (e.complete != null)
    rows.push(["Source scan", e.complete ? "Complete" : "Partial"]);
  evidence(
    signal.title,
    `<p>${escapeHtml(signal.explanation)}</p><div class="audit-event"><h3>Observed evidence</h3><p>Subject: ${escapeHtml(signal.subject)}</p>${rows.map(([label, value]) => `<p>${escapeHtml(label)}: ${escapeHtml(value)}</p>`).join("")}${Array.isArray(e.event_ids) ? `<p>Audit event references: ${e.event_ids.slice(0, 30).map(escapeHtml).join(", ")}${e.event_ids.length > 30 ? " (additional references omitted from this view)" : ""}</p>` : ""}</div><p>This is a review signal, not a finding of misuse. No approval, revocation, or termination happens automatically.</p>`,
  );
}

async function openHistory(id) {
  const run = inventoryRuns.find((run) => run.id === id);
  if (!run || (view === "workspace" && run.owner !== currentUser())) return;
  historyTarget = run;
  historyData = null;
  historyError = null;
  historyPage = null;
  historyReason = null;
  page = "Session history";
  render();
  if (view === "workspace") await readStreams();
}
let historyPage = null,
  historyReason = null;
async function readStreams(reason = null, caseRef = null) {
  if (!historyTarget) return;
  const id = historyTarget.id;
  const isAdmin = view === "admin";
  historyReason = isAdmin
    ? { reason, case_ref: caseRef || null, include_held: false }
    : null;
  try {
    historyData = await api(
      `/v1/${isAdmin ? "admin/" : ""}sessions/${encodeURIComponent(id)}/history/streams`,
      {
        method: "POST",
        body: JSON.stringify(
          isAdmin
            ? { history: { after: null, limit: 32 }, ...historyReason }
            : { after: null, limit: 32 },
        ),
      },
    );
    historyError = null;
  } catch (e) {
    historyError = e.message;
  }
  if (historyTarget?.id === id) render();
}
async function readHistoryPage(epoch, after = null) {
  if (!historyTarget) return;
  const id = historyTarget.id,
    isAdmin = view === "admin";
  if (isAdmin && !historyReason?.reason) {
    notify("State an investigation reason before reading protected content.");
    return;
  }
  try {
    const query = { stream_epoch: epoch, after, limit: 4 };
    historyPage = await api(
      `/v1/console/${isAdmin ? "admin/" : ""}sessions/${encodeURIComponent(id)}/history`,
      {
        method: "POST",
        body: JSON.stringify(
          isAdmin ? { history: query, ...historyReason } : query,
        ),
      },
    );
    historyError = null;
  } catch (e) {
    historyError = e.message;
    historyPage = null;
  }
  if (historyTarget?.id === id) render();
}
function recordText(record) {
  const content = record.content || {};
  if (content.type === "resize")
    return `[terminal resized to ${Number(content.rows)} × ${Number(content.cols)}]\n`;
  if (
    ["output", "agent_text"].includes(content.type) &&
    typeof content.data === "string"
  ) {
    try {
      const bytes = Uint8Array.from(atob(content.data), (c) => c.charCodeAt(0));
      return new TextDecoder("utf-8", { fatal: false })
        .decode(bytes)
        .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, (c) =>
          c === "\x1b" ? "␛" : "�",
        );
    } catch {
      return "[record data could not be decoded]\n";
    }
  }
  return "[unsupported record]\n";
}
function renderHistoryRecords() {
  if (!historyPage) return "";
  return `<section><h2>Protected recording page</h2><p>Explicit gaps remain visible. Content is rendered as text; terminal controls cannot execute.</p>${(historyPage.items || []).map((item, index) => (item.kind === "gap" ? `<div class="banner warn"><b>Recording gap</b><p>${escapeHtml(item.gap?.reason || "Captured output is unavailable.")}</p></div>` : `<div class="audit-event"><small class="subtle">Chunk ${escapeHtml(item.id)} · Sequence ${escapeHtml(item.from)}–${escapeHtml(item.to)}${item.redaction_saturated ? " · Redaction limit reported" : ""}</small><pre class="history-content" data-history-record="${index}"></pre></div>`)).join("") || "<p>No content or gap records in this page.</p>"}${historyPage.next ? `<button data-history-next="${escapeHtml(historyPage.next)}" data-history-epoch="${escapeHtml(historyPage.stream_epoch)}">Read next protected page</button>` : '<p class="subtle">End of this committed page range. This does not certify complete native capture.</p>'}</section>`;
}
const streamHistoryView = historyView;
historyView = function () {
  const html = streamHistoryView();
  if (!historyTarget) return html;
  const extra = historyData
    ? `<div class="links-row">${(historyData.streams || []).map((s) => `<button data-history-epoch="${escapeHtml(s.stream_epoch)}">Read ${escapeHtml(s.stream)} stream</button>`).join("")}</div>`
    : "";
  return html.replace(
    '<div class="banner" style="margin-top:20px">Recorded chunks use protected compressed formats. Readable playback remains available in the CLI. This view does not claim that capture is complete or decode unverified chunks.</div>',
    extra + renderHistoryRecords(),
  );
};
function governanceDialog(kind) {
  const revoke = kind === "revoke";
  const requestId = (revoke ? "req_" : "hold_") + crypto.randomUUID();
  evidence(
    revoke ? "Revoke remote access" : "Change a legal hold",
    `<form id="governance-form" data-kind="${kind}" data-original="${requestId}" class="history-form">${revoke ? '<label for="governance-scope">Access scope</label><select id="governance-scope"><option value="machine">Machine</option><option value="user">User</option><option value="global">Global remote access</option></select><label for="governance-subject">Exact machine or user ID (unused for global scope)</label><input id="governance-subject" autocomplete="off" maxlength="64">' : `<label for="hold-id">Hold ID</label><input id="hold-id" required value="${requestId}" autocomplete="off"><label for="hold-user">Exact affected user ID</label><input id="hold-user" required autocomplete="off"><label for="hold-revision">Revision (1 for new hold; next revision for change/release)</label><input id="hold-revision" required value="1" inputmode="numeric" pattern="[1-9][0-9]*"><label for="hold-active">Action</label><select id="hold-active"><option value="true">Place or update hold</option><option value="false">Release hold</option></select><label for="hold-reason">Reason</label><textarea id="hold-reason" required maxlength="512"></textarea><label for="hold-case">Case reference</label><input id="hold-case" maxlength="256">`}<label for="governance-confirm">Type ${revoke ? "REVOKE" : "HOLD"} to confirm</label><input id="governance-confirm" required pattern="${revoke ? "REVOKE" : "HOLD"}" autocomplete="off"><p>${revoke ? "Denial affects remote access. Native processes continue." : "One authorized platform admin may change this hold. Users do not see hold activity."}</p><p class="subtle">Original identity: ${requestId}</p><div id="governance-result" role="status"></div><button id="governance-submit" class="primary">Commit confirmed change</button></form>`,
  );
}
async function commitGovernance(form, event) {
  event.preventDefault();
  if (!auth?.platform_admin || view !== "admin") return;
  const kind = form.dataset.kind,
    id = form.dataset.original;
  const button = $("#governance-submit");
  button.disabled = true;
  let payload;
  if (kind === "revoke") {
    const scope = $("#governance-scope").value,
      value = $("#governance-subject").value.trim();
    payload = {
      request: id,
      target:
        scope === "global"
          ? { kind: "global" }
          : scope === "user"
            ? { kind: "user", user: value }
            : { kind: "machine", machine: value },
      confirmed: true,
    };
  } else {
    payload = {
      id: $("#hold-id").value.trim(),
      user: $("#hold-user").value.trim(),
      revision: $("#hold-revision").value,
      active: $("#hold-active").value === "true",
      case_ref: $("#hold-case").value.trim() || null,
      reason: $("#hold-reason").value.trim(),
    };
  }
  try {
    const result = await api(
      kind === "revoke" ? "/v1/admin/revocations" : "/v1/admin/holds",
      { method: "POST", body: JSON.stringify(payload) },
    );
    $("#governance-result").textContent =
      kind === "revoke"
        ? `Remote access denial is committed. Propagation: ${result.propagation}. Request ${result.request}.`
        : `Hold ${result.id} revision ${result.revision} committed. Active: ${result.active}.`;
    if (kind === "revoke")
      $("#governance-result").insertAdjacentHTML(
        "beforeend",
        `<p><button type="button" data-check-revocation="${escapeHtml(id)}">Check original propagation status</button></p>`,
      );
  } catch (error) {
    $("#governance-result").textContent =
      `Outcome needs verification: ${error.message}. The original identity is ${kind === "revoke" ? id : payload.id}. No request is automatically retried.`;
    if (kind === "revoke")
      $("#governance-result").insertAdjacentHTML(
        "beforeend",
        `<p><button type="button" data-check-revocation="${escapeHtml(id)}">Check original request status</button></p>`,
      );
  }
}
async function boot() {
  try {
    const session = await api("/console/auth/session");
    if (!session.authenticated) {
      renderLogin();
      return;
    }
    auth = session;
    const verified = await api("/v1/me");
    if (verified.user !== session.user)
      throw new Error("The browser session identity changed. Sign in again.");
    auth.platform_admin = verified.platform_admin;
    restoreStopResults();
    view =
      new URLSearchParams(location.search).get("view") === "admin" &&
      auth.platform_admin
        ? "admin"
        : "workspace";
    page = view === "admin" ? "Overview" : "My devices";
    await loadData();
  } catch (error) {
    renderLogin(error.message);
  }
}
$("#search").addEventListener("input", filterVisible);
$("#refresh").addEventListener("click", () => loadData());
$("#stop-form").addEventListener("submit", submitStop);
$("#stop-cancel").onclick = () => {
  if (!$("#stop-submit").disabled) {
    $("#stop-dialog").close();
    pendingStop = null;
  }
};
$("#stop-dialog").addEventListener("cancel", (event) => {
  if ($("#stop-submit").disabled) event.preventDefault();
});
$("#evidence-close").onclick = () => $("#evidence-dialog").close();
$("#logout").onclick = async () => {
  if ($("#stop-dialog").open && $("#stop-submit").disabled) {
    notify(
      "A native action is awaiting its result. Wait for its outcome before signing out.",
    );
    return;
  }
  try {
    await api("/console/auth/logout", { method: "POST" });
    auth = null;
    inventoryUsers = [];
    inventoryDevices = [];
    inventoryRuns = [];
    stopResults.clear();
    renderLogin("You have signed out. Native work continues.");
  } catch (e) {
    notify(e.message);
  }
};
document.addEventListener("click", async (event) => {
  const b = event.target.closest("button");
  if (!b) return;
  try {
    if (b.id === "retry-load") {
      await loadData();
    } else if (b.dataset.view) {
      if (b.dataset.view === "admin" && !auth?.platform_admin) return;
      view = b.dataset.view;
      page = view === "admin" ? "Overview" : "My devices";
      selectedUser = null;
      selectedDevice = null;
      historyTarget = null;
      observedAt = null;
      inventoryUsers = [];
      inventoryDevices = [];
      inventoryRuns = [];
      await loadData();
    } else if (b.dataset.page) {
      page = b.dataset.page;
      selectedUser = null;
      selectedDevice = null;
      historyTarget = null;
      render();
      await loadData({ quiet: true });
    } else if (b.dataset.user) {
      if (view !== "admin") return;
      selectedUser = b.dataset.user;
      selectedDevice = null;
      page = "Users";
      render();
    } else if (b.dataset.device) {
      const device = inventoryDevices.find((d) => d.id === b.dataset.device);
      if (!device || (view === "workspace" && device.owner !== currentUser()))
        return;
      selectedDevice = device.id;
      render();
    } else if (b.dataset.back) {
      if (b.dataset.back === "users") selectedUser = null;
      if (b.dataset.back === "history") {
        historyTarget = null;
        historyPage = null;
      }
      selectedDevice = null;
      render();
    } else if (b.dataset.stop) {
      await openStop(b.dataset.stop);
    } else if (b.dataset.reconcile) {
      await reconcileStop(b.dataset.reconcile);
    } else if (b.dataset.signal) {
      await showSignal(b.dataset.signal);
    } else if (b.dataset.history) {
      await openHistory(b.dataset.history);
    } else if (b.dataset.historyEpoch) {
      await readHistoryPage(
        b.dataset.historyEpoch,
        b.dataset.historyNext || null,
      );
    } else if (b.dataset.governance) {
      if (auth.platform_admin && view === "admin")
        governanceDialog(b.dataset.governance);
    } else if (b.dataset.checkRevocation) {
      const result = await api(
        "/v1/admin/revocations/" +
          encodeURIComponent(b.dataset.checkRevocation),
      );
      $("#governance-result").textContent =
        `Request ${result.request}: ${result.propagation}. Native sessions continue.`;
    }
  } catch (error) {
    notify(error.message);
  }
});
document.addEventListener("submit", (event) => {
  if (event.target.id === "history-reason-form") {
    event.preventDefault();
    readStreams(
      $("#history-reason").value.trim(),
      $("#history-case").value.trim(),
    );
  } else if (event.target.id === "governance-form")
    commitGovernance(event.target, event);
});
document.addEventListener("keydown", (event) => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    $("#search").focus();
  }
});
setInterval(() => {
  if (
    auth &&
    !document.hidden &&
    !loading &&
    !$("#stop-dialog").open &&
    !$("#evidence-dialog").open &&
    !$("#history-reason-form")
  )
    loadData({ quiet: true });
}, 10000);
window.addEventListener("pagehide", () => loadController?.abort());
boot();

function paintHistoryText() {
  document.querySelectorAll("[data-history-record]").forEach((node) => {
    const item = historyPage?.items?.[Number(node.dataset.historyRecord)];
    node.textContent = (item?.records || []).map(recordText).join("");
  });
}
