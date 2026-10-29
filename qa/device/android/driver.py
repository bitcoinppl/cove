"""adb and uiautomator driver for Cove on an Android device or emulator"""

import contextlib
import os
import re
import shutil
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path

from common import Device, Node, RowBlocked, RowFailed, ensure_tap_allowed

BOUNDS = re.compile(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]")

# the `package:` line `aapt2 dump badging` prints for an apk
BADGING_PACKAGE = re.compile(r"^package: name='([^']+)' versionCode='(\d+)'", re.MULTILINE)


def parse_hierarchy(xml: str) -> list[Node]:
    """Flatten a `uiautomator dump` hierarchy into visible nodes"""
    start = xml.find("<?xml")
    root = ET.fromstring(xml[start:] if start >= 0 else xml)
    nodes = []

    for element in root.iter("node"):
        match = BOUNDS.match(element.get("bounds", ""))
        if not match or element.get("visible-to-user", "true") != "true":
            continue

        nodes.append(
            Node(
                text=element.get("text", ""),
                desc=element.get("content-desc", ""),
                res_id=element.get("resource-id", ""),
                clazz=element.get("class", ""),
                bounds=tuple(int(value) for value in match.groups()),
                package=element.get("package", ""),
                enabled=element.get("enabled", "true") == "true",
                # compose marks a lazy list scrollable only while its items overflow the viewport
                scrollable=element.get("scrollable", "false") == "true",
            )
        )

    return nodes


def parse_badging(badging: str) -> tuple[str, int]:
    """Package name and versionCode from `aapt2 dump badging` output"""
    match = BADGING_PACKAGE.search(badging)
    if not match:
        raise RuntimeError("aapt2 dump badging printed no package line")

    return match.group(1), int(match.group(2))


def find_aapt2() -> Path:
    """The newest `aapt2` in the Android SDK, found from ANDROID_HOME, ANDROID_SDK_ROOT or the adb on PATH"""
    adb = shutil.which("adb")
    roots = [os.environ.get("ANDROID_HOME"), os.environ.get("ANDROID_SDK_ROOT"), adb and str(Path(adb).resolve().parents[1])]

    for root in filter(None, roots):
        tools = sorted(Path(root).glob("build-tools/*/aapt2"), key=lambda path: [int(part) for part in re.findall(r"\d+", path.parent.name)])
        if tools:
            return tools[-1]

    raise RowBlocked("aapt2 not found; set ANDROID_HOME to an Android SDK with build-tools so the apk can be read")


