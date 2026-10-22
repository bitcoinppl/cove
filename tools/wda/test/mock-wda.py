"""Mock WDA HTTP server for exercising the ios-* scripts without a device

usage: mock-wda.py PORT REQUEST_LOG
"""
import base64
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, LOG = int(sys.argv[1]), sys.argv[2]
SID = "mock-session"
# one visible label, one hidden button with a separate accessibility id, one disabled field
TREE = (
    '<XCUIElementTypeApplication type="XCUIElementTypeApplication" name="Cove" label="Cove" visible="true">'
    '<XCUIElementTypeStaticText type="XCUIElementTypeStaticText" label="About" visible="true" enabled="true" x="20" y="100" width="80" height="20"/>'
    '<XCUIElementTypeButton type="XCUIElementTypeButton" name="BackButton" label="Back" visible="false" enabled="true" x="0" y="40" width="44" height="44"/>'
    '<XCUIElementTypeTextField type="XCUIElementTypeTextField" name="amount" value="0" visible="true" enabled="false" x="10" y="200" width="300" height="40"/>'
    '</XCUIElementTypeApplication>'
)
PNG = base64.b64encode(b"\x89PNG\r\n\x1a\nmock").decode()


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def reply(self, code, value):
        body = json.dumps({"value": value, "sessionId": SID}).encode()
        self.send_response(code)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def handle_any(self, method):
        n = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(n)) if n else None
        with open(LOG, "a") as f:
            f.write(json.dumps({"method": method, "path": self.path, "body": body}) + "\n")
        p = self.path
        if p == "/status":
            return self.reply(200, {"ready": True})
        if p == "/session":
            return self.reply(200, {"sessionId": SID, "capabilities": {}})
        if p.startswith("/source"):
            return self.reply(200, TREE if "xml" in p else {"type": "Application"})
        if p == "/screenshot":
            return self.reply(200, PNG)
        if not p.startswith("/session/" + SID):
            return self.reply(404, {"error": "invalid session id", "message": "unknown session"})
        rest = p[len("/session/" + SID):]
        if rest == "/window/size":
            return self.reply(200, {"width": 375, "height": 667})
        if rest == "/element":
            if body["value"] == "About" or (body["using"] == "predicate string" and 'label == "Back"' in body["value"]):
                return self.reply(200, {"ELEMENT": "el-1"})
            return self.reply(404, {"error": "no such element", "message": "not found"})
        if rest == "/wda/getPasteboard":
            return self.reply(200, base64.b64encode(b"mock clip").decode())
        if rest.startswith("/source"):
            return self.reply(200, TREE if "xml" in rest else {"type": "Application"})
        if rest == "/screenshot" or p == "/screenshot":
            return self.reply(200, PNG)
        return self.reply(200, None)

    def do_GET(self):
        self.handle_any("GET")

    def do_POST(self):
        self.handle_any("POST")


HTTPServer(("127.0.0.1", PORT), H).serve_forever()
