"""Release-checklist rows scripted for iOS through WebDriverAgent, read-only

Only rows that need no OS setting change are scripted: the phone is a real device whose
settings stay as its owner left them. The labels match the iOS views (`Receive`, `Transactions`,
`Checking wallet history`) and the sidebar and network picker identifiers; none of this has been
run against a phone yet, so the first run on one should be watched
"""

import time
from typing import Callable

from common import ALLOWED_NETWORKS, RowBlocked, RowFailed, check
from ios.driver import Ios
from screens import error_text, is_wallet_home, labels, row_registry, wait_for_scan_to_settle, wait_for_wallet_home

ROWS: dict[str, Callable[[Ios, dict], None]] = {}

NETWORKS = ("Bitcoin", "Testnet", "Testnet4", "Signet")

# the toolbar button that opens the sidebar, by accessibility identifier
SIDEBAR_BUTTON = r"^app\.sidebar\.open$"


def return_to_wallet_home(driver: Ios) -> None:
    """Get back to the wallet home from wherever a previous row left the app"""
    if not driver.is_foreground():
        driver.launch()
        driver.wait_until(driver.is_foreground, "the app to come to the foreground", 20)

    for _ in range(6):
        if is_wallet_home(driver.app_nodes()):
            return

        try:
            driver.back()
        except RowFailed:
            # nothing to go back to, so the screen is a sheet or the sidebar; a relaunch is the reliable way out
            break

        time.sleep(1)

    driver.cold_restart()
    wait_for_wallet_home(driver)


row = row_registry("ios", ROWS, return_to_wallet_home)


def selected_network(driver: Ios) -> str:
    """Read the network from Settings > Network, where the selected row carries a checkmark"""
    driver.tap_label(SIDEBAR_BUTTON)
    driver.tap_label(r"^Settings$")
    driver.tap_label(r"^Network$")
    driver.wait_for(r"^(?i:checkmark)$")
    nodes = driver.app_nodes()
    marker = driver.find(r"^(?i:checkmark)$", nodes)
    rows = [node for node in nodes if node.label in NETWORKS]
    chosen = min(rows, key=lambda node: abs(node.center[1] - marker.center[1]), default=None)

    if chosen is None or abs(chosen.center[1] - marker.center[1]) > 40:
        raise RowBlocked("could not tell which network is selected in Settings")

    return chosen.label


def preflight(driver: Ios) -> None:
    """Block every row unless WDA answers, the app reaches a wallet home and it runs on a test network"""
    try:
        driver.ensure_installed()
    except RuntimeError as error:
        raise RowBlocked(f"WebDriverAgent is not reachable; run wda-doctor and wda-start ({error})") from error

    driver.launch()

    try:
        return_to_wallet_home(driver)
        network = selected_network(driver)
    except RowFailed as error:
        raise RowBlocked(f"could not read the network from the app ({error})") from error

    return_to_wallet_home(driver)

    if network.lower() not in ALLOWED_NETWORKS:
        raise RowBlocked(f"selected network is {network}; switch the app to Signet, Testnet or Testnet4 first")


@row("s15-repeated-navigation-and-background-route-scope")
def repeated_background_route_scope(driver: Ios, evidence: dict) -> None:
    timings = []

    for index in range(driver.loops):
        started = time.time()
        driver.tap_label(r"^Receive$")
        driver.wait_for(r"^Copy Address$|^Wallet Address$")
        before = set(labels(driver.app_nodes(), driver.package))

        driver.background()
        time.sleep(1)
        driver.foreground()
        driver.wait_for(r"^Copy Address$|^Wallet Address$", 15)

        after = set(labels(driver.app_nodes(), driver.package))
        check(after == before, f"loop {index + 1}: Receive changed after background; lost {sorted(before - after)}, gained {sorted(after - before)}")

        return_to_wallet_home(driver)
        timings.append(round(time.time() - started, 1))

    evidence["loops"] = driver.loops
    evidence["seconds_per_loop"] = timings


@row("s14-online-cold-restart-recovers-scan-start")
def online_cold_restart(driver: Ios, evidence: dict) -> None:
    driver.cold_restart()
    evidence["home_seconds"] = wait_for_wallet_home(driver)

    wait_for_scan_to_settle(driver, evidence)
    check(error_text(driver) is None, f"error shown after an online cold restart: {error_text(driver)}")
