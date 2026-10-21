"""WebDriverAgent driver for Cove on a real iPhone

HTTP and session handling come from `wdalib`, the client behind the `ios-*` scripts in `$WDA_TOOLS`
(default `~/code/tools/wda`). It is imported on first use so the XML parser here can be tested
without those tools or a phone. Nothing here changes an OS setting: iOS rows that need one are not written
"""

import base64
import os
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

from common import Device, Node, RowBlocked, RowFailed, ensure_tap_allowed

# WDA reports `/wda/apps/state` 4 for an app running in the foreground
FOREGROUND = 4

# element types that only lay out other elements or paint the system chrome
LAYOUT_TYPES = {"XCUIElementTypeOther", "XCUIElementTypeWindow", "XCUIElementTypeStatusBar"}

# the application element spans the whole screen, so matching it would tap the middle of whatever app is in front
APPLICATION_TYPE = "XCUIElementTypeApplication"


def parse_source(xml: str, bundle_id: str = "") -> list[Node]:
    """Flatten a WDA `/source?format=xml` tree into visible nodes

    The label comes from the element's `label`, then `name`, then `value`; the accessibility
    identifier WDA reports as `name` is kept as `res_id` so rows can match either
    """
    root = ET.fromstring(xml)
    nodes = []

    for element in root.iter():
        if element.get("visible") != "true":
            continue

        try:
            x, y = int(float(element.get("x", ""))), int(float(element.get("y", "")))
            width, height = int(float(element.get("width", ""))), int(float(element.get("height", "")))
        except ValueError:
            continue

        if width <= 0 or height <= 0:
            continue

        label, name, value = element.get("label") or "", element.get("name") or "", element.get("value") or ""
        kind = element.get("type", element.tag)
        if kind == APPLICATION_TYPE:
            continue


        # an unlabelled container is only structure, not something a row can read or tap
        if kind in LAYOUT_TYPES and not (label or name or value):
            continue

        nodes.append(
            Node(
                text=label or name or value,
                desc=value if value != (label or name) else "",
                res_id=name,
                clazz=kind,
                bounds=(x, y, x + width, y + height),
                package=bundle_id,
            )
        )

    return nodes


class Ios(Device):
    """The one iPhone WDA is attached to; WDA decides which device that is, so there is no serial to pass"""

    def __init__(self, bundle_id: str, out_dir: Path | None = None, loops: int = 10):
        self.package = bundle_id
        self.out_dir = out_dir
        self.loops = loops
        self.upgrade_apk = None
        self._wda = None

    @property
    def wda(self):
        if self._wda is None:
            tools = Path(os.environ.get("WDA_TOOLS", "~/code/tools/wda")).expanduser()
            if not (tools / "wdalib.py").is_file():
                raise RowBlocked(f"wdalib.py not found in {tools}; set WDA_TOOLS to the WebDriverAgent tools directory")

            sys.path.insert(0, str(tools))
            import wdalib

            self._wda = wdalib

        return self._wda

    def request(self, method: str, path: str, body: dict | None = None, timeout: float = 60) -> dict:
        try:
            return self.wda.http(method, path, body, timeout)
        except self.wda.Fail as error:
            raise RuntimeError(f"WDA {method} {path}: {error.message}") from error

    def session_post(self, path: str, body: dict | None = None) -> dict:
        try:
            return self.wda.post(path, body if body is not None else {})
        except self.wda.Fail as error:
            raise RuntimeError(f"WDA POST {path}: {error.message}") from error

    # app lifecycle

    def ensure_installed(self) -> None:
        # WDA has no install query, so a missing app shows up as a failed launch instead
        self.request("GET", "/status")

    def launch(self) -> None:
        self.session_post("/wda/apps/launch", {"bundleId": self.package})

    def force_stop(self) -> None:
        self.session_post("/wda/apps/terminate", {"bundleId": self.package})

    def cold_restart(self) -> None:
        self.force_stop()
        self.wait_until(lambda: self.app_state() != FOREGROUND, "app to exit", timeout=10)
        self.launch()

    def background(self) -> None:
        self.request("POST", "/wda/homescreen", {})
        self.wait_until(lambda: not self.is_foreground(), "app to leave the foreground", timeout=10)

    def foreground(self) -> None:
        # activate resumes the running app instead of relaunching it
        self.session_post("/wda/apps/activate", {"bundleId": self.package})
        self.wait_until(self.is_foreground, "app to return to the foreground", timeout=15)

    def app_state(self) -> int:
        return int(self.session_post("/wda/apps/state", {"bundleId": self.package}).get("value", 0))

    def is_foreground(self) -> bool:
        return self.app_state() == FOREGROUND

    def pid(self) -> str | None:
        # WDA does not expose the process id
        return None

    def clear_crash_log(self) -> None:
        pass

    def crashes(self) -> list[str]:
        # crash reports live on the phone and WDA can't read them, so a crash shows up as the app leaving the foreground
        return []

    # screen

    def ensure_app_in_front(self) -> None:
        """Fail unless this app is the one in front

        WDA reads and taps whatever app is in front, and another app can come forward mid-row
        (a system prompt, or another client driving the same phone), so a label found there must
        never be tapped
        """
        bundle_id = self.request("GET", "/wda/activeAppInfo").get("value", {}).get("bundleId")
        if bundle_id != self.package:
            raise RowFailed(f"{bundle_id or 'another app'} is in front instead of {self.package}")

    def nodes(self) -> list[Node]:
        self.ensure_app_in_front()
        return parse_source(self.request("GET", "/source?format=xml")["value"], self.package)

    def app_nodes(self) -> list[Node]:
        # nodes() only reads the tree while this app is in front
        return self.nodes()

    def tap(self, node: Node) -> None:
        ensure_tap_allowed(node.label)

        # the tree was read earlier, so check again right before touching the screen
        self.ensure_app_in_front()
        x, y = node.center
        self.session_post("/wda/tap", {"x": x, "y": y})

    def back(self) -> None:
        self.tap_label(r"^(Back|BackButton)$", timeout=5)

    def screen_size(self) -> tuple[int, int]:
        width, height = self.wda.window_size()
        return int(width), int(height)

    def scroll_down(self) -> None:
        width, height = self.screen_size()
        self.session_post(
            "/wda/dragfromtoforduration",
            {"fromX": width / 2, "fromY": height * 0.75, "toX": width / 2, "toY": height * 0.25, "duration": 0.3},
        )

    def screenshot(self, path: str) -> None:
        data = self.request("GET", "/screenshot")["value"]
        Path(path).write_bytes(base64.b64decode(data))
