"""Shared pieces for the release-checklist device scripts: results, checks and the tap guard"""

import json
import re
import time
import traceback
from dataclasses import dataclass, field
from pathlib import Path

# labels a script must never tap: these send, delete, export, wipe, sign out or touch a cloud backup
FORBIDDEN_LABEL = re.compile(
    r"\b(send|broadcast|sign|delete|remove|erase|wipe|reset|export|share|backup|back up|restore|"
    r"sign out|log out|disable|enable cloud|end session)\b",
    re.IGNORECASE,
)

# networks a script may run against; anything else fails the row before it starts
ALLOWED_NETWORKS = ("signet", "testnet", "testnet4", "testnet 4")


class RowFailed(Exception):
    """An assertion about the app failed"""


class RowBlocked(Exception):
    """The device or fixture isn't in a state the row can run from, so the result says nothing about the app"""


class ForbiddenTap(Exception):
    """A script tried to tap a control that could move funds or destroy data"""


def ensure_tap_allowed(label: str) -> None:
    if FORBIDDEN_LABEL.search(label):
        raise ForbiddenTap(f"refusing to tap {label!r}")


def check(condition: bool, message: str) -> None:
    if not condition:
        raise RowFailed(message)


@dataclass(frozen=True)
class Node:
    """One visible element of the screen, in the same shape for both platforms

    `text` is the visible string, `desc` the accessibility description and `res_id` the stable
    identifier (an Android resource id or test tag, an iOS accessibility identifier)
    """

    text: str
    desc: str
    res_id: str
    clazz: str
    bounds: tuple[int, int, int, int]
    package: str

    @property
    def label(self) -> str:
        return self.text or self.desc or self.res_id

    @property
    def center(self) -> tuple[int, int]:
        left, top, right, bottom = self.bounds
        return (left + right) // 2, (top + bottom) // 2


class Device:
    """Screen helpers shared by the drivers; a subclass supplies `nodes()` and `tap(node)`"""

    def nodes(self) -> list[Node]:
        raise NotImplementedError

    def tap(self, node: Node) -> None:
        raise NotImplementedError

    def find(self, pattern: str, nodes: list[Node] | None = None) -> Node | None:
        """First node whose label or identifier matches the regex"""
        regex = re.compile(pattern)
        candidates = self.nodes() if nodes is None else nodes

        return next((node for node in candidates if regex.search(node.label) or regex.search(node.res_id)), None)

    def wait_until(self, predicate, what: str, timeout: float = 20) -> None:
        deadline = time.time() + timeout
        while time.time() < deadline:
            if predicate():
                return
            time.sleep(0.5)

        raise RowFailed(f"timed out after {timeout}s waiting for {what}")

    def wait_for(self, pattern: str, timeout: float = 20) -> Node:
        found: list[Node] = []

        def present() -> bool:
            node = self.find(pattern)
            if node:
                found.append(node)

            return node is not None

        self.wait_until(present, repr(pattern), timeout)

        return found[-1]

    def tap_label(self, pattern: str, timeout: float = 20) -> Node:
        """Wait for a control and tap it; the driver's `tap` refuses controls that could move funds or destroy data"""
        node = self.wait_for(pattern, timeout)
        self.tap(node)

        return node


@dataclass
class RowResult:
    row: str
    platform: str
    status: str
    started_at: float
    seconds: float
    message: str = ""
    evidence: dict = field(default_factory=dict)

    def write(self, out_dir: Path) -> Path:
        out_dir.mkdir(parents=True, exist_ok=True)
        path = out_dir / f"{self.platform}-{self.row}.json"
        path.write_text(json.dumps(self.__dict__, indent=2, sort_keys=True) + "\n")

        return path


def run_row(row: str, platform: str, body, evidence: dict) -> RowResult:
    """Run one row body, mapping its outcome to pass, fail, blocked or error"""
    started = time.time()

    try:
        body()
        status, message = "pass", ""
    except RowFailed as error:
        status, message = "fail", str(error)
    except RowBlocked as error:
        status, message = "blocked", str(error)
    except ForbiddenTap as error:
        status, message = "error", str(error)
    except Exception as error:  # noqa: BLE001 a crashed script must still leave a result file
        status, message = "error", f"{type(error).__name__}: {error}"
        evidence["traceback"] = traceback.format_exc()

    return RowResult(row, platform, status, started, round(time.time() - started, 1), message, evidence)
