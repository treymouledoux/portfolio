"use strict";

(() => {
  // The header badge is the one place that reports whether this browser holds
  // the admin session, how long is left, and how to end it.
  const badge = document.getElementById("session-badge");
  if (!badge) return;
  const icon = badge.querySelector(".session-icon");
  const label = badge.querySelector(".session-label");
  const timer = badge.querySelector(".session-timer");
  let state = "checking";
  let expiresAt = 0;
  let checking = false;

  const request = step => fetch("/messages/auth/" + step, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    headers: { "Content-Type": "application/json" },
    body: "{}",
  });

  const remaining = () => Math.max(0, Math.round((expiresAt - Date.now()) / 1000));

  function clock(seconds) {
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  }

  function paint() {
    badge.dataset.state = state;
    if (state === "unlocked") {
      icon.textContent = "🔓";
      label.textContent = "Admin session";
      timer.textContent = expiresAt ? clock(remaining()) : "";
      badge.disabled = false;
      badge.title = "Lock this session now";
      badge.setAttribute("aria-label",
        `Admin session${expiresAt ? `, ${clock(remaining())} left` : ""}. Activate to lock.`);
      return;
    }
    icon.textContent = state === "checking" ? "…" : "🔒";
    label.textContent = state === "checking" ? "Checking session…" : "Locked · read-only";
    timer.textContent = "";
    badge.disabled = true;
    badge.removeAttribute("title");
    badge.setAttribute("aria-label",
      state === "checking" ? "Checking the passkey session" : "Passkey locked. Read-only.");
  }

  // Only a state change is announced; the countdown itself stays visual.
  function set(next, seconds = 0) {
    const previous = state;
    state = next;
    // A broker that reports no lifetime leaves the countdown hidden rather
    // than expiring a session the server would still accept.
    expiresAt = next === "unlocked" && seconds > 0 ? Date.now() + seconds * 1000 : 0;
    paint();
    // The first answer settles the badge; it is not a session change, but the
    // other panels still need it to know which controls to offer.
    if (previous === "checking") {
      window.dispatchEvent(new Event("session:settled"));
      return;
    }
    if (previous === state) return;
    window.dispatchEvent(new Event(state === "unlocked" ? "session:unlocked" : "session:locked"));
  }

  // Another tab, the Settings card or a host action can change the session, so
  // the browser asks the broker instead of trusting its own timer alone.
  async function refresh() {
    if (checking) return;
    checking = true;
    try {
      const response = await request("session");
      if (!response.ok) {
        set("locked");
        return;
      }
      const body = await response.json().catch(() => ({}));
      const seconds = Number(body.expires_in);
      set("unlocked", Number.isFinite(seconds) ? seconds : 0);
    } catch {
      // Offline keeps whatever was last known instead of inventing a state.
      if (state === "checking") set("locked");
    } finally {
      checking = false;
    }
  }

  badge.addEventListener("click", async () => {
    if (state !== "unlocked") return;
    badge.disabled = true;
    try {
      await request("lock");
    } catch {
      // The countdown below still expires the badge on its own.
    }
    set("locked");
  });

  setInterval(() => {
    if (state !== "unlocked" || !expiresAt) return;
    if (remaining() > 0) {
      timer.textContent = clock(remaining());
      return;
    }
    set("locked");
  }, 1000);
  setInterval(refresh, 30000);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) refresh();
  });

  window.SessionBadge = { refresh, current: () => state };
  paint();
  refresh();
})();
