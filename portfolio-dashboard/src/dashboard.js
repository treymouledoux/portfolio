"use strict";

const button = document.querySelector("#refresh");
const receipt = document.querySelector("#receipt");
const website = document.querySelector("#website");
const jetkvm = document.querySelector("#jetkvm");
const piMachine = document.querySelector("#pi-machine");
const sshCommand = document.querySelector("#ssh-command");
const localTime = new Intl.DateTimeFormat(undefined, {timeStyle: "medium"});

function setReading(element, label, state = "unknown") {
  element.lastElementChild.textContent = label;
  element.dataset.state = state;
}

function showSnapshot(snapshot) {
  piMachine.textContent = typeof snapshot.pi_machine === "string" && snapshot.pi_machine.length > 0
    ? snapshot.pi_machine : "the Pi";
  const configured = Boolean(snapshot.pi_machine && snapshot.pi_ssh_user);
  sshCommand.textContent = configured ? `ssh ${snapshot.pi_ssh_user}@${snapshot.pi_machine}` : "";
  sshCommand.hidden = !configured;
  for (const [id, probe] of [["web-ms", snapshot.website_probe], ["tcp-ms", snapshot.jetkvm_probe], ["ping-ms", snapshot.ping_probe]]) {
    document.getElementById(id).textContent = probe?.latency_ms === 0 ? "<1 ms" : Number.isFinite(probe?.latency_ms) ? `${probe.latency_ms} ms` : probe ? "No response" : "Awaiting check";
  }
  const online = snapshot.muse?.online;
  setReading(document.getElementById("muse"), online === true ? "Connected" : online === false ? "Offline" : "Unknown",
    online === true ? "good" : online === false ? "bad" : "unknown");
  const h = snapshot.host || {};
  document.getElementById("cpu").textContent = Number.isFinite(h.cpu_percent) ? `${h.cpu_percent.toFixed(1)}%` : "—";
  document.getElementById("ram").textContent = Number.isFinite(h.memory_used_mib) ? `${h.memory_used_mib} / ${h.memory_total_mib} MiB` : "—";
  const t = h.uptime_seconds;
  document.getElementById("uptime").textContent = Number.isFinite(t) ? `${Math.floor(t / 86400)}d ${Math.floor(t / 3600) % 24}h ${Math.floor(t / 60) % 60}m` : "—";
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

async function refresh(target) {
  if (button.disabled) return;
  button.disabled = true;
  receipt.textContent = "Loading the latest checks…";
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  try {
    let notice = "";
    if (typeof target === "string") {
      const checked = await fetch("/api/check/" + target, {
        method: "POST", cache:"no-store", mode:"same-origin", redirect:"error", signal:controller.signal
      });
      if (!checked.ok) throw new Error("Check failed");
      notice = (await checked.text()) + ". ";
    }
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
    const stamp = snapshot.website_probe?.checked_at;
    receipt.textContent = notice + (stamp ? `Website checked at ${localTime.format(new Date(stamp * 1000))}. ` : "") + "Checks run every 30 seconds.";
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

for (const control of document.querySelectorAll("button[data-test]")) {
  control.hidden = false;
  control.addEventListener("click", () => refresh(control.dataset.test));
}
setInterval(() => { if (!document.hidden) refresh(); }, 30000);
