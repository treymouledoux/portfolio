"use strict";

const button = document.querySelector("#refresh");
const receipt = document.querySelector("#receipt");
const website = document.querySelector("#website");
const jetkvm = document.querySelector("#jetkvm");
const piMachine = document.querySelector("#pi-machine");
const sshCommand = document.querySelector("#ssh-command");
const localTime = new Intl.DateTimeFormat(undefined, {timeStyle: "medium"});

// The Pi keeps one row per check, so a fresh tab opens on a full graph instead
// of an empty box waiting for the first poll. Every device keeps its own
// series; CPU and RAM are single lines over the same rows.
const DEVICES = {
  jetkvm: {label: "JetKVM"},
  muse: {label: "Muse"},
  portfolio: {label: "Portfolio (WAN)"},
};
const SERIES = {
  cpu: {label: "CPU"},
  ram: {label: "RAM used"},
};
const UNIT = {jetkvm: "ms", muse: "ms", portfolio: "ms", cpu: "%", ram: "MiB"};
const POINTS = 60;
const HISTORY_STORE = "dashboard-history";
const SVG = "http://www.w3.org/2000/svg";
const chosen = new Set(Object.keys(DEVICES));
const chart = {
  svg: document.getElementById("chart"),
  series: document.getElementById("chart-series"),
  title: document.getElementById("chart-title"),
  summary: document.getElementById("chart-summary"),
  span: document.getElementById("chart-span"),
  pills: document.getElementById("chart-pills"),
  menu: document.getElementById("device-menu"),
  count: document.getElementById("device-count"),
  tip: document.getElementById("chart-tip"),
  figure: document.querySelector("figure.chart"),
};
const viewButtons = [...document.querySelectorAll(".chart-tabs button")];
const deviceBoxes = [...document.querySelectorAll(".device-list input[type=checkbox]")];
const deviceToggle = document.getElementById("device-toggle");
const cpuMode = document.getElementById("cpu-mode");
const cpuModeWrap = document.getElementById("cpu-mode-wrap");
const chartMax = document.getElementById("chart-max");
let view = "ping";
let cpuView = "total";
let memoryTotal = null;
let rows = [];

function setReading(element, label, state = "unknown") {
  element.lastElementChild.textContent = label;
  element.dataset.state = state;
}

function latency(probe) {
  if (!probe) return "Awaiting check";
  if (probe.latency_ms === 0) return "<1 ms";
  return Number.isFinite(probe.latency_ms) ? `${probe.latency_ms} ms` : "No response";
}

function showSnapshot(snapshot) {
  piMachine.textContent = typeof snapshot.pi_machine === "string" && snapshot.pi_machine.length > 0
    ? snapshot.pi_machine : "the Pi";
  const configured = Boolean(snapshot.pi_machine && snapshot.pi_ssh_user);
  sshCommand.textContent = configured ? `ssh ${snapshot.pi_ssh_user}@${snapshot.pi_machine}` : "";
  sshCommand.hidden = !configured;
  document.getElementById("web-ms").textContent = latency(snapshot.website_probe);
  document.getElementById("ping-ms").textContent = latency(snapshot.ping_probe);
  document.getElementById("muse-ms").textContent = latency(snapshot.muse_ping_probe);
  const code = snapshot.website_probe?.code;
  document.getElementById("web-code").textContent = Number.isFinite(code)
    ? (code === 200 ? "200 OK" : String(code)) : "No response";
  document.getElementById("kvm-ip").textContent = snapshot.jetkvm_ip || "Not configured";
  document.getElementById("muse-ip").textContent = snapshot.muse_ip || "Not configured";
  const online = snapshot.muse?.online;
  setReading(document.getElementById("muse"), online === true ? "Connected" : online === false ? "Offline" : "Unknown",
    online === true ? "good" : online === false ? "bad" : "unknown");
  const h = snapshot.host || {};
  memoryTotal = Number.isFinite(h.memory_total_mib) && h.memory_total_mib > 0 ? h.memory_total_mib : null;
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

// The rows the Pi measured, newest last. A failed reading stays a gap.
async function loadHistory(signal) {
  try {
    const response = await fetch("/api/history", {
      cache: "no-store", mode: "same-origin", redirect: "error",
      headers: {Accept: "application/json"}, signal
    });
    if (!response.ok) return false;
    const body = await response.json();
    if (!Array.isArray(body?.samples)) return false;
    rows = body.samples.filter(row => Number.isFinite(row?.at)).slice(-POINTS);
    syncMenu();
    drawChart();
    return true;
  } catch {
    return false;
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
    await loadHistory(controller.signal);
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
    saveView();
    drawChart();
  });
}

