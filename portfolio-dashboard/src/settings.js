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
    // The passkey card and the header badge report the session this just opened.
    window.PasskeyPanel?.refresh?.();
    window.SessionBadge?.refresh?.();
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

  // ---- Session length ----------------------------------------------------
  const sessionMinutes = document.getElementById("session-minutes");
  const sessionLengthStatus = document.getElementById("session-length-status");

  // The broker owns the stored length; its enrollment gate reports it without a
  // session, so the greyed-out control can still show the real value.
  async function loadSessionLength() {
    const response = await post("/messages/auth/enrollment", {}).catch(() => null);
    if (!response?.ok) return;
    const body = await response.json().catch(() => ({}));
    if (Number.isFinite(body.minutes) && sessionMinutes) sessionMinutes.value = String(body.minutes);
  }

  function setSessionLength() {
    if (!sessionMinutes) return;
    sessionMinutes.disabled = !unlocked;
    if (sessionLengthStatus && !unlocked) sessionLengthStatus.textContent = "";
  }

  sessionMinutes?.addEventListener("change", async () => {
    sessionMinutes.disabled = true;
    if (sessionLengthStatus) sessionLengthStatus.textContent = "Saving…";
    const response = await post("/messages/auth/session-length", {
      minutes: Number(sessionMinutes.value),
    }).catch(() => null);
    if (!response?.ok) {
      if (sessionLengthStatus) sessionLengthStatus.textContent =
        "That length was not accepted. Check the session and try again.";
      setSessionLength();
      return;
    }
    const body = await response.json().catch(() => ({}));
    if (Number.isFinite(body.minutes)) sessionMinutes.value = String(body.minutes);
    if (sessionLengthStatus) sessionLengthStatus.textContent =
      `Unlocks now last ${sessionMinutes.value} minutes.`;
    setSessionLength();
    // The session in use moved to the new length, so its countdown must too.
    window.SessionBadge?.refresh?.();
  });

  // ---- Host operations ---------------------------------------------------
  const operationStatus = document.getElementById("operation-status");
  const operationsLock = document.getElementById("operations-lock");
  const restartControls = [...document.querySelectorAll("[data-restart]")];
  const rebootForm = document.getElementById("reboot-form");
  const rebootConfirm = document.getElementById("reboot-confirm");
  const rebootSubmit = rebootForm?.querySelector("button[type=submit]");

  function setOperationStatus(text) {
    if (operationStatus) operationStatus.textContent = text;
  }

  // These change the machine, so they stay greyed out until the header badge
  // reports an admin session instead of asking for a passkey on click.
  function setOperations() {
    for (const control of restartControls) control.disabled = !unlocked;
    if (rebootSubmit) rebootSubmit.disabled = !unlocked;
    if (rebootConfirm) rebootConfirm.disabled = !unlocked;
    // The note waits for a settled answer so an unlocked page never flashes it.
    if (operationsLock) {
      operationsLock.hidden = (window.SessionBadge?.current?.() ?? "checking") !== "locked";
    }
  }

  async function sessionLost() {
    setOperationStatus("That needs an active admin session. Unlock with your passkey above.");
    await window.SessionBadge?.refresh?.();
    syncSession();
  }

  for (const control of restartControls) {
    control.addEventListener("click", async () => {
      const target = control.dataset.restart;
      control.disabled = true;
      setOperationStatus(`Restarting ${target}…`);
      const response = await post(`/api/admin/restart/${target}`, {}).catch(() => null);
      if (response?.status === 401 || response?.status === 403) {
        await sessionLost();
        return;
      }
      setOperationStatus(response?.ok
        ? `${target} restart requested.`
        : "The host agent did not answer.");
      setOperations();
    });
  }

  rebootForm?.addEventListener("submit", async event => {
    event.preventDefault();
    if (rebootConfirm.value.trim().toUpperCase() !== "REBOOT") {
      setOperationStatus("Type REBOOT to confirm.");
      return;
    }
    setOperationStatus("Requesting reboot…");
    const response = await post("/api/admin/reboot", {confirm: "REBOOT"}).catch(() => null);
    if (response?.status === 401 || response?.status === 403) {
      await sessionLost();
      return;
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
  let term = null;
  let offset = 0;

  function say(text) {
    if (terminalStatus) terminalStatus.textContent = text;
  }

  // The shell draws its own screen, so the bytes go through a grid: a carriage
  // return rewrites the line it is on, and a clear sequence empties it.
  function paint() {
    if (!term) return;
    const line = parseFloat(getComputedStyle(screen).lineHeight) || 19;
    // Stay where the reader scrolled: only follow the tail when already there.
    const tail = screen.scrollTop + screen.clientHeight >= screen.scrollHeight - line - 4;
    term.render();
    if (tail) screen.scrollTop = screen.scrollHeight;
  }

  function append(text) {
    if (!term) return;
    term.write(text);
    paint();
  }

  // The PTY has to wrap where the page does, so the columns come from the box
  // the screen is drawn in and the font it uses.
  function size() {
    const style = getComputedStyle(screen);
    const canvas = size.canvas || (size.canvas = document.createElement("canvas"));
    const context = canvas.getContext("2d");
    context.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    const width = context.measureText("0123456789").width / 10 || 8;
    const inner = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight)
      + parseFloat(style.borderLeftWidth) + parseFloat(style.borderRightWidth);
    const vertical = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom)
      + parseFloat(style.borderTopWidth) + parseFloat(style.borderBottomWidth);
    let box = 0;
    if (terminal?.clientWidth) {
      const card = getComputedStyle(terminal);
      box = terminal.clientWidth
        - parseFloat(card.paddingLeft) - parseFloat(card.paddingRight)
        - parseFloat(card.borderLeftWidth) - parseFloat(card.borderRightWidth) - inner - 12;
    } else if (screen.clientWidth) {
      box = screen.clientWidth - inner - 12;
    }
    const height = parseFloat(style.maxHeight) || 352;
    const line = parseFloat(style.lineHeight) || 19;
    return {
      cols: box > 240 ? Math.max(40, Math.floor(box / width)) : 100,
      // One row of slack keeps the last line above a scrollbar.
      rows: Math.max(10, Math.min(40, Math.floor((height - vertical) / line) - 1)),
    };
  }

  // An open shell is its own state, and so is a closed one the live session
  // still covers: that one opens in a click instead of asking for the passkey.
  let shell = false;
  let unlocked = false;

  function renderTerminal() {
    screen.hidden = !shell;
    terminalForm.hidden = !shell;
    closeButton.hidden = !shell;
    unlockButton.hidden = shell;
    unlockButton.textContent = unlocked ? "Open shell" : "Unlock with passkey";
    const state = shell ? "open" : unlocked ? "ready" : "locked";
    if (terminal) terminal.dataset.state = state;
    if (badge) {
      badge.textContent = shell ? "Shell open" : unlocked ? "Unlocked" : "Locked";
      badge.dataset.state = state;
    }
  }

  // Every owner control on this page follows the one session the badge holds.
  function applySession() {
    renderTerminal();
    setOperations();
    setSessionLength();
  }

  function syncSession() {
    unlocked = window.SessionBadge?.current?.() === "unlocked";
    applySession();
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
        endSession(unlocked
          ? "Shell exited. Open a new one when you need it."
          : "Shell exited. Unlock again for a new session.");
        return;
      }
    }
  }

  async function startSession() {
    say("Opening a shell…");
    const {cols, rows} = size();
    const response = await post("/api/terminal/open", {cols, rows}).catch(() => null);
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
    shell = true;
    term = window.WebTerm?.create(screen, cols, rows) ?? null;
    renderTerminal();
    paint();
    say("Connected. Line input; the agent runs a real shell as your account.");
    terminalLine.focus();
    pump();
  }

  function endSession(text) {
    session = null;
    shell = false;
    renderTerminal();
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

  // The header badge owns the session; a lock ends the shell it was guarding
  // and returns the button to the passkey, while an unlock lets the closed
  // terminal reopen without a second ceremony.
  window.addEventListener("session:locked", () => {
    window.PasskeyPanel?.refresh?.();
    unlocked = false;
    if (shell) endSession("Session locked. Unlock again for a new shell.");
    applySession();
  });
  window.addEventListener("session:unlocked", () => {
    window.PasskeyPanel?.refresh?.();
    unlocked = true;
    applySession();
    loadSessionLength();
  });
  // The broker's first answer is not a change, but it still decides whether
  // the terminal, the session length and the host operations are available.
  window.addEventListener("session:settled", syncSession);
  syncSession();

  if (settingsView && !settingsView.hidden) {
    loadSettings();
    loadSessionLength();
  }
})();
