"use strict";

(() => {
  // The archive is readable without a passkey. Unlocking here starts the same
  // owner session the header badge reports and counts down.
  const sessionRequest = (step, body) => fetch("/messages/auth/" + step, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });

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
      window.SessionBadge?.refresh?.();
      reload();
    } catch (error) {
      button.disabled = false;
      notice(error.name === "NotAllowedError"
        ? "The passkey request was cancelled or timed out."
        : (error.message || "Couldn’t unlock the inbox."));
    }
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
    const unlock = document.getElementById("inbox-lock");
    if (unlock && unlock.dataset.bound !== "true") {
      unlock.dataset.bound = "true";
      unlock.disabled = false;
      unlock.addEventListener("click", () => unlockInbox(unlock));
    }
    for (const form of document.querySelectorAll(".mark-form")) {
      if (form.dataset.bound === "true") continue;
      form.dataset.bound = "true";
      markMessage(form);
    }
  }

  // The badge is authoritative: locking (or expiry) elsewhere hides the
  // controls, and unlocking from the header reveals them without a reload.
  window.addEventListener("session:unlocked", reload);
  window.addEventListener("session:locked", reload);

  window.MessageInbox = { init };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
