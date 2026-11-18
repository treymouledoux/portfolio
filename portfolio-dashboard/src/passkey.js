"use strict";

(() => {
  const panel = document.getElementById("passkey-panel");
  if (!panel) return;
  const login = document.getElementById("passkey-sign-in");
  const enroll = document.getElementById("passkey-enroll");
  const status = document.getElementById("passkey-status");
  const form = document.getElementById("delete-form");

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
      if (problem.error === "closed") throw Error("Passkey enrollment is closed. Ask the Pi administrator to open a new enrollment window.");
      if (problem.error === "unregistered") throw Error("You haven’t registered a passkey yet. Use Set up a passkey.");
      throw Error("Authorization failed. Try again or reopen the private confirmation page.");
    }
    return response.json();
  }

  function authenticated() {
    login.hidden = true;
    enroll.hidden = true;
    status.textContent = "Passkey verified.";
    if (form) form.hidden = false;
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
      status.textContent = error.name === "NotAllowedError"
        ? "The passkey request was cancelled or timed out."
        : (error.message || "Passkey verification failed.");
    } finally {
      login.disabled = false;
      enroll.disabled = false;
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
  post("session").then(authenticated).catch(() => {
    login.disabled = false;
    enroll.disabled = false;
    status.textContent = "Sign in with your dashboard passkey.";
  });
})();