for (const box of deviceBoxes) {
  box.addEventListener("change", () => {
    if (box.checked) chosen.add(box.value);
    else chosen.delete(box.value);
    syncMenu();
    saveView();
    drawChart();
  });
}
cpuMode.addEventListener("change", () => {
  cpuView = cpuMode.value === "cores" ? "cores" : "total";
  syncMenu();
  saveView();
  drawChart();
});
deviceToggle.hidden = false;
deviceToggle.addEventListener("click", () => {
  const every = chosen.size === Object.keys(DEVICES).length;
  chosen.clear();
  if (!every) for (const key of Object.keys(DEVICES)) chosen.add(key);
  syncMenu();
  saveView();
  drawChart();
});

// Hovering any dot names that single check: device, value and local time.
chart.svg.addEventListener("pointermove", event => {
  const hit = event.target instanceof Element ? event.target.closest(".chart-hit") : null;
  if (hit) showTip(hit);
  else hideTip();
});
chart.svg.addEventListener("pointerleave", hideTip);
if (typeof ResizeObserver === "function") {
  let width = 0;
  new ResizeObserver(entries => {
    const next = Math.round(entries[0].contentRect.width);
    if (next === width) return;
    width = next;
    drawChart();
  }).observe(chart.figure);
}

restoreView();
syncMenu();
drawChart();
loadHistory();

