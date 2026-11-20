"use strict";

(() => {
  // Another control on the page may sign in on its own; init() then refreshes
  // this panel's state instead of binding a second time.
  let reread = () => {};
  // The header badge can start the same sign-in; until a supported browser
  // binds the panel this reports why nothing happened.
  let unlockWithPasskey = async () => ({
    ok: false,
    message: "Open this private page in a current browser with passkey support.",
  });

  // The panel lives in Settings: it reports the session, signs in, locks and
  // enrolls. Enrollment stays closed until a root operator opens a window.
  function init() {
    const panel = document.getElementById("passkey-panel");
    if (!panel || panel.dataset.bound === "true") return;
    panel.dataset.bound = "true";
    const signIn = document.getElementById("passkey-sign-in");
    const signOut = document.getElementById("passkey-lock");
    const enroll = document.getElementById("passkey-enroll");
    const status = document.getElementById("passkey-status");
    // pending: the check has not answered yet; unknown: it failed. Only an
    // explicit "closed" hides setup, so a broker outage never blocks the one
    // path to first-time enrollment. The broker still refuses a real attempt.
    let enrollment = "pending";

    function syncEnroll() {
      enroll.disabled = enrollment === "pending" || enrollment === "closed";
      enroll.toggleAttribute("data-closed", enrollment === "closed");
      if (enrollment === "closed") {
        enroll.title = "Setting up a passkey needs an enrollment window opened on the Pi.";
      } else {
        enroll.removeAttribute("title");
      }
    }

    const request = step => fetch("/messages/auth/" + step, {
      method: "POST",
      credentials: "same-origin",
      cache: "no-store",
      headers: { "Content-Type": "application/json" },
      body: "{}",
    });

    async function post(endpoint, body = {}) {
      const response = await fetch("/messages/auth/" + endpoint, {
        method: "POST",
        credentials: "same-origin",
        cache: "no-store",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
      });
      if (!response.ok) {
        const problem = await response.json().catch(() => ({}));
        const error = Error(problem.error === "closed"
          ? "Passkey enrollment is closed. Ask the Pi administrator to open a new enrollment window."
          : problem.error === "unregistered"
            ? "You haven’t registered a passkey yet. Use Set up a passkey."
            : "Passkey verification failed. Please try again.");
        error.code = problem.error;
        throw error;
      }
      return response.json();
    }

    function showLocked(text) {
      signIn.hidden = false;
      signIn.disabled = false;
      signOut.hidden = true;
      status.textContent = text ?? "Locked. Unlock to change settings, remove messages or open a terminal.";
    }

    function showUnlocked() {
      signIn.hidden = true;
      signOut.hidden = false;
      signOut.disabled = false;
      // The header badge carries the countdown; this card just reports access.
      status.textContent = "Signed in for this browser.";
    }

    async function verify(register) {
      signIn.disabled = true;
      enroll.disabled = true;
      try {
        const kind = register ? "enroll" : "login";
        const { flow, publicKey } = await post(kind + "-options");
        const credential = register
          ? await navigator.credentials.create({ publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(publicKey) })
          : await navigator.credentials.get({ publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(publicKey) });
        if (!credential) throw Error("The passkey request was cancelled.");
        await post(kind + "-finish", { flow, credential: credential.toJSON() });
        if (register) {
          enroll.hidden = true;
          status.textContent = "Passkey registered. Sign in with it to verify it works.";
          signIn.focus();
        } else {
          showUnlocked();
          window.SessionBadge?.refresh?.();
        }
        return { ok: true };
      } catch (error) {
        if (error.code === "closed") enrollment = "closed";
        status.textContent = error.name === "NotAllowedError"
          ? "The passkey request was cancelled or timed out."
          : (error.message || "Passkey verification failed.");
        return { ok: false, message: status.textContent };
      } finally {
        signIn.disabled = false;
        syncEnroll();
      }
    }

    async function lockSession() {
      signOut.disabled = true;
      try {
        const response = await request("lock");
        if (!response.ok && response.status !== 401 && response.status !== 403) throw Error();
        showLocked("Session locked. Owner actions need the passkey again.");
        window.SessionBadge?.refresh?.();
      } catch {
        signOut.disabled = false;
        status.textContent = "Couldn’t lock the session. Check your connection and try again.";
      }
    }

    // Re-read the session after another control signed in on its own.
    function refresh() {
      request("session")
        .then(response => { if (response.ok) showUnlocked(); else showLocked(); })
        .catch(() => showLocked("Couldn’t check the passkey session. Check your connection and try again."));
    }

    if (window.top !== window.self || !window.isSecureContext ||
        !window.PublicKeyCredential?.parseCreationOptionsFromJSON ||
        !window.PublicKeyCredential?.parseRequestOptionsFromJSON ||
        !window.PublicKeyCredential.prototype.toJSON) {
      status.textContent = "Open this private page in a current browser with passkey support.";
      return;
    }
    signIn.addEventListener("click", () => verify(false));
    signOut.addEventListener("click", lockSession);
    enroll.addEventListener("click", () => verify(true));
    reread = refresh;
    unlockWithPasskey = () => verify(false);
    refresh();
    // The broker only accepts a first registration while a root operator has an
    // enrollment window open, so setup stays unavailable until it reports one.
    syncEnroll();
    post("enrollment")
      .then(({ open }) => {
        enrollment = open === true ? "open" : "closed";
        syncEnroll();
      })
      .catch(() => {
        enrollment = "unknown";
        syncEnroll();
      });
  }

  window.PasskeyPanel = { init, refresh: () => reread(), unlock: () => unlockWithPasskey() };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init);
  else init();
})();
