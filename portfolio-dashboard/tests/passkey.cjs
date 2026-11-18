// Synthetic Chromium WebAuthn registration + assertion against the actual FIDO broker.
// The virtual authenticator, isolated profile, test login and database exist only here.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");
const { createInterface } = require("node:readline");
const { chromium } = require("playwright-core");

const origin = "https://private.example.ts.net:8443";
const login = "owner@example.invalid";
const src = name => fs.readFileSync(path.join(__dirname, "../src", name), "utf8");
const python = `
import importlib.util, json, pathlib, sys, tempfile
spec = importlib.util.spec_from_file_location("broker", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
with tempfile.TemporaryDirectory() as fixture:
  root = pathlib.Path(fixture)
  broker = module.Broker(root, root/"fixture.db", sys.argv[2], {sys.argv[3]})
  broker.handle({"action":"open-enrollment","user":sys.argv[3]}, issuer=True)
  for line in sys.stdin:
    try:
      reply = broker.handle(json.loads(line))
    except module.Rejected as error:
      reply = {"ok":False,"error":str(error)}
    print(json.dumps(reply), flush=True)
`;

async function run() {
  const child = spawn(process.env.FIDO_PYTHON || "python3", ["-Bu", "-c", python,
    path.join(__dirname, "../../deploy/dashboard/inbox-actions.py"), origin, login],
    { stdio: ["pipe", "pipe", "inherit"] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  async function broker(command) {
    child.stdin.write(JSON.stringify({ user: login, ...command }) + "\n");
    const next = await lines.next();
    assert.equal(next.done, false, "synthetic broker stopped");
    return JSON.parse(next.value);
  }
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH, headless: true,
    args: ["--disable-background-networking", "--use-mock-keychain"] });
  try {
    const context = await browser.newContext();
    let session;
    const observed = [];
    await context.route("**/*", async route => {
      const url = new URL(route.request().url());
      assert.equal(url.origin, origin);
      if (url.pathname === "/messages/passkey")
        return route.fulfill({ contentType: "text/html", body: "<!doctype html>" + src("passkey-panel.html") +
          '<form id="delete-form" hidden></form><script src="/passkey.js" defer></script>' });
      if (url.pathname === "/passkey.js")
        return route.fulfill({ contentType: "text/javascript", body: src("passkey.js") });
      assert.equal(route.request().method(), "POST");
      assert.ok(url.pathname.startsWith("/messages/auth/"));
      const action = url.pathname.slice("/messages/auth/".length);
      observed.push(action);
      if (action === "session" && !session)
        return route.fulfill({ status:403, json:{error:"denied"} });
      const command = { action, ...route.request().postDataJSON() };
      if (action === "session") command.session = session;
      const result = await broker(command);
      assert.equal(result.ok, true, action + " was rejected");
      if (action === "login-finish") {
        assert.match(result.session, /^[a-f0-9]{64}$/);
        session = result.session;
      }
      // Only the same fields as the HTTP adapter are visible to page JavaScript.
      if (action.endsWith("-options"))
        return route.fulfill({json:{flow:result.flow, publicKey:result.publicKey}});
      return route.fulfill({json:{ok:true}});
    });
    const page = await context.newPage();
    page.setDefaultTimeout(12000);
    const cdp = await context.newCDPSession(page);
    await cdp.send("WebAuthn.enable");
    await cdp.send("WebAuthn.addVirtualAuthenticator",{options:{
      protocol:"ctap2",transport:"internal",hasResidentKey:true,
      hasUserVerification:true,isUserVerified:true,automaticPresenceSimulation:true
    }});
    await page.goto(origin + "/messages/passkey");
    await page.getByRole("button", {name:"Set up a passkey"}).click();
    await page.getByText("Passkey registered.", {exact:false}).waitFor();
    assert.equal(session, undefined);
    await page.getByRole("button", {name:"Sign in with passkey"}).click();
    await page.getByText("Passkey verified.", {exact:true}).waitFor();
    assert.equal(await page.locator("#delete-form").evaluate(form => form.hidden),false);
    await page.reload();
    await page.getByText("Passkey verified.", {exact:true}).waitFor();
    assert.deepEqual(observed,["session","enroll-options","enroll-finish","login-options","login-finish","session"]);
    console.log("Chromium virtual passkey registration, signed assertion and session reuse passed.");
  } finally {
    await browser.close();
    child.stdin.end();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
