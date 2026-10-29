"""Offline checks for the device-script helpers; run with `python3 -I -m unittest discover qa/device`"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from android.driver import parse_badging, parse_hierarchy  # noqa: E402
from common import Device, ForbiddenTap, Node, RowBlocked, RowFailed, ensure_tap_allowed  # noqa: E402
from ios.driver import Ios, parse_source  # noqa: E402
from screens import error_text, row_registry, wait_for_scan_to_start  # noqa: E402

DUMP = """UI hierchary dumped to: /dev/tty<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node text="" resource-id="" class="android.widget.FrameLayout" package="org.bitcoinppl.cove.dev"
        content-desc="" visible-to-user="true" bounds="[0,0][1080,2400]">
    <node text="Signet" resource-id="settings.network" class="android.widget.TextView"
          package="org.bitcoinppl.cove.dev" content-desc="" visible-to-user="true" bounds="[40,300][540,380]" />
    <node text="" resource-id="" class="android.view.View" package="org.bitcoinppl.cove.dev"
          content-desc="Open menu" visible-to-user="true" bounds="[0,100][120,220]" />
    <node text="Hidden" resource-id="" class="android.widget.TextView" package="org.bitcoinppl.cove.dev"
          content-desc="" visible-to-user="false" bounds="[0,0][10,10]" />
    <node text="Copy Address" resource-id="" class="android.widget.Button" package="org.bitcoinppl.cove.dev"
          content-desc="" enabled="false" visible-to-user="true" bounds="[40,900][1040,1000]" />
  </node>
</hierarchy>"""

WDA_SOURCE = """<?xml version="1.0" encoding="UTF-8"?>
<AppiumAUT>
  <XCUIElementTypeApplication type="XCUIElementTypeApplication" name="Cove" label="Cove" enabled="true" visible="true" x="0" y="0" width="402" height="874">
    <XCUIElementTypeOther type="XCUIElementTypeOther" enabled="true" visible="true" x="0" y="0" width="402" height="874">
      <XCUIElementTypeButton type="XCUIElementTypeButton" name="app.sidebar.open" label="Menu" enabled="true" visible="true" x="16" y="62" width="44" height="44"/>
      <XCUIElementTypeStaticText type="XCUIElementTypeStaticText" value="Receive" name="Receive" label="Receive" enabled="true" visible="true" x="220" y="300" width="160" height="50"/>
      <XCUIElementTypeStaticText type="XCUIElementTypeStaticText" value="42 sats" enabled="true" visible="true" x="20" y="400" width="100" height="20"/>
      <XCUIElementTypeStaticText type="XCUIElementTypeStaticText" label="Offscreen" enabled="true" visible="false" x="0" y="2000" width="100" height="20"/>
      <XCUIElementTypeStaticText type="XCUIElementTypeStaticText" label="Empty frame" enabled="true" visible="true" x="0" y="0" width="0" height="0"/>
    </XCUIElementTypeOther>
  </XCUIElementTypeApplication>
