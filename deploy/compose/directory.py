"""Development-only Graph shape adapter backed by Keycloak's authorized Admin API.

No stored credentials, enabled-state cache, password grant, or public listener.
Keycloak validates the forwarded service bearer and its read-user permission.
"""
import http.client
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import re
import socketserver
import threading
import urllib.parse


def account(body, expected_id):
    user = json.loads(body)
    if user.get("id") != expected_id or type(user.get("enabled")) is not bool:
        raise ValueError("invalid directory evidence")
    return {"id": expected_id, "accountEnabled": user["enabled"]}


def object_id(path):
    parsed = urllib.parse.urlsplit(path)
    match = re.fullmatch(r"/v1\.0/users/([0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12})", parsed.path)
    try:
        query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True, strict_parsing=True)
    except ValueError:
        return None
    if match and query == {"$select": ["id,accountEnabled"]}:
        return match[1]
    return None


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Never log authorization headers, URLs, or user identifiers.

    def reply(self, status, body=None):
        data = json.dumps(body).encode() if body is not None else b""
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/live":
            return self.reply(200, {"live": True})
        oid = object_id(self.path)
        if oid is None:
            # Only a real upstream missing user may become disabled evidence.
            return self.reply(400)
        bearer = self.headers.get("Authorization", "")
        if not bearer.startswith("Bearer ") or len(bearer) > 16384:
            return self.reply(401)
        upstream = http.client.HTTPConnection("keycloak", 8080, timeout=3)
        try:
            upstream.request("GET", "/admin/realms/qgr-local/users/" + oid, headers={"Authorization": bearer})
            response = upstream.getresponse()
            if response.status != 200:
                return self.reply(response.status if response.status in (401, 403, 404) else 503)
            body = response.read(65537)
            if len(body) > 65536:
                raise ValueError("oversize directory evidence")
            self.reply(200, account(body, oid))
        except (OSError, http.client.HTTPException, ValueError, AttributeError):
            self.reply(503)
        finally:
            upstream.close()


class Server(socketserver.ThreadingMixIn, HTTPServer):
    daemon_threads = True
    slots = threading.BoundedSemaphore(16)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(5)
        return connection, address

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()


if __name__ == "__main__":
    Server(("0.0.0.0", 8080), Handler).serve_forever()
