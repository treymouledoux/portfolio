"use strict";

(() => {
  const panel = document.getElementById("passkey-panel");
  if (!panel) return;
  const login = document.getElementById("passkey-sign-in");
  const enroll = document.getElementById("passkey-enroll");
  const status = document.getElementById("passkey-status");
  const locked = document.getElementById("inbox-locked");
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

  function authenticated() {
    login.hidden = true;
    enroll.hidden = true;
    status.textContent = "Passkey verified.";
    if (locked) window.location.reload();
  }

  async function verify(register) {
    login.disabled = true;
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
        login.focus();
      } else {
        authenticated();
      }
    } catch (error) {
      if (error.code === "closed") enrollment = "closed";
      status.textContent = error.name === "NotAllowedError"
        ? "The passkey request was cancelled or timed out."
        : (error.message || "Passkey verification failed.");
    } finally {
      login.disabled = false;
      syncEnroll();
    }
  }

  if (window.top !== window.self || !window.isSecureContext ||
      !window.PublicKeyCredential?.parseCreationOptionsFromJSON ||
      !window.PublicKeyCredential?.parseRequestOptionsFromJSON ||
      !window.PublicKeyCredential.prototype.toJSON) {
    status.textContent = "Open this private page in a current browser with passkey support.";
    return;
  }
  login.addEventListener("click", () => verify(false));
  enroll.addEventListener("click", () => verify(true));
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
  if (locked) {
    login.disabled = false;
    return;
  }
  post("session").then(authenticated).catch(() => {
    login.disabled = false;
    status.textContent = "Sign in with your dashboard passkey.";
  });
})();
