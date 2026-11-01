"use strict";
class ConsoleWire {
  constructor() {
    this.socket = null;
    this.pending = new Map();
    this.capabilities = new Set();
    this.closed = false;
  }
  envelope(type, body, target = {}) {
    return {
      v: 1,
      id: "msg_" + crypto.randomUUID(),
      ...target,
      type,
      ...(body === undefined ? {} : { body }),
    };
  }
  async connect() {
    const url = new URL("/v1/connect/client", location.href);
    url.protocol = location.protocol === "https:" ? "wss:" : "ws:";
    this.socket = new WebSocket(url);
    this.socket.binaryType = "arraybuffer";
    return new Promise((resolve, reject) => {
      let welcomed = false,
        ready = false;
      const timer = setTimeout(() => {
        reject(
          new Error("Gateway handshake timed out before any stop was sent."),
        );
        this.close();
      }, 8000);
      const fail = (message) => {
        clearTimeout(timer);
        if (!ready) reject(new Error(message));
        this.rejectPending(message);
      };
      this.socket.onopen = () =>
        this.socket.send(
          JSON.stringify(
            this.envelope("hello", {
              versions: [{ major: 1, minor: 0 }],
              required: ["routing-v1", "machine-pages-v1", "auth-refresh-v1"],
              optional: ["session-control-v1", "session-pages-v1"],
            }),
          ),
        );
      this.socket.onmessage = (event) => {
        if (typeof event.data !== "string" || event.data.length > 1024 * 1024) {
          fail("Unexpected gateway frame.");
          this.close();
          return;
        }
        let message;
        try {
          message = JSON.parse(event.data);
        } catch {
          fail("Invalid gateway response.");
          this.close();
          return;
        }
        if (message.v !== 1) {
          fail("Unsupported gateway response version.");
          this.close();
          return;
        }
        if (message.type === "welcome") {
          const body = message.body;
          if (body?.version?.major !== 1 || body?.version?.minor !== 0) {
            fail("The gateway protocol version is unsupported.");
            this.close();
            return;
          }
          this.capabilities = new Set(body.capabilities || []);
          welcomed = true;
          return;
        }
        if (message.type === "connection.ready") {
          if (
            !welcomed ||
            !message.body?.client ||
            message.body.host_generation != null ||
            message.body.activated !== false
          ) {
            fail("Gateway client activation was invalid.");
            this.close();
            return;
          }
          ready = true;
          clearTimeout(timer);
          resolve(this);
          return;
        }
        if (message.type === "error" && !ready) {
          fail(message.body?.message || "Gateway rejected the handshake.");
          this.close();
          return;
        }
        if (message.re && this.pending.has(message.re)) {
          const item = this.pending.get(message.re);
          this.pending.delete(message.re);
          clearTimeout(item.timer);
          if (
            message.type !== "error" &&
            ((item.target.machine && message.machine !== item.target.machine) ||
              (item.target.session && message.session !== item.target.session))
          ) {
            const error = new Error(
              "Gateway response identity did not match the original target.",
            );
            error.outcome = item.mutation ? "unknown" : "not_sent";
            item.reject(error);
            return;
          }
          if (message.type === "error") {
            const error = new Error(
              message.body?.message || "Gateway refused the operation.",
            );
            error.outcome = message.body?.outcome;
            error.code = message.body?.code;
            item.reject(error);
          } else item.resolve(message);
        }
      };
      this.socket.onerror = () => fail("Gateway connection failed.");
      this.socket.onclose = () => {
        this.closed = true;
        fail("Gateway connection closed. No operation was retried.");
      };
    });
  }
  request(type, body, target, mutation = false) {
    if (this.pending.size >= 8)
      return Promise.reject(new Error("Too many pending gateway reads."));
    if (this.closed || this.socket?.readyState !== WebSocket.OPEN)
      return Promise.reject(new Error("Gateway connection is unavailable."));
    const message = this.envelope(type, body, target);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(message.id);
        const error = new Error(
          mutation
            ? "Native stop outcome is unknown. The command was not resent."
            : "Gateway read timed out.",
        );
        error.outcome = mutation ? "unknown" : "not_sent";
        error.requestId = message.id;
        reject(error);
      }, 10000);
      this.pending.set(message.id, {
        resolve,
        reject,
        timer,
        mutation,
        target,
      });
      try {
        this.socket.send(JSON.stringify(message));
      } catch {
        clearTimeout(timer);
        this.pending.delete(message.id);
        const error = new Error(
          mutation
            ? "Native stop outcome is unknown."
            : "Gateway request could not be sent.",
        );
        error.outcome = mutation ? "unknown" : "not_sent";
        error.requestId = message.id;
        reject(error);
      }
    });
  }
  rejectPending(message) {
    for (const [id, p] of this.pending) {
      clearTimeout(p.timer);
      const error = new Error(message);
      error.outcome = p.mutation ? "unknown" : "not_sent";
      error.requestId = id;
      p.reject(error);
    }
    this.pending.clear();
  }
  close() {
    this.closed = true;
    this.socket?.close(1000, "Console operation finished");
    this.rejectPending("The observation connection closed.");
  }
  async session(run) {
    if (!this.capabilities.has("session-pages-v1"))
      throw new Error(
        "The gateway does not support explicit session lookup. No stop was sent.",
      );
    const reply = await this.request(
      "session.get",
      { session: run.id },
      { machine: run.device, session: run.id },
    );
    if (
      reply.type !== "session.found" ||
      reply.body?.id !== run.id ||
      reply.body?.machine !== run.device ||
      reply.body?.owner !== currentUser()
    )
      throw new Error("The session identity did not match. No stop was sent.");
    return reply.body;
  }
  async stop(run, externalConfirmed) {
    if (!this.capabilities.has("session-control-v1"))
      throw new Error(
        "The gateway did not negotiate session control. No stop was sent.",
      );
    const found = await this.session(run);
    if (["exited", "failed"].includes(found.state))
      return { alreadyEnded: true };
    if (["lost", "outcome_unknown"].includes(found.state))
      throw new Error(
        "The original native lifetime is lost or unconfirmed. No stop was sent.",
      );
    if (!found.capabilities?.terminate)
      throw new Error(
        "This session does not support termination. No stop was sent.",
      );
    if (found.origin === "external" && !externalConfirmed)
      throw new Error(
        "External session termination requires an additional confirmation. No stop was sent.",
      );
    const target = { machine: run.device, session: run.id };
    const generation = await this.request(
      "session.host.query",
      undefined,
      target,
    );
    if (
      generation.type !== "session.host" ||
      typeof generation.body?.generation !== "string" ||
      !/^\d{1,20}$/.test(generation.body.generation) ||
      BigInt(generation.body.generation) <= 0n ||
      BigInt(generation.body.generation) > 18446744073709551615n
    )
      throw new Error(
        "The current native host generation was not confirmed. No stop was sent.",
      );
    stopResults.set(run.id, {
      state: "Stop unknown",
      message:
        "A stop command is being submitted. Its original outcome must be verified before any new action.",
    });
    saveStopResults();
    const result = await this.request(
      "session.terminate",
      {
        host_generation: generation.body.generation,
        confirmed: true,
        external_confirmed: externalConfirmed,
      },
      target,
      true,
    );
    if (
      result.type !== "session.action.result" ||
      result.body?.action !== "terminate" ||
      !["applied", "already_applied"].includes(result.body?.outcome)
    ) {
      const error = new Error(
        "Native termination was not confirmed. The command was not resent.",
      );
      error.outcome = "unknown";
      throw error;
    }
    return result;
  }
}
function saveStopResults() {
  try {
    sessionStorage.setItem(
      "qgr-console-stops:" + currentUser(),
      JSON.stringify([...stopResults].slice(-500)),
    );
  } catch {
    /* A full private storage area must not cause a mutation retry. */
  }
}
function restoreStopResults() {
  try {
    const entries = JSON.parse(
      sessionStorage.getItem("qgr-console-stops:" + currentUser()) || "[]",
    );
    if (Array.isArray(entries))
      for (const [id, value] of entries.slice(-500))
        if (
          typeof id === "string" &&
          value &&
          ["Stop unknown", "Pending confirmation"].includes(value.state)
        )
          stopResults.set(id, {
            state: value.state,
            message: String(
              value.message || "Original stop outcome needs verification.",
            ),
            requestId: value.requestId,
          });
  } catch {
    /* Ignore malformed non-authoritative local display metadata. */
  }
}
let pendingStop = null;
async function openStop(id) {
  const run = inventoryRuns.find((run) => run.id === id),
    device = inventoryDevices.find((device) => device.id === run?.device);
  if (
    !run ||
    view !== "workspace" ||
    run.owner !== currentUser() ||
    !device?.online ||
    !run.terminate ||
    !["Running", "Idle"].includes(run.state) ||
    stopResults.has(id)
  ) {
    notify("No currently authorized stop action is available.");
    return;
  }
  pendingStop = run;
  $("#stop-target").textContent = `${run.name} · ${device.name} · ${run.id}`;
  $("#stop-confirmation").value = "";
  $("#external-confirmation").checked = false;
  $("#external-label").classList.toggle("hide", run.origin === "managed");
  $("#external-confirmation").required = run.origin !== "managed";
  $("#stop-error").textContent = "";
  $("#stop-submit").disabled = false;
  $("#stop-submit").textContent = "Stop session";
  $("#stop-dialog").showModal();
}
async function submitStop(event) {
  event.preventDefault();
  if (!pendingStop || $("#stop-confirmation").value !== "STOP") return;
  const run = pendingStop,
    external = $("#external-confirmation").checked;
  if (run.owner !== currentUser()) return;
  $("#stop-submit").disabled = true;
  $("#stop-submit").textContent = "Contacting native owner…";
  const wire = new ConsoleWire();
  try {
    await wire.connect();
    const result = await wire.stop(run, external);
    if (result.alreadyEnded) {
      stopResults.delete(run.id);
      saveStopResults();
      $("#stop-dialog").close();
      await loadData({ quiet: true });
      notify(
        "The original session had already ended. No stop command was sent.",
      );
      return;
    }
    stopResults.set(run.id, {
      state: "Pending confirmation",
      message:
        "The native owner acknowledged termination. Waiting for the projected session end.",
    });
    saveStopResults();
    $("#stop-dialog").close();
    notify(
      "Native termination acknowledged. Refreshing the original session state.",
    );
    await loadData({ quiet: true });
  } catch (error) {
    if (error.outcome === "unknown") {
      stopResults.set(run.id, {
        state: "Stop unknown",
        message: error.message,
        requestId: error.requestId,
      });
      saveStopResults();
      $("#stop-dialog").close();
      render();
      notify(
        "Stop outcome is unknown. Check the original session; no command was resent.",
      );
    } else {
      stopResults.delete(run.id);
      saveStopResults();
      $("#stop-error").textContent = error.message;
      $("#stop-submit").disabled = false;
      $("#stop-submit").textContent = "Stop session";
    }
  } finally {
    wire.close();
  }
}
async function reconcileStop(id) {
  const run = inventoryRuns.find((run) => run.id === id);
  if (!run || run.owner !== currentUser() || !stopResults.has(id)) return;
  const wire = new ConsoleWire();
  try {
    await wire.connect();
    const found = await wire.session(run);
    if (["exited", "failed", "lost"].includes(found.state)) {
      stopResults.delete(id);
      saveStopResults();
      await loadData({ quiet: true });
      notify("The original session has ended. No new stop command was sent.");
    } else {
      await loadData({ quiet: true });
      notify(
        "The original session end is still unconfirmed. No stop command was resent.",
      );
    }
  } catch (error) {
    notify(error.message + " The original stop remains unresolved.");
  } finally {
    wire.close();
  }
}