class Android(Device):
    """One device, addressed by serial so a second attached phone is never touched"""

    def __init__(
        self,
        serial: str,
        package: str,
        upgrade_apk: Path | None = None,
        out_dir: Path | None = None,
        loops: int = 10,
    ):
        self.serial = serial
        self.package = package
        # apk installed over the existing install by the upgrade row
        self.upgrade_apk = upgrade_apk
        # where rows leave failure screenshots next to their result json
        self.out_dir = out_dir
        # repetitions for rows that loop, such as repeated background and foreground
        self.loops = loops

    def adb(self, *args: str, timeout: float = 60) -> str:
        result = subprocess.run(
            ["adb", "-s", self.serial, *args], capture_output=True, text=True, timeout=timeout
        )

        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip()
            raise RuntimeError(f"adb {' '.join(args)} failed: {detail}")

        return result.stdout

    def shell(self, *args: str, timeout: float = 60) -> str:
        return self.adb("shell", *args, timeout=timeout).strip()

    # app lifecycle

    def ensure_installed(self) -> None:
        if f"package:{self.package}" not in self.shell("pm", "list", "packages", self.package).split():
            raise RowBlocked(f"{self.package} is not installed on {self.serial}")

    def apk_package_and_version(self, apk: Path) -> tuple[str, int]:
        result = subprocess.run([str(find_aapt2()), "dump", "badging", str(apk)], capture_output=True, text=True, timeout=60)
        if result.returncode != 0:
            raise RowBlocked(f"aapt2 could not read {apk}: {result.stderr.strip()}")

        return parse_badging(result.stdout)

    def installed_version_code(self) -> int:
        match = re.search(r"versionCode=(\d+)", self.shell("dumpsys", "package", self.package))
        if not match:
            raise RuntimeError(f"dumpsys package {self.package} shows no versionCode")

        return int(match.group(1))

    def install_over_existing(self, apk: Path) -> None:
        # -r keeps app data, which is what makes this an upgrade and not a fresh install
        self.adb("install", "-r", str(apk), timeout=600)

    def launch(self) -> None:
        self.shell("monkey", "-p", self.package, "-c", "android.intent.category.LAUNCHER", "1")

    def force_stop(self) -> None:
        self.shell("am", "force-stop", self.package)

    def cold_restart(self) -> None:
        self.force_stop()
        self.wait_until(lambda: self.pid() is None, "app process to exit", timeout=10)
        self.launch()

    def background(self) -> None:
        self.shell("input", "keyevent", "KEYCODE_HOME")
        self.wait_until(lambda: not self.is_foreground(), "app to leave the foreground", timeout=10)

    def foreground(self) -> None:
        # the launcher intent resumes the existing task instead of starting a new one
        self.launch()
        self.wait_until(self.is_foreground, "app to return to the foreground", timeout=15)

    def pid(self) -> str | None:
        result = subprocess.run(
            ["adb", "-s", self.serial, "shell", "pidof", self.package], capture_output=True, text=True
        )

        return result.stdout.strip() or None

    def is_foreground(self) -> bool:
        return self.package in self.shell("dumpsys", "activity", "activities").split("ResumedActivity", 1)[-1][:300]

    def clear_crash_log(self) -> None:
        self.adb("logcat", "-b", "crash", "-c")

    def crashes(self) -> list[str]:
        log = self.adb("logcat", "-b", "crash", "-d")
        return [line for line in log.splitlines() if self.package in line]

    # screen

    def nodes(self) -> list[Node]:
        # uiautomator reports a null root while the window is mid-transition, which clears within a moment
        for attempt in range(4):
            try:
                self.shell("uiautomator", "dump", "/sdcard/cove-qa-window.xml")
                break
            except RuntimeError:
                if attempt == 3:
                    raise
                time.sleep(1)

        return parse_hierarchy(self.adb("exec-out", "cat", "/sdcard/cove-qa-window.xml"))

    def app_nodes(self) -> list[Node]:
        return [node for node in self.nodes() if node.package == self.package]

    def tap(self, node: Node) -> None:
        ensure_tap_allowed(node.label)
        x, y = node.center
        self.shell("input", "tap", str(x), str(y))

    def back(self) -> None:
        self.shell("input", "keyevent", "KEYCODE_BACK")

    def screen_size(self) -> tuple[int, int]:
        match = re.search(r"(\d+)x(\d+)", self.shell("wm", "size").splitlines()[-1])
        if not match:
            raise RuntimeError("could not read the screen size")

        return int(match.group(1)), int(match.group(2))

    def scroll_down(self) -> None:
        """Drag the middle of the screen upward so content further down comes into view"""
        width, height = self.screen_size()
        x = width // 2
        self.shell("input", "swipe", str(x), str(height * 3 // 4), str(x), str(height // 4), "400")

    def swipe(self, x1: int, y1: int, x2: int, y2: int) -> None:
        self.shell("input", "swipe", str(x1), str(y1), str(x2), str(y2), "300")

    def screenshot(self, path: str) -> None:
        # another app's screen can hold private content, and failure screenshots get uploaded
        if not self.is_foreground():
            raise RowFailed(f"{self.package} is not in front, so no screenshot was taken")

        with open(path, "wb") as file:
            file.write(subprocess.run(["adb", "-s", self.serial, "exec-out", "screencap", "-p"], capture_output=True, check=True).stdout)

    # os settings, each restored to its original value afterwards

    @contextlib.contextmanager
    def setting(self, namespace: str, key: str, value: str):
        original = self.shell("settings", "get", namespace, key)
        self.shell("settings", "put", namespace, key, value)

        try:
            yield
        finally:
            if original == "null":
                self.shell("settings", "delete", namespace, key)
            else:
                self.shell("settings", "put", namespace, key, original)

    @contextlib.contextmanager
    def animations_off(self):
        with contextlib.ExitStack() as stack:
            for key in ("window_animation_scale", "transition_animation_scale", "animator_duration_scale"):
                stack.enter_context(self.setting("global", key, "0"))
            yield

    @contextlib.contextmanager
    def offline(self):
        was_on = self.shell("settings", "get", "global", "airplane_mode_on") == "1"
        self.shell("cmd", "connectivity", "airplane-mode", "enable")

        try:
            yield
        finally:
            if not was_on:
                self.shell("cmd", "connectivity", "airplane-mode", "disable")
