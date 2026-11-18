"use strict";

const button = document.querySelector("#refresh");
const receipt = document.querySelector("#receipt");
const website = document.querySelector("#website");
const jetkvm = document.querySelector("#jetkvm");
const piMachine = document.querySelector("#pi-machine");
const localTime = new Intl.DateTimeFormat(undefined, {timeStyle: "medium"});

function setReading(element, label, state = "unknown") {
  element.lastElementChild.textContent = label;
  element.dataset.state = state;
}

function showSnapshot(snapshot) {
  piMachine.textContent = typeof snapshot.pi_machine === "string" && snapshot.pi_machine.length > 0
    ? snapshot.pi_machine : "the Pi";
  if (snapshot.website === "healthy") {
    setReading(website, "Healthy", "good");
  } else if (snapshot.website === "unavailable") {
    setReading(website, "Unavailable", "bad");
  } else {
    setReading(website, "Unknown");
  }
  if (snapshot.jetkvm === "reachable") {
    setReading(jetkvm, "Reachable", "good");
  } else if (snapshot.jetkvm === "unavailable") {
    setReading(jetkvm, "Unavailable", "bad");
  } else {
    setReading(jetkvm, "Unknown");
  }
}

async function refresh() {
  if (button.disabled) return;
  button.disabled = true;
  piMachine.textContent = "the Pi";
  receipt.textContent = "Loading the latest checks…";
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await fetch("/api/status", {
      cache: "no-store", mode: "same-origin", redirect: "error",
      headers: {Accept: "application/json"}, signal: controller.signal
    });
    if (response.status === 401 || response.status === 403) {
      showSnapshot({});
      receipt.textContent = "Access denied. Check your tailnet connection and permissions.";
      return;
    }
    if (!response.ok) throw new Error("Status request failed");
    const snapshot = await response.json();
    if (!snapshot || !Object.hasOwn(snapshot, "website") || !Object.hasOwn(snapshot, "jetkvm")) {
      throw new Error("Incomplete status response");
    }
    showSnapshot(snapshot);
    receipt.textContent = `Snapshot received at ${localTime.format(new Date())}. Checks run every 30 seconds.`;
  } catch {
    showSnapshot({});
    receipt.textContent = "Couldn’t load service status. Check your connection and refresh.";
  } finally {
    clearTimeout(timeout);
    button.disabled = false;
  }
}

button.hidden = false;
button.addEventListener("click", refresh);
window.addEventListener("pageshow", refresh);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) refresh();
});
