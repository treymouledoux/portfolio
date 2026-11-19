// Synthetic browser test. All documents are supplied by the test's in-memory route.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const { chromium } = require("playwright-core");

const root = path.join(__dirname, "../../deploy/dashboard");
const pageText = name => readFileSync(path.join(root, name), "utf8");
const apex = "https://treymouledoux.me";
const privateOrigin = "https://status.example.invalid:8443";
const policy = pageText("Caddyfile").match(/Content-Security-Policy "([^"]+)"/)[1];

async function run() {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH, headless: true,
    args: ["--disable-background-networking", "--use-mock-keychain"],
  });
  try {
    const page = await browser.newPage();
    await page.clock.install();
    const counts = {};
    await page.route("**/*", async route => {
      const url = new URL(route.request().url());
      counts[url.pathname] = (counts[url.pathname] || 0) + 1;
      if (url.origin === apex) {
        assert.equal(url.pathname, "/dashboard");
        return route.fulfill({
          contentType: "text/html", body: pageText("landing.html"), headers: {
            "Content-Security-Policy": policy, "Cache-Control": "private, no-store",
          },
        });
      }
      assert.equal(url.origin, privateOrigin);
      const fixtures = {
        "/embed/status": '<a href="/embed/status">Service check</a>',
        "/embed/inbox-count": readFileSync(path.join(__dirname, "../src/inbox-badge.html"), "utf8")
          .replace("{{COUNT}}", "99+").replace("{{DETAIL}}", "103 unread / 200 total").replace("{{TONE}}", ""),
        "/messages": "<article>Synthetic message</article>",
      };
      assert.ok(fixtures[url.pathname]);
      return route.fulfill({contentType: "text/html", body: fixtures[url.pathname]});
    });
    async function advance() {
      // Wait for each requested counter to settle after asynchronous frame navigation.
      const before = counts["/embed/inbox-count"];
      const refreshed = page.waitForResponse(response => response.url() === privateOrigin + "/embed/inbox-count");
      await page.clock.fastForward(30_000);
      await refreshed;
      assert.equal(counts["/embed/inbox-count"], before + 1);
    }
    await page.goto(apex + "/dashboard");
    const badge = page.locator("iframe.count");
    assert.equal(await badge.getAttribute("sandbox"), "");
    assert.equal(counts["/messages"], undefined);
    for (const width of [320, 390, 768, 1280]) {
      await page.setViewportSize({width, height: 850});
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth));
      assert.ok(await badge.isVisible());
      const pill = page.frameLocator("iframe.count").getByRole("img", { name: "103 unread / 200 total" });
      const [frameBox, pillBox] = await Promise.all([badge.boundingBox(), pill.boundingBox()]);
      assert.ok(frameBox.width <= 32 && pillBox.width <= frameBox.width && pillBox.height <= frameBox.height, JSON.stringify({width, frameBox, pillBox}));
    }
    await advance();
    assert.equal(counts["/embed/status"], 2);
    assert.equal(counts["/dashboard"], 1);
    await page.evaluate(() => {
      Object.defineProperty(document, "hidden", { configurable: true, value: true });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await page.clock.fastForward(60_000);
    assert.equal(counts["/embed/status"], 2);
    const visible = page.waitForResponse(response => response.url() === privateOrigin + "/embed/status");
    await page.evaluate(() => {
      delete document.hidden;
      document.dispatchEvent(new Event("visibilitychange"));
    });
    await visible;
    await page.getByRole("button", { name:"Pause live updates" }).click();
    await page.clock.fastForward(60_000);
    assert.equal(counts["/embed/status"], 3);
    await page.getByRole("button", { name:"Resume live updates" }).click();
    await page.frameLocator("iframe.service-frame").getByRole("link").focus();
    const statusReads = counts["/embed/status"];
    await advance();
    assert.equal(counts["/embed/status"], statusReads);
    const resumed = page.waitForResponse(response => response.url() === privateOrigin + "/embed/status");
    await advance();
    await resumed;
    assert.equal(counts["/embed/status"], statusReads + 1);
    // The overlaid link covers the badge without delivering pointer events into the frame.
    const box = await badge.boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await page.waitForURL(privateOrigin + "/messages");
    await page.getByRole("article").waitFor();
    assert.equal(counts["/messages"], 1);
    console.log("Verified visible private badge, polling, pause/resume, focus, and same-tab private inbox navigation.");
  } finally {
    await browser.close();
  }
}

run().catch(error => { console.error(error); process.exitCode = 1; });