</AppiumAUT>"""


class WdaSourceTest(unittest.TestCase):
    def test_parses_visible_labelled_elements_with_bounds_and_identifiers(self):
        nodes = parse_source(WDA_SOURCE, "org.bitcoinppl.cove")

        self.assertEqual([node.label for node in nodes], ["Menu", "Receive", "42 sats"])
        self.assertEqual(nodes[0].res_id, "app.sidebar.open")
        self.assertEqual(nodes[1].bounds, (220, 300, 380, 350))
        self.assertEqual(nodes[1].center, (300, 325))
        self.assertEqual({node.package for node in nodes}, {"org.bitcoinppl.cove"})


class FakeWda(Ios):
    """An Ios driver whose WDA answers that `front` is the app in front and records taps"""

    def __init__(self, front: str):
        super().__init__("org.bitcoinppl.cove")
        self.front = front
        self.posted = []

    def request(self, method, path, body=None, timeout=60):
        if path == "/wda/activeAppInfo":
            return {"value": {"bundleId": self.front}}

        return {"value": WDA_SOURCE}

    def session_post(self, path, body=None):
        self.posted.append(path)
        return {}


class IosFrontAppTest(unittest.TestCase):
    def test_refuses_to_read_or_tap_when_another_app_is_in_front(self):
        driver = FakeWda("com.apple.Preferences")
        receive = parse_source(WDA_SOURCE, "org.bitcoinppl.cove")[1]

        with self.assertRaises(RowFailed):
            driver.nodes()
        with self.assertRaises(RowFailed):
            driver.tap(receive)
        self.assertEqual(driver.posted, [])

    def test_refuses_to_screenshot_another_app(self):
        driver = FakeWda("com.apple.MobileSMS")

        with tempfile.TemporaryDirectory() as out, self.assertRaises(RowFailed):
            driver.screenshot(str(Path(out) / "shot.png"))

    def test_taps_when_the_app_is_in_front(self):
        driver = FakeWda("org.bitcoinppl.cove")
        driver.tap(driver.find(r"^Receive$"))

        self.assertEqual(driver.posted, ["/wda/tap"])


class FakeScreen(Device):
    def __init__(self, nodes):
        self._nodes = nodes
        self.tapped = []

    def nodes(self):
        return self._nodes

    def app_nodes(self):
        return self._nodes

    def tap(self, node):
        ensure_tap_allowed(node.label)
        self.tapped.append(node.label)


class FindTest(unittest.TestCase):
    def test_matches_a_label_or_an_identifier_and_taps_through_the_guard(self):
        screen = FakeScreen(parse_source(WDA_SOURCE, "app"))

        self.assertEqual(screen.find(r"^app\.sidebar\.open$").label, "Menu")
        self.assertIsNone(screen.find(r"^Missing$"))

        screen.tap_label(r"^Receive$")
        self.assertEqual(screen.tapped, ["Receive"])

        screen._nodes = parse_source(WDA_SOURCE.replace('label="Receive"', 'label="Send"').replace('name="Receive"', 'name="Send"').replace('value="Receive"', 'value="Send"'), "app")
        with self.assertRaises(ForbiddenTap):
            screen.tap_label(r"^Send$")


def text_nodes(*texts):
    return [Node(text, "", "", "", (0, index * 50, 400, index * 50 + 40), "app") for index, text in enumerate(texts)]


class ScreenReadTest(unittest.TestCase):
    def test_finds_the_node_connection_alert_whatever_its_case(self):
        screen = FakeScreen(text_nodes("Receive", "Node Connection Failed", "OK"))

        self.assertEqual(error_text(screen), "Node Connection Failed")

    def test_a_scan_that_never_shows_blocks_the_row(self):
        screen = FakeScreen(text_nodes("Receive", "Transactions"))

        with self.assertRaises(RowBlocked):
            wait_for_scan_to_start(screen, {}, timeout=0.6)

        screen._nodes = text_nodes("Receive", "Transactions", "Scanning for transactions...")
        evidence = {}
        wait_for_scan_to_start(screen, evidence, timeout=0.6)
        self.assertIn("scan_start_seconds", evidence)


class RowRegistryTest(unittest.TestCase):
    class Driver:
        out_dir = None
        package = "app"

        def __init__(self, crashes):
            self._crashes = crashes

        def clear_crash_log(self):
            pass

        def crashes(self):
            return self._crashes

    def test_a_crash_during_a_row_fails_it_and_is_recorded(self):
        rows = {}
        row = row_registry("android", rows, lambda driver: None)

        @row("some-row")
        def body(driver, evidence):
            pass

        evidence = {}
        with self.assertRaisesRegex(RowFailed, "app crashed: FATAL"):
            rows["some-row"](self.Driver(["FATAL EXCEPTION app"]), evidence)
        self.assertEqual(evidence["crashes"], ["FATAL EXCEPTION app"])

        rows["some-row"](self.Driver([]), {})


class TapGuardTest(unittest.TestCase):
    def test_refuses_controls_that_move_funds_or_destroy_data(self):
        for label in ["Send", "Delete Wallet", "Export Labels", "Wipe Data", "Sign Out", "Share", "Enable Cloud Backup", "Broadcast", "End Session"]:
            with self.subTest(label=label), self.assertRaises(ForbiddenTap):
                ensure_tap_allowed(label)

    def test_allows_navigation_and_read_only_controls(self):
        for label in ["Signet", "Settings", "Network", "Receive", "Transactions", "Open menu", "Back"]:
            with self.subTest(label=label):
                ensure_tap_allowed(label)


class HierarchyTest(unittest.TestCase):
    def test_parses_visible_nodes_with_labels_and_centres(self):
        nodes = parse_hierarchy(DUMP)
        labels = [node.label for node in nodes]

        self.assertEqual(labels, ["", "Signet", "Open menu", "Copy Address"])
        self.assertEqual(nodes[1].center, (290, 340))
        self.assertEqual(nodes[2].desc, "Open menu")
        self.assertEqual([node.enabled for node in nodes], [True, True, True, False])

    def test_reads_the_package_and_version_code_from_badging(self):
        badging = "package: name='org.bitcoinppl.cove.dev' versionCode='42' versionName='1.4.0' platformBuildVersionName='15'\nsdkVersion:'26'\n"

        self.assertEqual(parse_badging(badging), ("org.bitcoinppl.cove.dev", 42))


class CommandLineTest(unittest.TestCase):
    def test_rejects_a_loop_count_below_one(self):
        run = Path(__file__).resolve().parent / "run.py"
        result = subprocess.run([sys.executable, "-I", str(run), "ios", "--loops", "0", "--list"], capture_output=True, text=True)

        self.assertEqual(result.returncode, 2)
        self.assertIn("--loops", result.stderr)


if __name__ == "__main__":
    unittest.main()
