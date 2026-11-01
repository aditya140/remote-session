#!/usr/bin/env python3
"""Qualify the real browser console against an isolated local Keycloak deployment.

The fixture uses real OAuth, PostgreSQL, gateway routing, CLI enrollment, and two
native PTYs. It never operates on the original qgr-local deployment. Trust the
fixture CA with --install-browser-ca; TLS verification remains enabled.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "deploy/compose"))
from stack import Stack, private
from keycloak import USERS, ready
from local_keycloak import login as cli_tokens, submit
from local_tui_smoke import request as local_request


class ProofFailure(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise ProofFailure(message)


def wait(condition, message, seconds=30):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = condition()
            if result:
                return result
        except Exception:
            # Conditions here are bounded read-only probes; mutations stay outside.
            pass
        time.sleep(0.2)
    raise ProofFailure(message)


@contextmanager
def browser_ca(path, install):
    """Trust only this public fixture CA, and remove only the new trust entry."""
    if not install:
        yield
        return
    if not shutil.which("certutil"):
        raise ProofFailure("certutil is required to install the browser fixture CA")
    database = Path.home() / ".pki/nssdb"
    database.mkdir(mode=0o700, parents=True, exist_ok=True)
    binding = "sql:" + str(database)
    if not (database / "cert9.db").exists():
        subprocess.run(["certutil", "-N", "--empty-password", "-d", binding],
                       check=True, capture_output=True)
    alias = "qgr-console-e2e-" + hashlib.sha256(path.read_bytes()).hexdigest()[:16]
    existed = subprocess.run(["certutil", "-L", "-d", binding, "-n", alias],
                             capture_output=True).returncode == 0
    if not existed:
        subprocess.run(["certutil", "-A", "-d", binding, "-n", alias, "-t", "C,,",
                        "-i", str(path)], check=True, capture_output=True)
    try:
        yield
    finally:
        if not existed:
            subprocess.run(["certutil", "-D", "-d", binding, "-n", alias], capture_output=True)


def browser_request(page, path, *, method="GET", body=None, headers=None):
    return page.evaluate("""async ({path, method, body, headers}) => {
      const response = await fetch(path, {method, credentials: 'same-origin',
        headers: {...(body === null ? {} : {'Content-Type': 'application/json'}), ...headers},
        signal: method === 'GET' ? AbortSignal.timeout(5000) : undefined,
        body: body === null ? undefined : JSON.stringify(body), cache: 'no-store'});
      let value = null; try { value = await response.json(); } catch (_) {}
      return {status: response.status, value};
    }""", {"path": path, "method": method, "body": body, "headers": headers or {}})


def session(page):
    response = browser_request(page, "/console/auth/session")
    require(response["status"] == 200, "browser session request failed")
    value = response["value"]
    require(value.get("authenticated") is True, "browser sign-in did not create an authenticated session")
    require(value.get("user") and value.get("csrf_token"), "browser session lacks identity or CSRF binding")
    require(not any(key in value for key in ("access_token", "refresh_token", "id_token")),
            "browser session exposed upstream tokens")
    return value


def sign_in(browser, origin, name, password):
    context = browser.new_context(viewport={"width": 1600, "height": 1100})
    context.set_default_timeout(15000)
    page = context.new_page()
    submit(page, origin + "/console/auth/login", name, password, approve=True)
    page.wait_for_url(re.compile(re.escape(origin) + r"/console(?:/|$|\?)"), timeout=25000)
    wait(lambda: browser_request(page, "/console/auth/session")["status"] == 200,
         "browser callback did not complete", seconds=20)
    identity = session(page)
    page.locator("#app").wait_for(state="visible", timeout=15000)
    page.locator("#nav button[data-page]").first.wait_for(state="visible", timeout=20000)
    return context, page, identity


def raw_request(origin, ca, path, headers=None, method="GET", body=None):
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
        urllib.request.HTTPSHandler(context=ssl.create_default_context(cafile=str(ca))))
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(origin + path, method=method, data=data,
                                    headers=headers or {})
    try:
        with opener.open(request, timeout=10) as response:
            value = response.read(128 * 1024)
            return response.status, value
    except urllib.error.HTTPError as error:
        return error.code, error.read(128 * 1024)


def browser_guards(origin, ca, context, page, identity):
    require(browser_request(page, "/console/auth/logout", method="POST")["status"] == 403,
            "unsafe cookie request without CSRF was accepted")
    cookies = "; ".join(cookie["name"] + "=" + cookie["value"]
                        for cookie in context.cookies(origin))
    headers = {"Cookie": cookies, "Origin": "https://foreign.example.invalid",
               "X-CSRF-Token": identity["csrf_token"], "Content-Type": "application/json"}
    require(raw_request(origin, ca, "/console/auth/logout", headers, "POST", {})[0] == 403,
            "cross-origin cookie mutation was accepted")
    websocket_headers = {"Cookie": cookies, "Origin": "https://foreign.example.invalid",
        "Connection": "Upgrade", "Upgrade": "websocket", "Sec-WebSocket-Version": "13",
        "Sec-WebSocket-Key": "dGhlIHNhbXBsZSBub25jZQ=="}
    require(raw_request(origin, ca, "/v1/connect/client", websocket_headers)[0] == 403,
            "cross-origin cookie WebSocket upgrade was accepted")
    require(page.evaluate("Object.keys(localStorage).every(k => !/token/i.test(k))"),
            "token material appeared in local storage")
    require(all(cookie["httpOnly"] and cookie["secure"] and cookie["sameSite"] in ("Lax", "Strict")
                for cookie in context.cookies(origin) if "qgr" in cookie["name"]),
            "browser auth cookie lacks required transport protection")


def gateway_failover(stack, page, expected_user):
    """Force requests onto different healthy nodes; restore all owned nodes."""
    first = ["gateway-1", "gateway-2", "gateway-3"]
    for node in (1, 2, 3, 4):
        stack.wait(lambda n=node: stack.probe(n), "pre-failover gateway readiness", seconds=35)
    page.bring_to_front()
    observed = {}
    def authenticated():
        try:
            response = browser_request(page, "/console/auth/session")
            value = response["value"] or {}
            parsed = urllib.parse.urlsplit(page.url)
            observed.update(status=response["status"], authenticated=value.get("authenticated"),
                            path=parsed.path, host=parsed.hostname)
            return response["status"] == 200 and value.get("user") == expected_user
        except Exception as error:
            observed.update(error=type(error).__name__)
            return False
    def verify(label):
        try:
            wait(authenticated, label, seconds=30)
        except ProofFailure:
            raise ProofFailure(label + ": " + json.dumps(observed)) from None
    def only_gateways(expected):
        services = set(stack.call("ps", "--status", "running", "--services", capture=True).decode().splitlines())
        return {name for name in services if name.startswith("gateway-")} == set(expected)
    try:
        stack.call("stop", "--timeout", "1", *first, capture=True)
        require(only_gateways(["gateway-4"]), "first failover did not isolate gateway 4")
        verify("shared session did not work on remaining gateway")
        stack.call("start", *first, capture=True)
        for node in (1, 2, 3):
            stack.wait(lambda n=node: stack.probe(n), "gateway restoration", seconds=35)
        stack.call("stop", "--timeout", "1", "gateway-4", capture=True)
        require(only_gateways(first), "second failover did not exclude gateway 4")
        verify("shared session did not survive switching away from gateway 4")
    finally:
        stack.call("start", "gateway-1", "gateway-2", "gateway-3", "gateway-4", capture=True)
        for node in (1, 2, 3, 4):
            stack.wait(lambda n=node: stack.probe(n), "gateway restoration", seconds=35)


class NativeFixture:
    def __init__(self, browser, stack, binary, credentials, work):
        self.stack, self.work, self.sessions = stack, work, []
        self.native, self.client = work / "native", work / "client"
        self.root = work / "console-native"
        self.root.mkdir(mode=0o700)
        self.config = work / "host.json"
        self.started = False
        self.diagnostics = (work / "native-diagnostics.log").open("wb")
        self.origin = f"https://localhost:{stack.args.edge_port}"
        self.machine_name = "console-proof-" + secrets.token_hex(4)
        private(self.config, json.dumps({"gateway": self.origin, "os_user": "console-e2e",
            "name": self.machine_name, "hostname": self.machine_name, "roots": [str(self.root)],
            "allow_private_file": True, "ca_bundle": str(stack.args.state / "certificates/ca.pem")}))
        self.base = [str(binary), "--state-dir", str(self.native)]
        self.host = self.base + ["host", "--config", str(self.config)]
        self.remote = [str(binary), "--state-dir", str(self.client), "remote", "--gateway", self.origin,
                       "--ca-bundle", str(stack.args.state / "certificates/ca.pem")]
        self.token = work / "bob-token"
        tokens = cli_tokens(browser, stack, "bob", credentials["bob"])
        private(self.token, tokens["access_token"])
        self.remote += ["--token-file", str(self.token)]
        try:
            self.enroll(browser, credentials["bob"])
        except BaseException:
            self.cleanup()
            raise

    def command(self, argv, timeout=30):
        result = subprocess.run(argv, stdout=subprocess.PIPE, stderr=self.diagnostics, timeout=timeout)
        require(result.returncode == 0, "native CLI operation failed; inspect private diagnostics")
        return json.loads(result.stdout) if result.stdout.startswith((b"[", b"{")) else None

    def enroll(self, browser, password):
        opener = self.work / "bin"
        opener.mkdir(mode=0o700)
        script = opener / "xdg-open"
        private(script, '#!/usr/bin/env python3\nimport os,sys\nfrom pathlib import Path\nPath(os.environ["QGR_CONSOLE_LOGIN_URL"]).write_text(sys.argv[1])\n')
        script.chmod(0o700)
        url_file = self.work / "login-url"
        environment = dict(os.environ, PATH=str(opener) + os.pathsep + os.environ["PATH"],
                           QGR_CONSOLE_LOGIN_URL=str(url_file))
        process = subprocess.Popen(self.host + ["login", "--oauth-config", str(self.stack.args.state / "oauth.json"),
            "--acknowledge-recording"], env=environment, stdout=subprocess.PIPE, stderr=self.diagnostics)
        context = browser.new_context()
        try:
            wait(url_file.exists, "host CLI did not open its login flow", seconds=15)
            submit(context.new_page(), url_file.read_text(), "bob", password, approve=True)
            output, _ = process.communicate(timeout=25)
            require(process.returncode == 0, "real owner enrollment failed")
            self.machine = json.loads(output)["machine"]
        finally:
            context.close()
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
        self.started = True
        self.command(self.host + ["up", "--no-service"])
        wait(lambda: self.command(self.remote + ["browse", self.machine, str(self.root)]) is not None,
             "native connector did not become available", seconds=45)
        for marker in ("CONSOLE_STOP_TARGET", "CONSOLE_PRESERVE_TARGET"):
            result = self.command(self.remote + ["run", self.machine, "--cwd", str(self.root), "--",
                "/bin/sh", "-c", "stty -echo; printf '" + marker + "\\n'; while :; do sleep 1; done"])
            self.sessions.append(result["session"]["id"])
        require(self.sessions[0] != self.sessions[1], "two native launches shared a session identity")
        wait(lambda: all(self.state(s) == "running" for s in self.sessions),
             "owned native PTYs did not reach running state", seconds=15)

    def state(self, session_id):
        sessions = local_request(self.native, "list")
        return next((item["state"] for item in sessions if item["id"] == session_id), None)

    def cleanup(self):
        for session_id in self.sessions:
            if self.state(session_id) in ("running", "idle"):
                subprocess.run(self.base + ["terminate", session_id, "--yes"], stdout=subprocess.DEVNULL,
                               stderr=self.diagnostics, timeout=10)
        if self.started:
            subprocess.run(self.host + ["down", "--keep-sessions"], stdout=subprocess.DEVNULL,
                           stderr=self.diagnostics, timeout=10)
            subprocess.run(self.base + ["stop", "--terminate-sessions"], stdout=subprocess.DEVNULL,
                           stderr=self.diagnostics, timeout=10)
        self.token.unlink(missing_ok=True)
        self.diagnostics.close()


def owner_stop_proof(fixture, bob_page, alice_page, bob_identity, passed, work):
    ids = set(fixture.sessions)
    def projected():
        result = browser_request(bob_page, "/v1/me/sessions?limit=100")
        return result["status"] == 200 and ids <= {row["id"] for row in result["value"]["items"]}
    wait(projected, "native sessions did not project into owner console", seconds=35)
    own = browser_request(bob_page, "/v1/me/sessions?limit=100")["value"]["items"]
    require(all(row["owner"] == bob_identity["user"] for row in own), "owner session list leaked another owner")
    native_rows = [row for row in own if row["id"] in ids]
    require(len(native_rows) == 2 and all(row.get("model") is None for row in native_rows),
            "PTY session model metadata was invented")
    admin = browser_request(alice_page, "/v1/admin/users/" + bob_identity["user"])
    require(admin["status"] == 200 and ids <= {row["id"] for row in admin["value"]["sessions"]},
            "admin user drill-down did not show the real owner sessions")
    alice_own = browser_request(alice_page, "/v1/me/sessions?limit=100")["value"]["items"]
    require(not ids.intersection(row["id"] for row in alice_own),
            "admin role leaked another owner through the personal endpoint")
    require(browser_request(alice_page, "/v1/machines/" + fixture.machine)["status"] in (403, 404),
            "personal machine lookup accepted another owner")
    alice_page.bring_to_front()
    alice_page.locator('[data-view="admin"]').click()
    alice_page.locator('#nav [data-page="Users"]').click()
    alice_page.locator('[data-user="' + bob_identity["user"] + '"]').first.click()
    alice_page.locator('[data-device="' + fixture.machine + '"]').first.wait_for(state="visible", timeout=20000)
    require(fixture.machine_name in alice_page.locator("#content").inner_text(),
            "admin user UI omitted the real enrolled machine")
    require(alice_page.locator('button[data-stop]').count() == 0,
            "admin metadata UI offered another owner's native stop")
    alice_page.screenshot(path=str(work / "admin-user-devices.png"), full_page=True)
    passed("real owner/native inventory, admin user UI drill-down, and owner isolation")
    device_page = browser_request(bob_page, "/v1/me/machines?limit=100")
    require(device_page["status"] == 200, "owner device-list request failed")
    device = next((row for row in device_page["value"]["items"] if row["id"] == fixture.machine), None)
    require(device is not None and isinstance(device.get("last_connected_ms"), int)
            and device["last_connected_ms"] > 0,
            "owner device list omitted the durable last-connected activation timestamp")
    passed("owner device-list includes the native connector activation timestamp")
    bob_page.bring_to_front()
    bob_page.locator('#nav [data-page="My devices"]').click()
    target = bob_page.locator('[data-session-row="' + fixture.sessions[0] + '"]')
    target.wait_for(state="visible", timeout=20000)
    target.locator('[data-stop]').click()
    bob_page.locator("#stop-confirmation").fill("WRONG")
    require(not bob_page.locator("#stop-dialog form").evaluate("form => form.checkValidity()"),
            "wrong stop confirmation was valid")
    bob_page.locator("#stop-cancel").click()
    require(fixture.state(fixture.sessions[0]) == "running", "cancel stopped the native process")
    target.locator('[data-stop]').click()
    bob_page.locator("#stop-confirmation").fill("STOP")
    bob_page.locator("#stop-submit").click()
    wait(lambda: fixture.state(fixture.sessions[0]) == "exited",
         "browser stop did not terminate the selected native process", seconds=20)
    require(fixture.state(fixture.sessions[1]) == "running", "browser stop terminated unrelated work")
    wait(lambda: "Exited" in target.inner_text(),
         "console did not display the confirmed native session end", seconds=30)
    bob_page.screenshot(path=str(work / "owner-native-stop.png"), full_page=True)
    passed("actual browser WSS stop, cancelled confirmation, and unrelated native session survival")



def gateway_images(stack):
    """Record that every real gateway uses the selected built image."""
    image = stack.environment.get("QGR_GATEWAY_IMAGE") or "qgenie-remote-gateway:development"
    def inspect(*arguments):
        result = subprocess.run(stack.docker + list(arguments), env=stack.environment,
                                capture_output=True, timeout=15, check=False)
        require(result.returncode == 0, "gateway image inspection failed")
        return result.stdout.decode().strip()
    expected = inspect("image", "inspect", image, "--format", "{{.Id}}")
    actual = {}
    for node in (1, 2, 3, 4):
        stack.wait(lambda n=node: stack.probe(n), "selected gateway readiness", seconds=35)
        service = "gateway-" + str(node)
        container = stack.call("ps", "--quiet", service, capture=True).decode().strip()
        require(bool(container) and "\n" not in container, "gateway container identity was ambiguous")
        actual[service] = inspect("inspect", "--format", "{{.Image}}", container)
        require(actual[service] == expected, "gateway does not use the selected image")
    return expected, actual


def main():
    from playwright.sync_api import sync_playwright
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=ROOT / "target/console-stack")
    parser.add_argument("--project", default="qgr-console-e2e")
    parser.add_argument("--edge-port", type=int, default=8543)
    parser.add_argument("--identity-port", type=int, default=9543)
    parser.add_argument("--docker-host", default="unix:///var/run/docker.sock")
    parser.add_argument("--client-bin", type=Path, required=True)
    parser.add_argument("--chromium", default="/usr/bin/chromium")
    parser.add_argument("--install-browser-ca", action="store_true")
    parser.add_argument("--skip-gateway-failover", action="store_true")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    args.state, args.client_bin = args.state.resolve(), args.client_bin.resolve()
    args.identity = "keycloak"
    require(args.project != "qgr-local" and args.edge_port != 8443,
            "this proof requires an isolated stack and separate edge port")
    metadata = json.loads((args.state / "state.json").read_text())
    require(metadata["project"] == args.project and metadata["edge_port"] == args.edge_port
            and metadata["identity_port"] == args.identity_port and metadata["identity"] == "keycloak",
            "proof arguments do not match the initialized isolated stack")
    stack = Stack(args, metadata)
    work = args.state / ("console-proof-" + secrets.token_hex(4))
    work.mkdir(mode=0o700)
    destination = args.report or args.state / "last-browser-console-e2e.json"
    report = {"format": "qgr-browser-console-e2e-v1", "status": "running", "checks": [],
              "unrun": ["corporate Entra", "production fault domains", "macOS", "real model telemetry"]}
    began = time.monotonic()
    fixture = None
    def passed(name):
        report["checks"].append(name)
        print("PASS " + name, flush=True)
    try:
        report["stage"] = "identity and gateway readiness"
        stack.wait(lambda: ready(stack), "local identity readiness", seconds=90)
        origin = f"https://localhost:{args.edge_port}"
        ca = args.state / "certificates/ca.pem"
        credentials = json.loads((args.state / "users.json").read_text())
        wait(lambda: raw_request(origin, ca, "/v1/me")[0] == 401,
             "gateway owner API did not become ready and protected", seconds=45)
        require(raw_request(origin, ca, "/v1/admin/users")[0] == 401, "anonymous admin API request succeeded")
        passed("anonymous owner and admin API access denied")
        expected_image, actual_images = gateway_images(stack)
        report["expected_gateway_image"] = expected_image
        report["gateway_images"] = actual_images
        passed("all four isolated gateways run the selected built image")
        with browser_ca(ca, args.install_browser_ca), sync_playwright() as playwright:
            browser = playwright.chromium.launch(executable_path=args.chromium, headless=True,
                args=["--no-sandbox", "--disable-dev-shm-usage"])
            contexts = []
            try:
                report["stage"] = "browser sign-in and role checks"
                alice_context, alice, alice_identity = sign_in(browser, origin, "alice", credentials["alice"])
                contexts.append(alice_context)
                bob_context, bob, bob_identity = sign_in(browser, origin, "bob", credentials["bob"])
                contexts.append(bob_context)
                require(alice_identity["platform_admin"] is True and bob_identity["platform_admin"] is False,
                        "real local app-role mapping is incorrect")
                require(browser_request(bob, "/v1/admin/users")["status"] == 403,
                        "non-admin browser could read admin users")
                passed("real Keycloak browser PKCE, separate users, and platform-admin role enforcement")
                report["stage"] = "cookie CSRF and origin checks"
                browser_guards(origin, ca, bob_context, bob, bob_identity)
                passed("cookie protection, CSRF, cross-origin mutation and WebSocket denial, and no upstream token exposure")
                if not args.skip_gateway_failover:
                    report["stage"] = "cross-gateway browser session"
                    gateway_failover(stack, alice, alice_identity["user"])
                    passed("same encrypted browser session survives requests forced across different gateways")
                report["stage"] = "native enrollment and two PTY starts"
                fixture = NativeFixture(browser, stack, args.client_bin, credentials, work)
                report["stage"] = "owner UI stop and isolation"
                owner_stop_proof(fixture, bob, alice, bob_identity, passed, work)
                report["stage"] = "admin telemetry"
                alice.bring_to_front()
                alice.locator('[data-view="admin"]').click()
                alice.locator('#nav [data-page="Telemetry & signals"]').click()
                alice.locator("#content").wait_for(state="visible")
                alice.screenshot(path=str(work / "admin-telemetry.png"), full_page=True)
                result = browser_request(alice, "/v1/admin/telemetry/summary")
                require(result["status"] == 200, "real telemetry summary request failed")
                summary = result["value"]
                require(summary["enrolled_devices"] >= 1 and summary["confirmed_active_sessions"] >= 1,
                        "telemetry summary omitted the surviving native fixture")
                require(summary["usage"]["tokens"] is None,
                        "telemetry invented native model usage")
                require(not any(name in alice.locator("#content").inner_text()
                                for name in ("Alex Lee", "Sam Rivera", "Morgan Chen")),
                        "prototype fixture identities leaked into the production console")
                passed("real API-backed admin telemetry and explicit unavailable usage")
                report["stage"] = "pre-logout authenticated WebSocket connection"
                bob.bring_to_front()
                bob.evaluate("""async () => {
                    window.__consoleProofWire = new ConsoleWire();
                    await window.__consoleProofWire.connect();
                    window.__consoleProofClosed = false;
                    window.__consoleProofWire.socket.addEventListener('close', () => {
                        window.__consoleProofClosed = true;
                    });
                }""")
                report["stage"] = "browser logout commit"
                logout_began = time.monotonic()
                logout = browser_request(bob, "/console/auth/logout", method="POST",
                    headers={"X-CSRF-Token": bob_identity["csrf_token"]})
                require(logout["status"] in (200, 204), "valid browser logout failed")
                report["stage"] = "post-logout HTTP denial"
                require(browser_request(bob, "/v1/me")["status"] == 401,
                        "logged-out browser session retained API authority")
                report["stage"] = "post-logout WebSocket closure"
                wait(lambda: bob.evaluate("window.__consoleProofClosed === true"),
                     "logout did not close the already-open authenticated WebSocket", seconds=8)
                report["browser_ws_logout_close_ms"] = int((time.monotonic() - logout_began) * 1000)
                require(fixture.state(fixture.sessions[1]) == "running",
                        "browser logout terminated surviving native work")
                passed("browser logout removes API authority, closes existing WSS, and preserves native work")
            finally:
                if fixture is not None:
                    fixture.cleanup()
                for context in contexts:
                    context.close()
                browser.close()
        report["status"] = "passed"
        report["stage"] = "complete"
        return 0
    except BaseException as error:
        report["status"] = "failed"
        report["failure"] = {"kind": type(error).__name__,
            "check": str(error) if isinstance(error, ProofFailure) else "inspect the owned local fixture"}
        print("FAIL " + report["failure"]["check"], file=sys.stderr)
        return 1
    finally:
        report["elapsed_ms"] = int((time.monotonic() - began) * 1000)
        report["artifact_directory"] = str(work.relative_to(ROOT)) if work.is_relative_to(ROOT) else str(work)
        destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        destination.write_text(json.dumps(report, indent=2) + "\n")
        destination.chmod(0o600)


if __name__ == "__main__":
    sys.exit(main())
