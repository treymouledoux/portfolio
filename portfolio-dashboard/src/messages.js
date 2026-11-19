"use strict";

(() => {
  // The archive is readable without a passkey. Unlocking here starts the same
  // admin session the header badge reports and counts down.
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

  // The server writes each stamp in a readable UTC form; here it becomes the
  // reader's own clock, with today's messages named as today.
  const dayAndTime = new Intl.DateTimeFormat(undefined, {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
  });
  const dayAndYear = new Intl.DateTimeFormat(undefined, {
    month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit",
  });
  const clock = new Intl.DateTimeFormat(undefined, {hour: "numeric", minute: "2-digit"});
  const exact = new Intl.DateTimeFormat(undefined, {dateStyle: "full", timeStyle: "medium"});

  // Browsers only promise three fractional digits, and the writer can emit
  // more, so the extra precision is dropped rather than risking an invalid date.
  const parse = stored => new Date(stored.replace(/(\.\d{3})\d+/, "$1"));

  function humanize(stored, now) {
    const when = parse(stored);
    if (Number.isNaN(when.valueOf())) return stored;
    if (when.toDateString() === now.toDateString()) return `Today at ${clock.format(when)}`;
    return (when.getFullYear() === now.getFullYear() ? dayAndTime : dayAndYear).format(when);
  }

  function humanizeTimes() {
    const now = new Date();
    for (const node of document.querySelectorAll("#view-messages time[datetime]")) {
      const stored = node.getAttribute("datetime");
      node.textContent = humanize(stored, now);
      const when = parse(stored);
      if (!Number.isNaN(when.valueOf())) node.title = exact.format(when);
    }
  }

  function init() {
    if (window.top !== window.self) return;
    humanizeTimes();
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

  // The badge is authoritative: locking (or expiry) hides the controls and
  // unlocking reveals them without a reload. Only the view on screen refreshes
  // itself, so unlocking on Overview or Settings never navigates here.
  function viewingMessages() {
    const view = document.getElementById("view-messages");
    return Boolean(view && !view.hidden);
  }
  window.addEventListener("session:unlocked", () => { if (viewingMessages()) reload(); });
  window.addEventListener("session:locked", () => { if (viewingMessages()) reload(); });

  window.MessageInbox = { init };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
