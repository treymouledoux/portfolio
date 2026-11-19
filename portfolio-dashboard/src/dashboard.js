"use strict";

const button = document.querySelector("#refresh");
const receipt = document.querySelector("#receipt");
const website = document.querySelector("#website");
const jetkvm = document.querySelector("#jetkvm");
const piMachine = document.querySelector("#pi-machine");
const sshCommand = document.querySelector("#ssh-command");
const localTime = new Intl.DateTimeFormat(undefined, {timeStyle: "medium"});

// One sample per poll, kept in the page and drawn as a line the panel swaps
// between. Ping gaps stay gaps instead of becoming zero.
const SERIES = {
  ping: {label: "Ping", unit: "ms", read: snapshot => snapshot.ping_probe?.latency_ms},
  cpu: {label: "CPU", unit: "%", read: snapshot => snapshot.host?.cpu_percent},
  ram: {label: "RAM used", unit: "MiB", read: snapshot => snapshot.host?.memory_used_mib},
};
const HISTORY_SIZE = 60;
const HISTORY_STORE = "dashboard-history";
const HISTORY_MAX_AGE = 30 * 60 * 1000;
const plotted = {ping: [], cpu: [], ram: []};
const chart = {
  line: document.getElementById("chart-line"),
  dot: document.getElementById("chart-dot"),
  title: document.getElementById("chart-title"),
  summary: document.getElementById("chart-summary"),
  span: document.getElementById("chart-span"),
};
let series = "ping";

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
    recordChart(snapshot);
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

// A reload keeps this tab's recent line; nothing leaves the browser.
function restoreChart() {
  try {
    const saved = JSON.parse(sessionStorage.getItem(HISTORY_STORE));
    const cutoff = Date.now() - HISTORY_MAX_AGE;
    for (const [name, samples] of Object.entries(plotted)) {
      if (!Array.isArray(saved?.[name])) continue;
      plotted[name] = saved[name]
        .filter(sample => Number.isFinite(sample?.at) && sample.at >= cutoff)
        .slice(-HISTORY_SIZE)
        .map(sample => ({
          at: sample.at,
          value: Number.isFinite(sample.value) ? sample.value : null,
        }));
    }
  } catch {}
}

function saveChart() {
  try {
    sessionStorage.setItem(HISTORY_STORE, JSON.stringify(plotted));
  } catch {}
}

for (const control of document.querySelectorAll(".chart-tabs button")) {
  control.hidden = false;
  control.addEventListener("click", () => {
    series = control.dataset.series;
    for (const other of document.querySelectorAll(".chart-tabs button")) {
      other.setAttribute("aria-pressed", String(other === control));
    }
    drawChart();
  });
}
restoreChart();
drawChart();

function formatValue(spec, value) {
  return spec.unit === "%" ? `${value.toFixed(1)}%` : `${Math.round(value)} ${spec.unit}`;
}

function recordChart(snapshot) {
  const at = Date.now();
  for (const [name, spec] of Object.entries(SERIES)) {
    const raw = spec.read(snapshot);
    const samples = plotted[name];
    // A refresh right after a poll is the same check, not a new sample.
    if (samples.length && at - samples.at(-1).at < 5000) samples.pop();
    samples.push({at, value: typeof raw === "number" && Number.isFinite(raw) ? raw : null});
    if (samples.length > HISTORY_SIZE) samples.shift();
  }
  saveChart();
  drawChart();
}

function drawChart() {
  const spec = SERIES[series];
  const samples = plotted[series];
  chart.title.textContent = `${spec.label} over time`;
  const known = samples.filter(sample => Number.isFinite(sample.value));
  if (!known.length) {
    chart.line.setAttribute("d", "");
    chart.dot.setAttribute("hidden", "");
    chart.summary.textContent = samples.length
      ? `No ${spec.label.toLowerCase()} readings in this window.`
      : "Waiting for the first sample…";
    chart.span.textContent = "";
    return;
  }
  const values = known.map(sample => sample.value);
  const low = Math.min(...values);
  const high = Math.max(...values);
  const room = high - low || Math.max(Math.abs(high) * 0.2, 1);
  const floor = low - room * 0.15;
  const ceiling = high + room * 0.15;
  const x = index => (samples.length < 2 ? 640 : (index / (samples.length - 1)) * 640);
  const y = value => 190 - ((value - floor) / (ceiling - floor)) * 180;
  let path = "";
  let drawing = false;
  samples.forEach((sample, index) => {
    if (!Number.isFinite(sample.value)) { drawing = false; return; }
    path += `${drawing ? "L" : "M"}${x(index).toFixed(1)} ${y(sample.value).toFixed(1)}`;
    drawing = true;
  });
  chart.line.setAttribute("d", path);
  const latest = known.at(-1);
  chart.dot.removeAttribute("hidden");
  chart.dot.setAttribute("cx", x(samples.lastIndexOf(latest)).toFixed(1));
  chart.dot.setAttribute("cy", y(latest.value).toFixed(1));
  const average = values.reduce((sum, value) => sum + value, 0) / values.length;
  chart.summary.textContent =
    `Now ${formatValue(spec, latest.value)} · avg ${formatValue(spec, average)} · low ${formatValue(spec, low)} · high ${formatValue(spec, high)}`;
  const minutes = (samples.at(-1).at - samples[0].at) / 60000;
  chart.span.textContent = samples.length < 2
    ? "One sample so far; a new one every 30 s."
    : `${samples.length} samples over the last ${minutes < 1 ? "minute" : `${Math.round(minutes)} min`}.`;
}
