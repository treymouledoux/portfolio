"use strict";

(() => {
  const lock = document.getElementById("inbox-lock");
  if (!lock || window.top !== window.self) return;
  const inbox = document.getElementById("inbox");
  const notice = document.getElementById("inbox-notice");

  const sessionRequest = step => fetch("/messages/auth/" + step, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: "{}",
  });

  function conceal() {
    inbox.hidden = true;
    inbox.replaceChildren();
    window.location.reload();
  }

  lock.disabled = false;
  lock.addEventListener("click", async () => {
    lock.disabled = true;
    try {
      const response = await sessionRequest("lock");
      if (!response.ok && response.status !== 401 && response.status !== 403) throw Error();
      conceal();
    } catch {
      lock.disabled = false;
      notice.textContent = "Couldn’t lock the inbox. Check your connection and try again.";
    }
  });

  for (const form of document.querySelectorAll(".mark-form")) {
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
        if (response.status === 403 || response.status === 401) { conceal(); return; }
        if (response.status !== 204) throw Error();
        location.reload();
      } catch {
        button.disabled = false;
        notice.textContent = "Couldn’t update the message. Refresh the inbox and try again.";
      }
    });
  }

  let checking = false;
  async function checkSession() {
    if (document.hidden || checking) return;
    checking = true;
    try {
      if (!(await sessionRequest("session")).ok) { conceal(); return; }
      if (!document.hidden) inbox.hidden = false;
    } catch {
      conceal();
    } finally {
      checking = false;
    }
  }
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) inbox.hidden = true;
    else checkSession();
  });
  window.addEventListener("pagehide", () => { inbox.hidden = true; });
  window.addEventListener("pageshow", event => {
    if (event.persisted) checkSession();
  });
  setInterval(checkSession, 15000);
})();