function syncMenu() {
  chart.menu.hidden = view !== "ping";
  // The core breakdown only exists for CPU, and only once the Pi reports it.
  cpuModeWrap.hidden = view !== "cpu" || coreCount() < 2;
  cpuMode.value = cpuView;
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

// Only the chosen view and devices are remembered; the samples come from the Pi.
function restoreView() {
  try {
    const saved = JSON.parse(sessionStorage.getItem(HISTORY_STORE));
    const devices = Array.isArray(saved?.devices) ? saved.devices.filter(key => key in DEVICES) : [];
    if (devices.length) {
      chosen.clear();
      for (const key of devices) chosen.add(key);
    }
    if (["ping", "cpu", "ram"].includes(saved?.view)) view = saved.view;
    if (["total", "cores"].includes(saved?.cpu)) cpuView = saved.cpu;
    for (const control of viewButtons) control.setAttribute("aria-pressed", String(control.dataset.view === view));
  } catch {}
}

function saveView() {
  try {
    sessionStorage.setItem(HISTORY_STORE, JSON.stringify({devices: [...chosen], view, cpu: cpuView}));
  } catch {}
}

// Core series share the rows with the aggregate: `cores` holds one reading per
// CPU in /proc/stat order.
function coreCount() {
  return Math.min(8, Math.max(0, ...rows.map(row => Array.isArray(row.cores) ? row.cores.length : 0)));
}

function seriesFor(key) {
  const core = /^core(\d)$/.exec(key);
  const index = core ? Number(core[1]) : -1;
  return rows.map(row => {
    const value = index >= 0 ? row.cores?.[index] : row[key];
    return {at: row.at * 1000, value: Number.isFinite(value) ? value : null};
  });
}

function unitFor(key) {
  return UNIT[key] ?? "%";
}

// Dot radii that stay round on screen although the plot stretches to fit.
function dotSize(pixels) {
  const box = chart.svg.getBoundingClientRect();
  return {
    rx: (pixels * 640) / Math.max(box.width, 1),
    ry: (pixels * 200) / Math.max(box.height, 1),
  };
}

function showTip(target) {
  const at = Number(target.dataset.at);
  chart.tip.textContent = `${target.dataset.label} · ${target.dataset.value}`
    + (Number.isFinite(at) ? ` · ${localTime.format(new Date(at))}` : "");
  chart.tip.hidden = false;
  const point = target.getBoundingClientRect();
  const frame = chart.figure.getBoundingClientRect();
  const half = chart.tip.offsetWidth / 2;
  const left = Math.min(Math.max(point.left + point.width / 2 - frame.left, half + 8), Math.max(frame.width - half - 8, half + 8));
  chart.tip.style.left = `${Math.round(left)}px`;
  chart.tip.style.top = `${Math.round(point.top - frame.top - 6)}px`;
}

function hideTip() {
  chart.tip.hidden = true;
}

function drawChart() {
  const cores = view === "cpu" && cpuView === "cores" ? coreCount() : 0;
  const lines = view === "ping"
    ? Object.keys(DEVICES).filter(key => chosen.has(key)).map(key => ({key, label: DEVICES[key].label}))
    : cores > 0
      ? Array.from({length: cores}, (_, index) => ({key: `core${index}`, label: `Core ${index}`}))
      : [{key: view, label: SERIES[view].label}];
  const data = Object.fromEntries(lines.map(line => [line.key, seriesFor(line.key)]));
  chart.series.replaceChildren();
  chart.pills.replaceChildren();
  hideTip();
  chart.title.textContent = view === "ping"
    ? "Ping comparison"
    : `${SERIES[view].label} over time${cores ? `, one line per core` : ""}`;
  showMax(view === "ram" && Number.isFinite(memoryTotal));
  if (!lines.length) {
    chart.summary.textContent = "No devices selected. Pick at least one under Devices.";
    chart.span.textContent = "";
    return;
  }
  const count = Math.max(0, ...lines.map(line => data[line.key].length));
  const values = lines.flatMap(line => data[line.key]
    .map(entry => entry.value)
    .filter(value => Number.isFinite(value)));
  if (!values.length) {
    chart.summary.textContent = "Waiting for the first sample…";
    chart.span.textContent = "";
    drawPills(lines, data);
    return;
  }
  const low = Math.min(...values);
  const high = Math.max(...values);
  const room = high - low || Math.max(Math.abs(high) * 0.2, 1);
  // Memory is drawn against the whole module, so the top line is the maximum
  // the Pi has rather than the busiest moment in the window.
  const capped = view === "ram" && Number.isFinite(memoryTotal);
  const floor = capped ? 0 : view === "ping" ? 0 : low - room * 0.15;
  const ceiling = capped ? memoryTotal : view === "ping" ? Math.max(high * 1.15, 5) : high + room * 0.15;
  const x = index => (count < 2 ? 640 : (index / (count - 1)) * 640);
  const y = value => 190 - ((value - floor) / (ceiling - floor)) * 180;
  const dot = dotSize(3);
  const reach = dotSize(8);
  for (const line of lines) {
    const samples = data[line.key];
    let path = "";
    let drawing = false;
    const points = [];
    for (const [index, entry] of samples.entries()) {
      if (!Number.isFinite(entry.value)) { drawing = false; continue; }
      const cx = x(index).toFixed(1);
      const cy = y(entry.value).toFixed(1);
      path += `${drawing ? "L" : "M"}${cx} ${cy}`;
      drawing = true;
      points.push({cx, cy, at: entry.at, value: entry.value});
    }
    if (!path) continue;
    chart.series.append(svgNode("path", {
      d: path,
      class: `chart-line series-${line.key}`,
      "vector-effect": "non-scaling-stroke",
    }));
    for (const point of points) {
      chart.series.append(svgNode("ellipse", {
        cx: point.cx,
        cy: point.cy,
        rx: dot.rx.toFixed(2),
        ry: dot.ry.toFixed(2),
        class: `chart-dot series-${line.key}`,
      }));
      chart.series.append(svgNode("ellipse", {
        cx: point.cx,
        cy: point.cy,
        rx: reach.rx.toFixed(2),
        ry: reach.ry.toFixed(2),
        class: "chart-hit",
        "data-label": line.label,
        "data-value": formatValue(unitFor(line.key), point.value),
        "data-at": String(point.at),
      }));
    }
  }
  drawPills(lines, data);
  // The latest readings of the chosen view, not of every device.
  const latestValues = lines
    .map(line => data[line.key].filter(entry => Number.isFinite(entry.value)).at(-1)?.value)
    .filter(value => Number.isFinite(value));
  const average = values.reduce((sum, value) => sum + value, 0) / values.length;
  if (view === "ping") {
    const unit = "ms";
    chart.summary.textContent =
      `${lines.length} device${lines.length === 1 ? "" : "s"} · range ${formatValue(unit, low)}–${formatValue(unit, high)}`;
  } else {
    const unit = unitFor(view);
    if (cores) {
      const now = lines.map(line => {
        const last = data[line.key].filter(entry => Number.isFinite(entry.value)).at(-1)?.value;
        return Number.isFinite(last) ? formatValue(unit, last) : "—";
      });
      chart.summary.textContent =
        `${cores} cores · now ${now.join(" · ")} · avg ${formatValue(unit, average)} · peak ${formatValue(unit, high)}`;
    } else {
      chart.summary.textContent =
        `Now ${formatValue(unit, latestValues[0] ?? average)} · avg ${formatValue(unit, average)} · low ${formatValue(unit, low)} · high ${formatValue(unit, high)}`;
    }
    if (capped) chart.summary.textContent += ` · ${formatValue(unit, memoryTotal)} max`;
  }
  const times = lines.flatMap(line => data[line.key].map(entry => entry.at));
  const minutes = (Math.max(...times) - Math.min(...times)) / 60000;
  chart.span.textContent = count < 2
    ? "One check so far; a new one every 30 s."
    : `${count} checks over the last ${minutes < 1 ? "minute" : `${Math.round(minutes)} min`}.`;
}

function showMax(show) {
  chartMax.hidden = !show;
  chartMax.textContent = show ? `${Math.round(memoryTotal)} MiB installed · top line is the maximum` : "";
}

function drawPills(lines, data) {
  for (const line of lines) {
    const unit = unitFor(line.key);
    const values = data[line.key].map(entry => entry.value).filter(value => Number.isFinite(value));
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
