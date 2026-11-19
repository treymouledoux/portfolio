// Local browser regression. Uses Playwright Core + an installed Chromium via
// CHROME_PATH. Every request is fulfilled in memory with synthetic values.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright-core");

const source = name => readFileSync(path.join(__dirname, "../src", name), "utf8");
const origin = "https://inbox.example.test";
const id = "a".repeat(64);
const hold = "b".repeat(64);
const commits = [];
const arms = [];
const cancels = [];
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

const bulkId = "c".repeat(64);

function documentFor() {
  const match = source("inbox.rs").match(/<form class="remove-form"[^]*?<\/form>/);
  assert.ok(match, "missing inline confirmation form");
  const form = (operation, target, label) => match[0].replaceAll("{operation}", operation)
    .replaceAll("{id}", target).replaceAll("{label}", label);
  const entries = `<div id="inbox-notice" role="status"></div>
    <form class="mark-form" method="post" action="/messages/read/${id}?end=34"><button disabled>Mark as read</button></form>
    ${form("delete", id, "Remove message")}${form("delete-all", "", "Remove all messages")}`;
  // app.js is served empty here, so the inbox falls back to a document reload.
  return source("app.html").replace("{{TITLE}}", "Messages")
    .replace("{{OVERVIEW_HIDDEN}}", " hidden").replace("{{MESSAGES_HIDDEN}}", "")
    .replace("{{OVERVIEW_CURRENT}}", "").replace("{{MESSAGES_CURRENT}}", ' aria-current="page"')
    .replace("{{MESSAGES}}", `<div id="inbox-view" data-unread="1" data-total="1">${entries}</div>`);
}

async function fixture(browser, javaScriptEnabled = true) {
  const context = await browser.newContext({ javaScriptEnabled, hasTouch: true });
  let loads = 0;
  let locked = false;
  let marked = false;
  await context.route("**/*", async route => {
    const request = route.request();
    const url = new URL(request.url());
    assert.equal(url.origin, origin);
    if (url.pathname === "/messages") {
      loads++;
      return route.fulfill({contentType:"text/html", body: locked ? "<h1>Locked</h1>" : documentFor()});
    }
    if (["/passkey.js","/hold-delete.js","/messages.js","/session.js","/messages.css","/dashboard.css"].includes(url.pathname))
      return route.fulfill({contentType:url.pathname.endsWith("css") ? "text/css" : "text/javascript",body:source(url.pathname.slice(1))});
    if (["/app.js","/dashboard.js"].includes(url.pathname))
      return route.fulfill({contentType:"text/javascript", body:""});
    if (url.pathname === "/messages/auth/session")
      return route.fulfill({status:locked ? 403 : 200, json: locked ? {error:"denied"} : {ok:true, expires_in:600}});
    if (url.pathname === "/messages/auth/lock") { locked = true; return route.fulfill({json:{ok:true}}); }
    if (url.pathname === "/messages/delete-all") return route.fulfill({json:{id:bulkId,count:3}});
    assert.equal(request.method(), "POST");
    if (url.pathname === "/messages/auth/arm") {
      arms.push(request.postDataJSON());
      return route.fulfill({ json: { hold } });
    }
    if (url.pathname === "/messages/auth/cancel") {
      cancels.push(request.postDataJSON());
      return route.fulfill({ json: { ok: true } });
    }
    if (url.pathname === "/messages/read/" + id) {
      assert.equal(url.search, "?end=34");
      marked = true;
      return route.fulfill({status:204});
    }
    assert.ok(url.pathname.startsWith("/messages/action/"));
    commits.push({ path: url.pathname, body: request.postData() });
    return route.fulfill({status:204});
  });
  const page = await context.newPage();
  await page.goto(origin + "/messages");
  return { context, page, loads: () => loads, marked: () => marked, expire: () => { locked = true; } };
}

async function run() {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH, headless: true,
    args: ["--disable-background-networking", "--use-mock-keychain"],
  });
  try {
    const disabled = await fixture(browser, false);
    assert.ok(await disabled.page.locator(".hold-delete").first().isDisabled());
    assert.ok(await disabled.page.getByRole("button", {name:"Mark as read"}).isDisabled());
    await disabled.page.keyboard.press("Enter");
    assert.equal(commits.length, 0);
    await disabled.context.close();

    const { page, context, loads, marked } = await fixture(browser);
    await Promise.all([page.waitForNavigation(),page.getByRole("button", {name:"Mark as read"}).click()]);
    assert.equal(page.url(), origin + "/messages");
    assert.ok(marked());
    assert.equal(loads(), 2);
    let operation = "delete";
    let button = page.locator('[data-operation="delete"] .hold-delete');
    const state = value => page.waitForFunction(([op, expected]) =>
      document.querySelector(`[data-operation="${op}"] .hold-delete`)?.dataset.state === expected, [operation, value]);
    const arm = async () => { await button.click(); await state("armed"); };
    const holdPointer = async () => { await page.mouse.down(); await state("holding"); };

    await page.locator(".remove-form").first().evaluate(form => form.requestSubmit());
    assert.equal(commits.length, 0);
    await arm();
    assert.deepEqual(arms.at(-1), { operation: "delete", id });
    await holdPointer();
    await page.waitForFunction(() => {
      const fill = getComputedStyle(document.querySelector(".hold-delete"), "::before").transform;
      const scale = new DOMMatrix(fill).a;
      return scale > 0.01 && scale < 1;
    });
    await page.mouse.up();
    await state("idle");
    assert.deepEqual(cancels.at(-1), { hold });
    assert.ok(await page.locator("input[name=hold]").first().isDisabled());

    for (const interrupt of [
      () => page.keyboard.press("Escape"),
      () => page.locator("a").first().focus(),
      () => page.mouse.move(1, 1),
      async () => {
        await page.evaluate(() => {
          Object.defineProperty(document, "hidden", {value:true, configurable:true});
          document.dispatchEvent(new Event("visibilitychange"));
          delete document.hidden;
          document.dispatchEvent(new Event("visibilitychange"));
        });
      },
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
    await page.waitForNavigation();
    await page.mouse.up();
    assert.deepEqual(commits, [{ path: `/messages/action/delete/${id}`, body: "hold=" + hold }]);

    operation = "delete-all";
    button = page.locator('[data-operation="delete-all"] .hold-delete');
    await button.focus();
    await page.keyboard.press("Space");
    await state("armed");
    assert.deepEqual(arms.at(-1), { operation: "delete-all", id:bulkId });
    await page.keyboard.down("Enter");
    await state("holding");
    await page.waitForNavigation();
    await page.keyboard.up("Enter");
    assert.equal(commits.length, 2);
    assert.deepEqual(commits[1], { path: `/messages/action/delete-all/${bulkId}`, body: "hold=" + hold });
    assert.equal(page.url(), origin + "/messages");
    assert.equal(context.pages().length, 1);
    // The header badge is the one lock action now.
    await page.getByRole("button", {name:/Admin session/}).click();
    await page.getByRole("heading", {name:"Locked"}).waitFor();
    // An expired session drops the page back to its locked controls.
    const expired = await fixture(browser);
    expired.expire();
    await expired.page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await expired.page.getByRole("heading", {name:"Locked"}).waitFor();
    assert.equal(expired.page.url(), origin + "/messages");
    console.log("PASS: inert without JS, two-step target binding, red progress, interruption, touch and keyboard, five-second commits");
  } finally {
    await browser.close();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
