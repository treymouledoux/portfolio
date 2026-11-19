"use strict";

(() => {
  const sessionRequest = step => fetch("/messages/auth/" + step, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: "{}",
  });

  // A locked, expired or denied inbox is a server fragment now, so the view is
  // replaced in place instead of reloading the document.
  function reload() {
    if (window.Dashboard) window.Dashboard.refreshMessages();
    else window.location.reload();
  }

  function notice(text) {
    const target = document.getElementById("inbox-notice");
    if (target) target.textContent = text;
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
      lock.addEventListener("click", () => lockInbox(lock));
    }
    for (const form of document.querySelectorAll(".mark-form")) {
      if (form.dataset.bound === "true") continue;
      form.dataset.bound = "true";
      markMessage(form);
    }
  }

  // The inbox is only readable while the passkey session lasts. Hide the
  // rendered archive whenever the tab is away and restore it after a check.
  function concealForBackground() {
    const inboxView = document.getElementById("inbox-view");
    if (inboxView) inboxView.hidden = true;
  }

  let checking = false;
  async function checkSession() {
    if (document.hidden || checking || !document.getElementById("inbox-lock")) return;
    checking = true;
    try {
      if (!(await sessionRequest("session")).ok) { reload(); return; }
      const inboxView = document.getElementById("inbox-view");
      if (inboxView && !document.hidden) inboxView.hidden = false;
    } catch {
      reload();
    } finally {
      checking = false;
    }
  }

  document.addEventListener("visibilitychange", () => {
    if (document.hidden) concealForBackground();
    else checkSession();
  });
  window.addEventListener("pagehide", concealForBackground);
  setInterval(checkSession, 15000);

  window.MessageInbox = { init };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
