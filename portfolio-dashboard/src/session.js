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
  let inFlight = null;
  let unlocking = false;

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
      const seconds = remaining();
      icon.textContent = "🔓";
      label.textContent = "Admin session";
      timer.textContent = expiresAt ? clock(seconds) : "";
      // The last minute turns amber, so a session never lapses mid-action unwarned.
      badge.toggleAttribute("data-warning", Boolean(expiresAt) && seconds <= 60);
      badge.disabled = false;
      badge.title = "Lock this session now";
      badge.setAttribute("aria-label",
        `Admin session${expiresAt ? `, ${clock(seconds)} left` : ""}. Activate to lock.`);
      return;
    }
    icon.textContent = state === "checking" ? "…" : "🔒";
    label.textContent = state === "checking" ? "Checking session…" : "Locked · read-only";
    timer.textContent = "";
    badge.toggleAttribute("data-warning", false);
    // A locked badge is also the way back in: one click asks the broker for a
    // passkey challenge, the same ceremony the Settings card runs.
    badge.disabled = state === "checking";
    if (state === "checking") {
      badge.removeAttribute("title");
    } else {
      badge.title = "Unlock with your passkey";
    }
    badge.setAttribute("aria-label", state === "checking"
      ? "Checking the passkey session"
      : "Passkey locked. Read-only. Activate to unlock with your passkey.");
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
  // the browser asks the broker instead of trusting its own timer alone. One
  // request at a time: every caller waits on the same answer.
  function refresh() {
    if (inFlight) return inFlight;
    inFlight = (async () => {
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
        inFlight = null;
      }
    })();
    return inFlight;
  }

  async function lock() {
    badge.disabled = true;
    try {
      await request("lock");
    } catch {
      // The countdown below still expires the badge on its own.
    }
    set("locked");
  }

  // Unlocking from the badge runs the Settings panel's ceremony, so both entry
  // points share one passkey flow and one status line.
  async function unlock() {
    if (unlocking) return;
    const signIn = window.PasskeyPanel?.unlock;
    if (typeof signIn !== "function") return;
    unlocking = true;
    badge.disabled = true;
    icon.textContent = "🔓";
    label.textContent = "Unlocking…";
    timer.textContent = "";
    badge.toggleAttribute("data-warning", false);
    badge.setAttribute("aria-label", "Waiting for the passkey prompt");
    let result = null;
    try {
      result = await signIn();
    } catch {
      // A thrown ceremony reads the same as a refused one below.
    } finally {
      unlocking = false;
    }
    if (result?.ok) {
      await refresh();
      if (state !== "unlocked") set("locked");
      return;
    }
    set("locked");
    badge.title = result?.message || "Couldn’t unlock with the passkey. Try again.";
  }

  badge.addEventListener("click", () => {
    if (state === "unlocked") lock();
    else if (state === "locked") unlock();
  });

  setInterval(() => {
    if (state !== "unlocked" || !expiresAt) return;
    const seconds = remaining();
    if (seconds > 0) {
      timer.textContent = clock(seconds);
      badge.toggleAttribute("data-warning", seconds <= 60);
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
