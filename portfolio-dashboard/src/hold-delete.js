"use strict";

if (window.top === window.self) {
  document.querySelectorAll(".remove-form").forEach(setupHoldConfirmation);
}

function setupHoldConfirmation(form) {
  const button = form.querySelector(".hold-delete");
  const label = button.querySelector("span");
  const help = form.querySelector(".hold-help");
  const field = form.elements.namedItem("hold");
  const holdMs = 5000;
  let generation = 0;
  let token = "";
  let target = "";
  let active = null;
  let timer;
  let suppressClick = false;

  function state(name, caption, instruction) {
    button.dataset.state = name;
    label.textContent = caption;
    help.textContent = instruction;
  }

  function idle(instruction = "") {
    field.disabled = true;
    field.value = "";
    button.disabled = false;
    state("idle", button.dataset.label, instruction);
  }

  function revoke(hold) {
    // The broker also expires unused tokens if navigation or loss of network
    // prevents delivery of this best-effort cancellation.
    fetch("/messages/auth/cancel", {
      method: "POST", credentials: "same-origin", keepalive: true,
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ hold }),
    }).catch(() => {});
  }

  function cancel() {
    if (["idle", "submitting"].includes(button.dataset.state)) return;
    generation++;
    clearTimeout(timer);
    active = null;
    if (token) revoke(token);
    token = "";
    idle("Canceled. Click to arm again.");
  }

  async function arm() {
    const attempt = ++generation;
    state("arming", "Arming…", "Preparing a confirmation for this selection.");
    try {
      target = form.dataset.id;
      let caption = "Hold to remove";
      if (form.dataset.operation === "delete-all") {
        const result = await fetch("/messages/delete-all", { cache: "no-store" });
        if (result.status === 401 || result.status === 403) { window.location.reload(); return; }
        if (!result.ok) throw Error();
        const snapshot = await result.json();
        if (!Number.isSafeInteger(snapshot.count) || snapshot.count < 1) throw Error();
        if (attempt !== generation) return;
        target = snapshot.id;
        caption = `Hold to remove all ${snapshot.count}`;
      }
      if (!/^[a-f0-9]{64}$/.test(target)) throw Error();
      const response = await fetch("/messages/auth/arm", {
        method: "POST", credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ operation: form.dataset.operation, id: target }),
      });
      if (response.status === 401 || response.status === 403) { window.location.reload(); return; }
      if (!response.ok) throw new Error();
      const { hold } = await response.json();
      if (typeof hold !== "string" || !/^[a-f0-9]{64}$/.test(hold)) throw new Error();
      if (attempt !== generation) {
        revoke(hold);
        return;
      }
      token = hold;
      state("armed", caption, "Hold for five seconds to permanently remove. Release early or press Escape to cancel.");
    } catch {
      if (attempt === generation) idle("Couldn’t arm this action. Reload the page to check your session and try again.");
    }
  }

  function tick() {
    if (!active) return;
    if (document.hidden || !document.hasFocus() || document.activeElement !== button) {
      cancel();
      return;
    }
    const remaining = holdMs - (performance.now() - active.start);
    if (remaining > 0) {
      label.textContent = `Hold… ${Math.ceil(remaining / 1000)} s`;
      timer = setTimeout(tick, Math.min(remaining, 250));
      return;
    }
    active = null;
    field.value = token;
    field.disabled = false;
    token = "";
    state("submitting", "Removing…", "Submitting your confirmed selection.");
    button.disabled = true;
    commit();
  }

  function start(input) {
    button.focus({ preventScroll: true });
    active = { ...input, start: performance.now() };
    suppressClick = true;
    state("holding", "Hold… 5 s", "Keep holding. Release early to cancel.");
    tick();
  }

  // The broker verifies the token, target and elapsed time again at commit.
  async function commit() {
    try {
      const response = await fetch("/messages/action/" + form.dataset.operation + "/" + target, {
        method: "POST", credentials: "same-origin", cache: "no-store",
        body: new URLSearchParams({ hold: field.value }),
      });
      if (response.status === 401 || response.status === 403) { window.location.reload(); return; }
      if (response.status === 409) { idle("The inbox changed. Review the messages and try again."); return; }
      if (response.status !== 204) throw Error();
      if (new URLSearchParams(location.search).has("before")) {
        const url = new URL(location.href);
        url.searchParams.delete("before");
        location.replace(url);
      } else { location.reload(); }
    } catch {
      idle("Couldn’t verify the result. Refresh the inbox before trying again.");
    }
  }

  form.addEventListener("submit", event => event.preventDefault());
  button.addEventListener("click", event => {
    event.preventDefault();
    if (!suppressClick && button.dataset.state === "idle") {
      button.focus({ preventScroll: true });
      arm();
    }
    suppressClick = false;
  });
  button.addEventListener("pointerdown", event => {
    if (!event.isPrimary || event.button !== 0) return;
    suppressClick = false;
    if (button.dataset.state !== "armed") return;
    event.preventDefault();
    button.setPointerCapture(event.pointerId);
    start({ pointer: event.pointerId });
  });
  button.addEventListener("pointermove", event => {
    if (active?.pointer !== event.pointerId) return;
    const rect = button.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX >= rect.right ||
        event.clientY < rect.top || event.clientY >= rect.bottom) cancel();
  });
  for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
    button.addEventListener(name, event => {
      if (active?.pointer === event.pointerId) cancel();
    });
  }
  button.addEventListener("contextmenu", event => event.preventDefault());
  button.addEventListener("keydown", event => {
    if (event.key !== " " && event.key !== "Enter") return;
    if (event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      event.preventDefault();
      return;
    }
    suppressClick = false;
    if (button.dataset.state !== "armed") return;
    event.preventDefault();
    start({ key: event.key });
  });
  button.addEventListener("keyup", event => {
    if (active?.key === event.key) {
      event.preventDefault();
      cancel();
    }
  });
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") cancel();
  });
  button.addEventListener("blur", cancel);
  window.addEventListener("blur", cancel);
  window.addEventListener("pagehide", cancel);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) cancel();
  });
  window.addEventListener("pageshow", event => {
    if (event.persisted) idle();
  });
  idle();
}
