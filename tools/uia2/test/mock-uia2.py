"""Mock UiAutomator2 server for exercising the android-* scripts without a device

usage: mock-uia2.py PORT REQUEST_LOG
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, LOG = int(sys.argv[1]), sys.argv[2]
SID = "mock-session"
# a real 1x1 PNG so sips can resize it
PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="

# a wallet screen with a settings-style switch row, under a PIN pad drawn on top of it
SOURCE = """<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy index="0" class="hierarchy" width="1080" height="2400">
  <android.widget.FrameLayout class="android.widget.FrameLayout" resource-id="screen.root" text="" content-desc="" clickable="false" displayed="true" bounds="[0,0][1080,2400]">
    <android.view.View class="android.view.View" resource-id="" text="" content-desc="" clickable="true" displayed="true" bounds="[60,500][1020,650]">
      <android.widget.TextView class="android.widget.TextView" resource-id="" text="Sent" content-desc="" clickable="false" displayed="true" bounds="[100,520][300,570]"/>
      <android.widget.TextView class="android.widget.TextView" resource-id="" text="-209 SATS" content-desc="" clickable="false" displayed="true" bounds="[700,520][1000,570]"/>
    </android.view.View>
    <android.widget.TextView class="android.widget.TextView" resource-id="" text="Enable PIN" content-desc="" clickable="false" displayed="true" bounds="[60,700][500,760]"/>
    <android.view.View class="android.view.View" resource-id="" text="" content-desc="" clickable="true" checked="true" displayed="true" bounds="[880,680][1020,780]"/>
    <android.widget.TextView class="android.widget.TextView" resource-id="" text="About" content-desc="" clickable="false" displayed="true" bounds="[60,900][300,960]"/>
    <android.view.View class="android.view.View" resource-id="" text="" content-desc="" clickable="true" displayed="true" bounds="[60,1000][1020,1100]">
      %s
    </android.view.View>
    <android.view.View class="android.view.View" resource-id="numberPadPin.viewport" text="" content-desc="" clickable="false" displayed="true" bounds="[0,0][1080,2400]">
      <android.widget.TextView class="android.widget.TextView" resource-id="" text="Enter Cove PIN" content-desc="" clickable="false" displayed="true" bounds="[60,300][1020,400]"/>
      <android.view.View class="android.view.View" resource-id="numberPadPin.digit.1" text="" content-desc="" clickable="true" displayed="true" bounds="[48,1356][360,1575]"/>
      <android.view.View class="android.view.View" resource-id="numberPadPin.digit.2" text="" content-desc="" clickable="true" displayed="true" bounds="[384,1356][696,1575]"/>
    </android.view.View>
  </android.widget.FrameLayout>
</hierarchy>"""


# enough lines that android-vis fills a pipe whose reader quit early
FILLER = "\n      ".join('<android.widget.TextView class="android.widget.TextView" resource-id="" text="Filler %d" content-desc="" '
                         'clickable="true" displayed="true" bounds="[60,1010][1020,1090]"/>' % i for i in range(1500))
SOURCE = SOURCE % FILLER

# routes the scripts may call; anything else is a 404, so a wrong path fails its test
GETS = {"/window/current/size", "/element/active", "/element/el-focused/text", "/element/el-1/screenshot",
        "/source", "/screenshot", "/appium/device/info", ""}
POSTS = {"/appium/settings", "/elements", "/element/el-1/click", "/element/el-2/click", "/actions",
         "/appium/gestures/click", "/appium/gestures/long_click", "/back", "/appium/device/press_keycode",
         "/keys", "/appium/device/set_clipboard", "/appium/device/get_clipboard"}


def source():
    # a test can swap the screen by writing REQUEST_LOG.source
    try:
        with open(LOG + ".source") as f:
            return f.read()
    except OSError:
        return SOURCE


# the focused field's text, so android-type's readback reflects what was sent
FIELD = {"text": ""}


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
            # a test can report another server version by writing REQUEST_LOG.version
            try:
                with open(LOG + ".version") as f:
                    version = f.read().strip()
            except FileNotFoundError:
                version = "10.6.6"
            return self.reply(200, {"ready": True, "build": {"version": version}})
        if p == "/session":
            return self.reply(200, {"sessionId": SID, "capabilities": {}})
        if not p.startswith("/session/" + SID):
            return self.reply(404, {"error": "invalid session id", "message": "unknown session"})
        rest = p[len("/session/" + SID):]
        if rest not in (GETS if method == "GET" else POSTS):
            return self.reply(404, {"error": "unknown command", "message": "%s %s is not a server route" % (method, rest)})
        if rest == "/window/current/size":
            return self.reply(200, {"width": 1080, "height": 2400})
        if rest == "/elements":
            sel = body["selector"]
            if 'text("Send")' in sel:
                return self.reply(200, [{"ELEMENT": "el-1"}, {"ELEMENT": "el-2"}])
            hit = ('text("About")' in sel or 'description("Back")' in sel or 'resourceId("numberPadPin.digit.1")' in sel)
            return self.reply(200, [{"ELEMENT": "el-1"}] if hit else [])
        if rest == "/element/active":
            return self.reply(200, {"ELEMENT": "el-focused"})
        if rest == "/keys":
            text = body["text"]
            if text.startswith("FAIL"):
                # the real server quotes the submitted text in this error
                return self.reply(500, {"error": "invalid element state", "message": "Cannot set the element to '%s'." % text})
            # a field with a length limit keeps only part of what was typed
            FIELD["text"] = text[:4] if text.startswith("trunc") else text
            return self.reply(200, None)
        if rest == "/element/el-focused/text":
            return self.reply(200, FIELD["text"])
        if rest == "/element/el-1/screenshot":
            return self.reply(500, {"error": "unable to capture screen", "message": "Failed to capture a screenshot. Does the current view have 'secure' flag set?"})
        if rest == "/source":
            return self.reply(200, source())
        if rest == "/screenshot":
            return self.reply(200, PNG)
        if rest == "/appium/device/get_clipboard":
            return self.reply(200, "")
        if rest == "/appium/device/info":
            return self.reply(200, {"apiVersion": "33"})
        return self.reply(200, None)

    def do_GET(self):
        self.handle_any("GET")

    def do_POST(self):
        self.handle_any("POST")


HTTPServer(("127.0.0.1", PORT), H).serve_forever()
