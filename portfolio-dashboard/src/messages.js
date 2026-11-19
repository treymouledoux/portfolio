"use strict";

(() => {
  // The archive is readable without a passkey; this requests the fifteen-minute
  // owner session that the broker requires before anything changes.
  const sessionRequest = (step, body) => fetch("/messages/auth/" + step, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });

  // A locked or expired session is a server-rendered state, so the view is
  // replaced in place instead of reloading the document.
  function reload() {
    if (window.Dashboard) window.Dashboard.refreshMessages();
    else window.location.reload();
  }

  function notice(text) {
    const target = document.getElementById("inbox-notice");
    if (target) target.textContent = text;
  }

  async function unlockInbox(button) {
    button.disabled = true;
    try {
      const options = await sessionRequest("login-options");
      if (options.status === 404) throw new Error("No passkey is registered yet. Set one up in Settings.");
      if (!options.ok) throw new Error("Unlocking is unavailable. Check your connection and try again.");
      const { flow, publicKey } = await options.json();
      const assertion = await navigator.credentials.get({
        publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey),
      });
      if (!assertion) throw new Error("The passkey request was cancelled.");
      const finished = await sessionRequest("login-finish", { flow, credential: assertion.toJSON() });
      if (!finished.ok) throw new Error("That passkey was not accepted.");
      reload();
    } catch (error) {
      button.disabled = false;
      notice(error.name === "NotAllowedError"
        ? "The passkey request was cancelled or timed out."
        : (error.message || "Couldn’t unlock the inbox."));
    }
  }

  function lockInbox(button) {
    button.disabled = true;
    sessionRequest("lock").then(response => {
      if (!response.ok && response.status !== 401 && response.status !== 403) throw Error();
      reload();
    }).catch(() => {
      button.disabled = false;
      notice("Couldn’t lock the inbox. Check your connection and try again.");
    });
  }

  function markMessage(form) {
    const button = form.querySelector("button");
    button.disabled = false;
    form.addEventListener("submit", async event => {
      event.preventDefault();
      if (button.disabled) return;
      button.disabled = true;
      try {
        const response = await fetch(form.action, {
          method: "POST", credentials: "same-origin", cache: "no-store",
        });
        if (response.status === 403 || response.status === 401) { reload(); return; }
        if (response.status !== 204) throw Error();
        reload();
      } catch {
        button.disabled = false;
        notice("Couldn’t update the message. Refresh the inbox and try again.");
      }
    });
  }

  function init() {
    if (window.top !== window.self) return;
    const lock = document.getElementById("inbox-lock");
    if (lock && lock.dataset.bound !== "true") {
      lock.dataset.bound = "true";
      lock.disabled = false;
      const unlocked = lock.dataset.state === "unlocked";
      lock.addEventListener("click", () => unlocked ? lockInbox(lock) : unlockInbox(lock));
    }
    for (const form of document.querySelectorAll(".mark-form")) {
      if (form.dataset.bound === "true") continue;
      form.dataset.bound = "true";
      markMessage(form);
    }
  }

  // Only an open session can lapse, and it lapses back to the locked view.
  let checking = false;
  async function checkSession() {
    const lock = document.getElementById("inbox-lock");
    if (!lock || lock.dataset.state !== "unlocked" || document.hidden || checking) return;
    checking = true;
    try {
      if (!(await sessionRequest("session")).ok) reload();
    } catch {
      // Offline: keep showing what was already rendered.
    } finally {
      checking = false;
    }
  }

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) checkSession();
  });
  setInterval(checkSession, 15000);

  window.MessageInbox = { init };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
