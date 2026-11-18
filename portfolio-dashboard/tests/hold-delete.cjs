// Local browser regression. Uses Playwright Core + an installed Chromium via
// CHROME_PATH. Every request is fulfilled in memory with synthetic values.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright-core");

const source = name => readFileSync(path.join(__dirname, "../src", name), "utf8");
const script = source("hold-delete.js");
const css = source("dashboard.css");
const origin = "https://inbox.example.test";
const id = "a".repeat(64);
const hold = "b".repeat(64);
const commits = [];
const arms = [];
const cancels = [];
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

function documentFor(operation) {
  const label = operation === "delete" ? "Delete message" : "Delete all messages";
  // Exercise the actual confirmation control. Revealing the form simulates
  // passkey.js having verified an existing session on this synthetic origin.
  const match = source("inbox_actions.rs").match(/<form id="delete-form"[^]*?<\/form>/);
  assert.ok(match, "missing private confirmation form");
  const form = match[0].replaceAll("{action}", operation).replaceAll("{id}", id)
    .replaceAll("{button}", label);
  return `<!doctype html><meta name="viewport" content="width=device-width">
    <link rel="stylesheet" href="/dashboard.css"><main class="inbox-action">
    ${form}<a href="/">Cancel</a></main><script src="/hold-delete.js" defer></script>
    <script>document.querySelector("form").hidden = false;</script>`;
}

async function fixture(browser, javaScriptEnabled = true) {
  const context = await browser.newContext({ javaScriptEnabled, hasTouch: true });
  await context.route("**/*", async route => {
    const request = route.request();
    const url = new URL(request.url());
    assert.equal(url.origin, origin);
    if (url.pathname === "/hold-delete.js")
      return route.fulfill({ contentType: "text/javascript", body: script });
    if (url.pathname === "/dashboard.css")
      return route.fulfill({ contentType: "text/css", body: css });
    if (url.pathname === "/messages/auth/arm") {
      assert.equal(request.method(), "POST");
      arms.push(request.postDataJSON());
      return route.fulfill({ json: { hold } });
    }
    if (url.pathname === "/messages/auth/cancel") {
      cancels.push(request.postDataJSON());
      return route.fulfill({ json: { ok: true } });
    }
    if (request.method() === "POST") {
      commits.push({ path: url.pathname, body: request.postData() });
      return route.fulfill({ contentType: "text/html", body: "<h1>Fixture complete</h1>" });
    }
    assert.ok(["/delete", "/delete-all"].includes(url.pathname));
    return route.fulfill({ contentType: "text/html", body: documentFor(url.pathname.slice(1)) });
  });
  const page = await context.newPage();
  await page.goto(origin + "/delete");
  return { context, page };
}

async function run() {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH, headless: true,
    args: ["--disable-background-networking", "--use-mock-keychain"],
  });
  try {
    const disabled = await fixture(browser, false);
    assert.ok(await disabled.page.locator("#hold-delete").isDisabled());
    assert.ok(await disabled.page.locator("form").isHidden());
    assert.equal(await disabled.page.locator("button[type=submit],input[type=submit]").count(), 0);
    await disabled.page.keyboard.press("Enter");
    assert.equal(commits.length, 0);
    await disabled.context.close();

    const { page, context } = await fixture(browser);
    const button = page.locator("#hold-delete");
    const state = value => page.waitForFunction(value =>
      document.querySelector("#hold-delete")?.dataset.state === value, value);
    const arm = async () => { await button.click(); await state("armed"); };
    const holdPointer = async () => { await page.mouse.down(); await state("holding"); };

    await page.locator("form").evaluate(form => form.requestSubmit());
    assert.equal(commits.length, 0);
    await arm();
    assert.deepEqual(arms.at(-1), { operation: "delete", id });
    await holdPointer();
    await page.waitForFunction(() => {
      const fill = getComputedStyle(document.querySelector("#hold-delete"), "::before").transform;
      const scale = new DOMMatrix(fill).a;
      return scale > 0.01 && scale < 1;
    });
    await page.mouse.up();
    await state("idle");
    assert.deepEqual(cancels.at(-1), { hold });
    assert.ok(await page.locator("input[name=hold]").isDisabled());

    for (const interrupt of [
      () => page.keyboard.press("Escape"),
      () => page.locator("a").focus(),
      () => page.mouse.move(1, 1),
      () => page.evaluate(() => {
        Object.defineProperty(document, "hidden", { value: true, configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));
        delete document.hidden;
      }),
    ]) {
      await arm();
      await holdPointer();
      await interrupt();
      await state("idle");
      await page.mouse.up();
      await state("idle");
    }

    // Real Chromium touch cancellation, e.g. the OS interrupts a long press.
    await arm();
    const cdp = await context.newCDPSession(page);
    const box = await button.boundingBox();
    await cdp.send("Input.dispatchTouchEvent", {
      type: "touchStart", touchPoints: [{ x: box.x + box.width / 2, y: box.y + box.height / 2 }],
    });
    await state("holding");
    await cdp.send("Input.dispatchTouchEvent", { type: "touchCancel", touchPoints: [] });
    await state("idle");
    assert.equal(commits.length, 0);

    // Native key repeat during the initial Enter must not start a second gesture.
    await button.focus();
    await page.keyboard.down("Enter");
    await state("armed");
    await page.keyboard.down("Enter");
    await state("armed");
    await page.keyboard.up("Enter");
    await page.keyboard.down("Space");
    await state("holding");
    await page.keyboard.up("Space");
    await state("idle");

    await arm();
    await holdPointer();
    await wait(4200);
    await state("holding");
    assert.equal(commits.length, 0);
    await page.getByRole("heading", { name: "Fixture complete" }).waitFor();
    await page.mouse.up();
    assert.deepEqual(commits, [{ path: `/messages/action/delete/${id}`, body: "hold=" + hold }]);

    await page.goto(origin + "/delete-all");
    await button.focus();
    await page.keyboard.press("Space");
    await state("armed");
    assert.deepEqual(arms.at(-1), { operation: "delete-all", id });
    await page.keyboard.down("Enter");
    await state("holding");
    await page.getByRole("heading", { name: "Fixture complete" }).waitFor();
    await page.keyboard.up("Enter");
    assert.equal(commits.length, 2);
    assert.deepEqual(commits[1], { path: `/messages/action/delete-all/${id}`, body: "hold=" + hold });
    console.log("PASS: inert without JS, two-step target binding, red progress, interruption, touch and keyboard, five-second commits");
  } finally {
    await browser.close();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
