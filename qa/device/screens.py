"""Screen helpers and the row decorator shared by the Android and iOS rows

Both apps use the same copy for the wallet home (`Receive`, `Transactions`) and its scan banner,
so one set of readers serves both drivers
"""

import re
import time
from typing import Callable

from common import Device, Node, RowBlocked, RowFailed, WaitTimedOut

# text that only an error alert or banner carries; the app's normal copy never uses these words
ERROR_TEXT = re.compile(r"\b(error|failed|failure|unable to|could not|couldn't|can't connect|no connection)\b", re.IGNORECASE)

# shown on the wallet home while a history scan runs: the first scan, then later scans of a wallet with transactions
SCAN_IN_PROGRESS = r"^(Checking wallet history|Scanning for transactions)"

# a wallet that finished its first scan hides scan progress for this long, so a quicker scan never shows
RETURNING_WALLET_SCAN_PROGRESS_DELAY = 5


def labels(nodes: list[Node], package: str) -> list[str]:
    """Distinct visible labels of the app's own windows, in screen order, shortened for evidence"""
    seen: list[str] = []
    for node in nodes:
        label = node.label.replace("\n", " ")[:60]
        if node.package == package and label and label not in seen:
            seen.append(label)

    return seen


def has(nodes: list[Node], pattern: str) -> bool:
    regex = re.compile(pattern)
    return any(regex.search(node.label) for node in nodes)


def is_wallet_home(nodes: list[Node]) -> bool:
    return has(nodes, r"^Receive$") and has(nodes, r"^Transactions$")


def error_text(driver: Device) -> str | None:
    # pass the compiled regex so its IGNORECASE flag survives, which alert titles like `Node Connection Failed` need
    node = driver.find(ERROR_TEXT, driver.app_nodes())
    return node.label if node else None


def save_screenshot(driver, platform: str, row_id: str, evidence: dict) -> None:
    if driver.out_dir is None:
        return

    try:
        driver.out_dir.mkdir(parents=True, exist_ok=True)
        path = driver.out_dir / f"{platform}-{row_id}.png"
        driver.screenshot(str(path))
        evidence["screenshot"] = str(path)
        evidence["screen_labels_at_failure"] = labels(driver.nodes(), driver.package)[:25]
    except Exception as error:  # noqa: BLE001 a missing screenshot must not hide the real failure
        evidence["screenshot_error"] = str(error)


def wait_for_wallet_home(driver, timeout: float = 45) -> float:
    """Wait until the wallet home renders and return how long that took"""
    started = time.time()
    driver.wait_until(lambda: is_wallet_home(driver.app_nodes()), "the wallet home to render", timeout)

    return round(time.time() - started, 1)


def wait_for_scan_to_start(driver, evidence: dict, timeout: float = 30) -> None:
    """Wait until the wallet home shows a history scan running, recording how long that took

    The screen can't tell a scan that finished before its progress showed from one that never
    started, so a scan that never shows blocks the row instead of passing it. An error shown
    while waiting is a clear answer, so it fails the row, and so does any other failure, such as
    the app leaving the foreground
    """
    started = time.time()
    alerts: list[str] = []

    def started_or_failed() -> bool:
        nodes = driver.app_nodes()
        alert = driver.find(ERROR_TEXT, nodes)
        if alert:
            alerts.append(alert.label)

        return alert is not None or has(nodes, SCAN_IN_PROGRESS)

    # only the timeout is unobservable; a crash or another app in front is a finding and must stay a failure
    try:
        driver.wait_until(started_or_failed, "a wallet history scan to show", timeout)
    except WaitTimedOut as error:
        raise RowBlocked(
            f"no history scan showed within {timeout}s, so the row can't tell whether one started; a returning wallet "
            f"hides scan progress for its first {RETURNING_WALLET_SCAN_PROGRESS_DELAY}s, so use a wallet with "
            "transactions whose scan takes longer than that"
        ) from error

    if alerts:
        raise RowFailed(f"error shown while waiting for a history scan: {alerts[-1]}")

    evidence["scan_start_seconds"] = round(time.time() - started, 1)


def wait_for_scan_to_settle(driver, evidence: dict, timeout: float = 90) -> None:
    """Wait for the wallet history scan to leave its in-progress state, recording how long that took"""
    started = time.time()
    saw_scan = False
    quiet_polls = 0

    def settled() -> bool:
        nonlocal saw_scan, quiet_polls
        nodes = driver.app_nodes()
        in_progress = has(nodes, SCAN_IN_PROGRESS)
        saw_scan = saw_scan or in_progress

        # two quiet polls in a row, because the banner can be absent for one frame while the screen redraws
        quiet_polls = quiet_polls + 1 if is_wallet_home(nodes) and not in_progress else 0

        return quiet_polls >= 2

    driver.wait_until(settled, "the wallet history scan to finish", timeout)
    evidence["scan_seen_in_progress"] = saw_scan
    evidence["scan_settle_seconds"] = round(time.time() - started, 1)
    evidence["settled_labels"] = labels(driver.app_nodes(), driver.package)[:15]


def row_registry(platform: str, rows: dict, return_to_home: Callable) -> Callable[[str], Callable]:
    """Build the `@row(id)` decorator for one platform

    A registered row starts from a clean crash log and the wallet home, takes a failure
    screenshot, and fails if the app crashed while it ran
    """

    def row(row_id: str):
        def register(body: Callable):
            def run(driver, evidence: dict) -> None:
                driver.clear_crash_log()

                try:
                    return_to_home(driver)
                    body(driver, evidence)
                except RowFailed:
                    save_screenshot(driver, platform, row_id, evidence)
                    raise
                finally:
                    evidence["crashes"] = driver.crashes()

                if evidence["crashes"]:
                    raise RowFailed(f"app crashed: {evidence['crashes'][0]}")

            rows[row_id] = run

            return body

        return register

    return row
