"use strict";

(() => {
  const settingsView = document.getElementById("view-settings");
  const terminal = document.getElementById("webterm");
  if (!settingsView && !terminal) return;

  function post(url, body) {
    return fetch(url, {
      method: "POST",
      credentials: "same-origin",
      cache: "no-store",
      redirect: "error",
      headers: {"Content-Type": "application/json", Accept: "application/json"},
      body: JSON.stringify(body ?? {}),
    });
  }

  // One ceremony for every owner action: the broker owns the passkey, and the
  // dashboard only learns whether this browser now holds a live session.
  async function unlock() {
    const options = await post("/messages/auth/login-options", {});
    if (!options.ok) throw new Error("Passkey sign-in is unavailable. Ask the Pi administrator.");
    const {flow, publicKey} = await options.json();
    const assertion = await navigator.credentials.get({
      publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey),
    });
    if (!assertion) throw new Error("The passkey request was cancelled.");
    const finished = await post("/messages/auth/login-finish", {flow, credential: assertion.toJSON()});
    if (!finished.ok) throw new Error("That passkey was not accepted.");
  }

  // ---- Telemetry ---------------------------------------------------------
  const interval = document.getElementById("setting-interval");
  const retention = document.getElementById("setting-retention");
  const telemetryForm = document.getElementById("telemetry-form");
  const telemetryStatus = document.getElementById("telemetry-status");

  async function loadSettings() {
    if (!telemetryForm) return;
    try {
      const response = await fetch("/api/settings", {
        cache: "no-store", mode: "same-origin", redirect: "error",
        headers: {Accept: "application/json"},
      });
      if (!response.ok) throw new Error();
      const body = await response.json();
      interval.value = String(body.interval_secs);
      retention.value = String(body.retention);
    } catch {
      telemetryStatus.textContent = "Could not read the current cadence.";
    }
  }

  telemetryForm?.addEventListener("submit", async event => {
    event.preventDefault();
    telemetryStatus.textContent = "Saving…";
    const response = await post("/api/settings", {
      interval_secs: Number(interval.value),
      retention: Number(retention.value),
    }).catch(() => null);
    if (response?.status === 401 || response?.status === 403) {
      try {
        await unlock();
      } catch (error) {
        telemetryStatus.textContent = error.message;
        return;
      }
      return telemetryForm.requestSubmit();
    }
    if (!response?.ok) {
      telemetryStatus.textContent = "That change was not accepted.";
      return;
    }
    const body = await response.json();
    telemetryStatus.textContent = `Checking every ${body.interval_secs} s, keeping ${body.retention} checks.`;
  });

  // ---- Host operations ---------------------------------------------------
  const operationStatus = document.getElementById("operation-status");
  const rebootForm = document.getElementById("reboot-form");
  const rebootConfirm = document.getElementById("reboot-confirm");

  function setOperationStatus(text) {
    if (operationStatus) operationStatus.textContent = text;
  }

  for (const control of document.querySelectorAll("[data-restart]")) {
    control.addEventListener("click", async () => {
      const target = control.dataset.restart;
      control.disabled = true;
      setOperationStatus(`Restarting ${target}…`);
      let response = await post(`/api/admin/restart/${target}`, {}).catch(() => null);
      if (response?.status === 401 || response?.status === 403) {
        try {
          await unlock();
        } catch (error) {
          setOperationStatus(error.message);
          control.disabled = false;
          return;
        }
        response = await post(`/api/admin/restart/${target}`, {}).catch(() => null);
      }
      setOperationStatus(response?.ok
        ? `${target} restart requested.`
        : "The host agent did not answer.");
      control.disabled = false;
    });
  }

  rebootForm?.addEventListener("submit", async event => {
    event.preventDefault();
    if (rebootConfirm.value.trim().toUpperCase() !== "REBOOT") {
      setOperationStatus("Type REBOOT to confirm.");
      return;
    }
    setOperationStatus("Requesting reboot…");
    let response = await post("/api/admin/reboot", {confirm: "REBOOT"}).catch(() => null);
    if (response?.status === 401 || response?.status === 403) {
      try {
        await unlock();
      } catch (error) {
        setOperationStatus(error.message);
        return;
      }
      response = await post("/api/admin/reboot", {confirm: "REBOOT"}).catch(() => null);
    }
    setOperationStatus(response?.ok
      ? "Reboot requested. Everything reconnects in about a minute."
      : "The host agent did not answer.");
  });

  // ---- Web terminal ------------------------------------------------------
  const screen = document.getElementById("webterm-screen");
  const terminalForm = document.getElementById("webterm-form");
  const terminalLine = document.getElementById("webterm-line");
  const terminalStatus = document.getElementById("webterm-status");
  const unlockButton = document.getElementById("webterm-unlock");
  const closeButton = document.getElementById("webterm-close");
  const badge = document.getElementById("webterm-badge");
  const decoder = new TextDecoder();
  let session = null;
  let offset = 0;

  function say(text) {
    if (terminalStatus) terminalStatus.textContent = text;
  }

  function append(text) {
    if (!screen) return;
    screen.textContent += text;
    // A terminal that grows without bound eventually stalls the page.
    if (screen.textContent.length > 60000) screen.textContent = screen.textContent.slice(-40000);
    screen.scrollTop = screen.scrollHeight;
  }

  function showSession(open) {
    screen.hidden = !open;
    terminalForm.hidden = !open;
    closeButton.hidden = !open;
    unlockButton.hidden = open;
    badge.textContent = open ? "Connected" : "Locked";
    badge.dataset.state = open ? "good" : "locked";
  }

  async function pump() {
    while (session) {
      const current = session;
      const response = await post("/api/terminal/read", {id: current, offset}).catch(() => null);
      if (!response?.ok) {
        endSession("The terminal closed.");
        return;
      }
      const body = await response.json();
      if (body.data) {
        const bytes = Uint8Array.from(atob(body.data), char => char.charCodeAt(0));
        append(decoder.decode(bytes, {stream: true}));
      }
      offset = body.offset;
      if (body.ended) {
        endSession("Shell exited. Unlock again for a new session.");
        return;
      }
    }
  }

  async function startSession() {
    say("Opening a shell…");
    const response = await post("/api/terminal/open", {
      cols: Math.max(60, Math.floor((screen.clientWidth || 900) / 8)),
      rows: 30,
    }).catch(() => null);
    if (response?.status === 401 || response?.status === 403) {
      try {
        await unlock();
      } catch (error) {
        say(error.message);
        return;
      }
      return startSession();
    }
    if (!response?.ok) {
      say(response?.status === 503
        ? "The host agent is not installed or did not answer."
        : "The shell could not be opened.");
      return;
    }
    const body = await response.json();
    session = body.id;
    offset = 0;
    showSession(true);
    say("Connected. Line input; the agent runs a real shell as your account.");
    terminalLine.focus();
    pump();
  }

  function endSession(text) {
    session = null;
    showSession(false);
    say(text);
  }

  unlockButton?.addEventListener("click", startSession);

  closeButton?.addEventListener("click", async () => {
    if (session) await post("/api/terminal/close", {id: session}).catch(() => null);
    endSession("Session closed.");
  });

  terminalForm?.addEventListener("submit", async event => {
    event.preventDefault();
    if (!session) return;
    const line = terminalLine.value;
    terminalLine.value = "";
    const response = await post("/api/terminal/write", {id: session, data: `${line}\n`}).catch(() => null);
    if (!response?.ok) endSession("The terminal closed.");
  });

  window.addEventListener("beforeunload", () => {
    if (!session) return;
    navigator.sendBeacon?.("/api/terminal/close", new Blob([JSON.stringify({id: session})], {type: "application/json"}));
  });

  if (settingsView) loadSettings();
})();
