"use strict";

const button = document.querySelector("#refresh");
const receipt = document.querySelector("#receipt");
const website = document.querySelector("#website");
const jetkvm = document.querySelector("#jetkvm");
const piMachine = document.querySelector("#pi-machine");
const sshCommand = document.querySelector("#ssh-command");
const localTime = new Intl.DateTimeFormat(undefined, {timeStyle: "medium"});

// One sample per poll. Ping keeps every device separately so the panel can
// overlay any subset of them; CPU and RAM are single lines over the same
// samples. Missing readings stay gaps instead of becoming zero.
const DEVICES = {
  jetkvm: {label: "JetKVM", read: snapshot => snapshot.ping_probe?.latency_ms},
  muse: {label: "Muse", read: snapshot => snapshot.muse_ping_probe?.latency_ms},
  portfolio: {label: "Portfolio (WAN)", read: snapshot => snapshot.website_probe?.latency_ms},
};
const SERIES = {
  cpu: {label: "CPU", unit: "%", read: snapshot => snapshot.host?.cpu_percent},
  ram: {label: "RAM used", unit: "MiB", read: snapshot => snapshot.host?.memory_used_mib},
};
const UNIT = {jetkvm: "ms", muse: "ms", portfolio: "ms", cpu: "%", ram: "MiB"};
const HISTORY_SIZE = 60;
const HISTORY_STORE = "dashboard-history";
const HISTORY_MAX_AGE = 30 * 60 * 1000;
const SVG = "http://www.w3.org/2000/svg";
const plotted = {jetkvm: [], muse: [], portfolio: [], cpu: [], ram: []};
const chosen = new Set(Object.keys(DEVICES));
const chart = {
  series: document.getElementById("chart-series"),
  title: document.getElementById("chart-title"),
  summary: document.getElementById("chart-summary"),
  span: document.getElementById("chart-span"),
  pills: document.getElementById("chart-pills"),
  menu: document.getElementById("device-menu"),
  count: document.getElementById("device-count"),
};
const viewButtons = [...document.querySelectorAll(".chart-tabs button")];
const deviceBoxes = [...document.querySelectorAll(".device-list input[type=checkbox]")];
const deviceToggle = document.getElementById("device-toggle");
let view = "ping";

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
  for (const [id, probe] of [["web-ms", snapshot.website_probe], ["tcp-ms", snapshot.jetkvm_probe], ["ping-ms", snapshot.ping_probe], ["muse-ms", snapshot.muse_ping_probe]]) {
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

for (const control of viewButtons) {
  control.hidden = false;
  control.addEventListener("click", () => {
    view = control.dataset.view;
    for (const other of viewButtons) other.setAttribute("aria-pressed", String(other === control));
    syncMenu();
    saveChart();
    drawChart();
  });
}

for (const box of deviceBoxes) {
  box.addEventListener("change", () => {
    if (box.checked) chosen.add(box.value);
    else chosen.delete(box.value);
    syncMenu();
    saveChart();
    drawChart();
  });
}
deviceToggle.hidden = false;
deviceToggle.addEventListener("click", () => {
  const every = chosen.size === Object.keys(DEVICES).length;
  chosen.clear();
  if (!every) for (const key of Object.keys(DEVICES)) chosen.add(key);
  syncMenu();
  saveChart();
  drawChart();
});

restoreChart();
syncMenu();
drawChart();

function syncMenu() {
  chart.menu.hidden = view !== "ping";
  const total = Object.keys(DEVICES).length;
  chart.count.textContent = `${chosen.size} / ${total}`;
  deviceToggle.textContent = chosen.size === total ? "Select none" : "Select all";
  for (const box of deviceBoxes) box.checked = chosen.has(box.value);
}

function formatValue(unit, value) {
  return unit === "%" ? `${value.toFixed(1)}%` : `${Math.round(value)} ${unit}`;
}

function svgNode(name, attributes) {
  const node = document.createElementNS(SVG, name);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  return node;
}

function recordChart(snapshot) {
  const at = Date.now();
  for (const [key, device] of Object.entries(DEVICES)) remember(key, device.read(snapshot), at);
  for (const [key, series] of Object.entries(SERIES)) remember(key, series.read(snapshot), at);
  saveChart();
  drawChart();
}

function remember(key, raw, at) {
  const samples = plotted[key];
  // A refresh right after a poll is the same check, not a new sample.
  if (samples.length && at - samples.at(-1).at < 5000) samples.pop();
  samples.push({at, value: typeof raw === "number" && Number.isFinite(raw) ? raw : null});
  if (samples.length > HISTORY_SIZE) samples.shift();
}

// A reload keeps this tab's recent lines; nothing leaves the browser.
function restoreChart() {
  try {
    const saved = JSON.parse(sessionStorage.getItem(HISTORY_STORE));
    const cutoff = Date.now() - HISTORY_MAX_AGE;
    for (const key of Object.keys(plotted)) {
      const samples = saved?.samples?.[key];
      if (!Array.isArray(samples)) continue;
      plotted[key] = samples
        .filter(sample => Number.isFinite(sample?.at) && sample.at >= cutoff)
        .slice(-HISTORY_SIZE)
        .map(sample => ({
          at: sample.at,
          value: Number.isFinite(sample.value) ? sample.value : null,
        }));
    }
    const devices = Array.isArray(saved?.devices) ? saved.devices.filter(key => key in DEVICES) : [];
    if (devices.length) {
      chosen.clear();
      for (const key of devices) chosen.add(key);
    }
    if (saved?.view === "ping" || saved?.view === "cpu" || saved?.view === "ram") view = saved.view;
    for (const control of viewButtons) control.setAttribute("aria-pressed", String(control.dataset.view === view));
  } catch {}
}

function saveChart() {
  try {
    sessionStorage.setItem(HISTORY_STORE, JSON.stringify({samples: plotted, devices: [...chosen], view}));
  } catch {}
}

function drawChart() {
  const lines = view === "ping"
    ? Object.keys(DEVICES).filter(key => chosen.has(key)).map(key => ({key, label: DEVICES[key].label}))
    : [{key: view, label: SERIES[view].label}];
  chart.series.replaceChildren();
  chart.pills.replaceChildren();
  chart.title.textContent = view === "ping" ? "Ping comparison" : `${SERIES[view].label} over time`;
  if (!lines.length) {
    chart.summary.textContent = "No devices selected. Pick at least one under Devices.";
    chart.span.textContent = "";
    return;
  }
  const count = Math.max(0, ...lines.map(line => plotted[line.key].length));
  const values = lines.flatMap(line => plotted[line.key]
    .map(entry => entry.value)
    .filter(value => Number.isFinite(value)));
  if (!values.length) {
    chart.summary.textContent = "Waiting for the first sample…";
    chart.span.textContent = "";
    drawPills(lines);
    return;
  }
  const low = Math.min(...values);
  const high = Math.max(...values);
  const room = high - low || Math.max(Math.abs(high) * 0.2, 1);
  const floor = view === "ping" ? 0 : low - room * 0.15;
  const ceiling = view === "ping" ? Math.max(high * 1.15, 5) : high + room * 0.15;
  const x = index => (count < 2 ? 640 : (index / (count - 1)) * 640);
  const y = value => 190 - ((value - floor) / (ceiling - floor)) * 180;
  for (const line of lines) {
    const samples = plotted[line.key];
    let path = "";
    let drawing = false;
    for (const [index, entry] of samples.entries()) {
      if (!Number.isFinite(entry.value)) { drawing = false; continue; }
      path += `${drawing ? "L" : "M"}${x(index).toFixed(1)} ${y(entry.value).toFixed(1)}`;
      drawing = true;
    }
    if (!path) continue;
    chart.series.append(svgNode("path", {
      d: path,
      class: `chart-line series-${line.key}`,
      "vector-effect": "non-scaling-stroke",
    }));
    const latest = samples.reduce((found, entry, index) => (Number.isFinite(entry.value) ? index : found), -1);
    if (latest >= 0) {
      chart.series.append(svgNode("circle", {
        cx: x(latest).toFixed(1),
        cy: y(samples[latest].value).toFixed(1),
        r: "3.5",
        class: `chart-dot series-${line.key}`,
        "vector-effect": "non-scaling-stroke",
      }));
    }
  }
  drawPills(lines);
  // The latest readings of the chosen view, not of every device.
  const latestValues = lines
    .map(line => plotted[line.key].filter(entry => Number.isFinite(entry.value)).at(-1)?.value)
    .filter(value => Number.isFinite(value));
  const average = values.reduce((sum, value) => sum + value, 0) / values.length;
  if (view === "ping") {
    const unit = "ms";
    chart.summary.textContent =
      `${lines.length} device${lines.length === 1 ? "" : "s"} · range ${formatValue(unit, low)}–${formatValue(unit, high)}`;
  } else {
    const unit = UNIT[view];
    chart.summary.textContent =
      `Now ${formatValue(unit, latestValues[0] ?? average)} · avg ${formatValue(unit, average)} · low ${formatValue(unit, low)} · high ${formatValue(unit, high)}`;
  }
  const times = lines.flatMap(line => plotted[line.key].map(entry => entry.at));
  const minutes = (Math.max(...times) - Math.min(...times)) / 60000;
  chart.span.textContent = count < 2
    ? "One sample so far; a new one every 30 s."
    : `${count} samples over the last ${minutes < 1 ? "minute" : `${Math.round(minutes)} min`}.`;
}

function drawPills(lines) {
  for (const line of lines) {
    const unit = UNIT[line.key];
    const values = plotted[line.key].map(entry => entry.value).filter(value => Number.isFinite(value));
    const pill = document.createElement("div");
    pill.className = `chart-pill series-${line.key}`;
    const head = document.createElement("p");
    head.className = "chart-pill-head";
    const name = document.createElement("span");
    name.className = "chart-pill-name";
    name.textContent = line.label;
    const current = document.createElement("strong");
    current.textContent = values.length ? formatValue(unit, values.at(-1)) : "—";
    head.append(name, current);
    const detail = document.createElement("p");
    detail.className = "chart-pill-detail";
    detail.textContent = values.length
      ? `min ${formatValue(unit, Math.min(...values))} · avg ${formatValue(unit, values.reduce((sum, value) => sum + value, 0) / values.length)} · max ${formatValue(unit, Math.max(...values))}`
      : "No samples yet";
    pill.append(head, detail);
    chart.pills.append(pill);
  }
}
