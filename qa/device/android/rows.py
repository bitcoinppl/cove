"""Release-checklist rows scripted for Android, read-only: they navigate and observe, and never send, delete, export or touch backups

Each row is registered under its manifest id with `@row`, which also clears the crash log first,
checks it afterwards, takes a failure screenshot and returns the app to the wallet home so the next
row starts from a known place
"""

import time
from typing import Callable

from android.driver import Android
from common import ALLOWED_NETWORKS, RowBlocked, RowFailed, check
from screens import (
    error_text,
    has,
    is_wallet_home,
    labels,
    row_registry,
    wait_for_scan_to_settle,
    wait_for_scan_to_start,
    wait_for_wallet_home,
)

ROWS: dict[str, Callable[[Android, dict], None]] = {}

NETWORKS = ("Bitcoin", "Testnet", "Testnet4", "Signet")

# the receive sheet controls the font rows compare; both must stay reachable at any font scale
RECEIVE_CONTROLS = ("Copy Address", "Create New Address")

# the largest font scale Android's own settings screen offers
LARGEST_FONT_SCALE = "2.0"


# navigation

def return_to_wallet_home(driver: Android) -> None:
    """Get back to the wallet home from wherever a previous row left the app, without leaving the app"""
    if not driver.is_foreground():
        driver.launch()
        driver.wait_until(driver.is_foreground, "the app to come to the foreground", 20)

    # back on the root screen would leave the app, so only press it while something else is showing
    for _ in range(6):
        nodes = driver.app_nodes()
        if is_wallet_home(nodes):
            return

        if has(nodes, r"^My Wallets$"):
            close_sidebar(driver)
        elif nodes:
            driver.back()

        if not driver.is_foreground():
            driver.launch()

        time.sleep(1)

    driver.cold_restart()
    wait_for_wallet_home(driver)


row = row_registry("android", ROWS, return_to_wallet_home)


def close_sidebar(driver: Android) -> None:
    # Back with the sidebar open leaves the app, so drag it shut instead
    width, height = driver.screen_size()
    driver.swipe(width * 2 // 3, height // 2, width // 10, height // 2)


def open_settings(driver: Android) -> None:
    driver.tap_label(r"^Menu$")
    driver.tap_label(r"^Settings$")
    driver.wait_for(r"^General$")


def open_settings_screen(driver: Android, name: str) -> None:
    """Push one screen from the settings root; the root's own rows share these labels, so wait for the root to go"""
    driver.tap_label(rf"^{name}$")
    driver.wait_until(lambda: not has(driver.app_nodes(), r"^General$"), f"{name} to replace the settings root")


def close_settings_screen(driver: Android) -> None:
    driver.back()
    driver.wait_for(r"^General$")


def open_receive(driver: Android) -> None:
    driver.tap_label(r"^Receive$")
    driver.wait_for(r"^Copy Address$|^Wallet Address$")


def selected_network(driver: Android) -> str:
    """Read the network from Settings > Network, where the selected row carries a `Selected` marker"""
    open_settings(driver)
    driver.tap_label(r"^Network$")
    driver.wait_for(r"^Selected$")
    nodes = driver.app_nodes()
    marker = driver.find(r"^Selected$", nodes)
    rows = [node for node in nodes if node.label in NETWORKS]
    chosen = min(rows, key=lambda node: abs(node.center[1] - marker.center[1]), default=None)

    if chosen is None or abs(chosen.center[1] - marker.center[1]) > 60:
        raise RowBlocked("could not tell which network is selected in Settings")

    return chosen.label


def preflight(driver: Android) -> None:
    """Block every row unless the app is installed, reaches a wallet home and runs on a test network"""
    driver.ensure_installed()
    driver.launch()
    driver.wait_until(driver.is_foreground, "the app to come to the foreground", 20)

    try:
        return_to_wallet_home(driver)
    except RowFailed as error:
        raise RowBlocked(f"no wallet home is reachable; create a Signet wallet first ({error})") from error

    network = selected_network(driver)
    return_to_wallet_home(driver)

    if network.lower() not in ALLOWED_NETWORKS:
        raise RowBlocked(f"selected network is {network}; switch the app to Signet, Testnet or Testnet4 first")


# receive sheet

def receive_controls(driver: Android, evidence: dict, key: str) -> list[str]:
    """Open Receive and list the expected controls that can be reached, scrolling the sheet when one is off screen"""
    open_receive(driver)

    # Copy Address shows disabled until the address loads, and a control that never enables is not usable
    driver.wait_until(
        lambda: all(node.enabled for node in driver.app_nodes() if node.label in RECEIVE_CONTROLS),
        "the receive controls to enable",
        15,
    )

    _, height = driver.screen_size()
    found: list[str] = []
    scrolls = 0

    for scrolls in range(5):
        nodes = driver.app_nodes()
        for control in RECEIVE_CONTROLS:
            node = driver.find(f"^{control}$", nodes)

            # a control drawn under the navigation bar, or one that can't be used, counts as unreachable
            if node and control not in found and node.enabled and node.bounds[3] <= height:
                found.append(control)

        if len(found) == len(RECEIVE_CONTROLS):
            break

        driver.scroll_down()
        time.sleep(0.7)

    evidence[key] = {"found": found, "scrolls": scrolls, "labels": labels(driver.nodes(), driver.package)[:20]}
    driver.back()
    driver.wait_until(lambda: is_wallet_home(driver.app_nodes()), "the receive sheet to close", 10)

    return found


# rows

@row("s16-remove-animations-keeps-navigation-usable")
def remove_animations(driver: Android, evidence: dict) -> None:
    with driver.animations_off():
        # the settings change reaches the app on its next frame, so give it a moment
        time.sleep(1)
        visited = []

        for screen in ("Appearance", "Network", "Currency"):
            open_settings(driver)
            open_settings_screen(driver, screen)
            visited.append(screen)
            close_settings_screen(driver)
            driver.back()
            wait_for_wallet_home(driver, 15)

        open_receive(driver)
        visited.append("Receive")
        driver.back()
        wait_for_wallet_home(driver, 15)

        evidence["screens_visited"] = visited

    check(is_wallet_home(driver.app_nodes()), "wallet home is not usable after navigating with animations off")


@row("s16-receive-copy-address-missing-at-largest-font")
def copy_address_at_largest_font(driver: Android, evidence: dict) -> None:
    with driver.setting("system", "font_scale", LARGEST_FONT_SCALE):
        wait_for_wallet_home(driver, 20)
        found = receive_controls(driver, evidence, "largest_font")

    check("Copy Address" in found, f"Copy Address is not reachable at font scale {LARGEST_FONT_SCALE}; reachable: {found}")


@row("s16-restore-normal-font-and-compare-receive-controls")
def restore_normal_font_and_compare(driver: Android, evidence: dict) -> None:
    original = driver.shell("settings", "get", "system", "font_scale")

    with driver.setting("system", "font_scale", LARGEST_FONT_SCALE):
        wait_for_wallet_home(driver, 20)
        largest = receive_controls(driver, evidence, "largest_font")

    check(driver.shell("settings", "get", "system", "font_scale") == original, "font scale was not restored")
    wait_for_wallet_home(driver, 20)
    normal = receive_controls(driver, evidence, "normal_font")

    check(sorted(normal) == sorted(RECEIVE_CONTROLS), f"controls missing at normal font: {normal}")
    check(sorted(largest) == sorted(normal), f"largest font shows {largest} but normal font shows {normal}")


@row("s15-repeated-navigation-and-background-route-scope")
def repeated_background_route_scope(driver: Android, evidence: dict) -> None:
    # the last route is a sheet over the wallet home, the others are pushed settings screens
    routes = ("Appearance", "Network", "Currency", "Receive")
    pid = driver.pid()
    timings = []

    for index in range(driver.loops):
        route = routes[index % len(routes)]
        started = time.time()

        if route == "Receive":
            open_receive(driver)
            marker = r"^Copy Address$"
        else:
            open_settings(driver)
            open_settings_screen(driver, route)
            marker = r"^Back$"

        driver.wait_for(marker)
        before = set(labels(driver.app_nodes(), driver.package))
        driver.background()
        time.sleep(1)
        driver.foreground()
        driver.wait_for(marker, 15)

        nodes = driver.app_nodes()
        after = set(labels(nodes, driver.package))
        check(after == before, f"loop {index + 1}: {route} changed after background; lost {sorted(before - after)}, gained {sorted(after - before)}")
        check(not has(nodes, r"^General$"), f"loop {index + 1}: {route} fell back to the settings root after background")
        check(driver.pid() == pid, f"loop {index + 1}: the app process was replaced while backgrounded")

        if route == "Receive":
            driver.back()
        else:
            close_settings_screen(driver)
            driver.back()

        wait_for_wallet_home(driver, 15)
        timings.append(round(time.time() - started, 1))

    evidence["loops"] = driver.loops
    evidence["seconds_per_loop"] = timings
    evidence["routes"] = list(routes)


@row("s14-online-cold-restart-recovers-scan-start")
def online_cold_restart(driver: Android, evidence: dict) -> None:
    check(driver.shell("settings", "get", "global", "airplane_mode_on") != "1", "airplane mode is on; the row needs a connection")

    driver.cold_restart()
    evidence["home_seconds"] = wait_for_wallet_home(driver)
    evidence["first_labels"] = labels(driver.app_nodes(), driver.package)[:15]

    wait_for_scan_to_start(driver, evidence)
    wait_for_scan_to_settle(driver, evidence)
    check(error_text(driver) is None, f"error shown after an online cold restart: {error_text(driver)}")


@row("s14-android-scan-survives-short-background")
def scan_survives_short_background(driver: Android, evidence: dict) -> None:
    driver.cold_restart()
    wait_for_wallet_home(driver)

    # the scan starts right after launch, so this is the window a background could interrupt it
    wait_for_scan_to_start(driver, evidence)
    driver.background()
    time.sleep(3)
    driver.foreground()

    evidence["home_seconds"] = wait_for_wallet_home(driver, 20)
    wait_for_scan_to_settle(driver, evidence)
    check(error_text(driver) is None, f"error shown after a short background: {error_text(driver)}")


@row("s01-two-cold-launches-after-upgrade")
def two_cold_launches_after_upgrade(driver: Android, evidence: dict) -> None:
    if driver.upgrade_apk is None:
        raise RowBlocked("pass --upgrade-apk PATH to install the build under test over the existing install")

    if not driver.upgrade_apk.is_file():
        raise RowBlocked(f"{driver.upgrade_apk} does not exist")

    # installing another app's apk, or the same version again, succeeds without running any upgrade
    apk_package, apk_version = driver.apk_package_and_version(driver.upgrade_apk)
    installed_version = driver.installed_version_code()
    evidence["version_code_before"] = installed_version
    evidence["apk_version_code"] = apk_version

    if apk_package != driver.package:
        raise RowBlocked(f"{driver.upgrade_apk} is {apk_package}, not {driver.package}")

    if apk_version <= installed_version:
        raise RowBlocked(
            f"installed versionCode {installed_version} is not older than the apk's {apk_version}, so installing it "
            "is not an upgrade; install the previous release and open a wallet in it first"
        )

    wallets_before = wallet_names(driver)
    check(bool(wallets_before), "no wallets listed before the upgrade, so there is nothing to keep")

    started = time.time()
    driver.install_over_existing(driver.upgrade_apk)
    evidence["install_seconds"] = round(time.time() - started, 1)
    evidence["wallets_before"] = wallets_before
    check(driver.installed_version_code() == apk_version, f"versionCode {apk_version} is not installed after the upgrade")

    for launch in (1, 2):
        driver.cold_restart()
        evidence[f"launch_{launch}_home_seconds"] = wait_for_wallet_home(driver)

    wallets_after = wallet_names(driver)
    evidence["wallets_after"] = wallets_after

    check(wallets_after == wallets_before, f"wallet list changed across the upgrade: {wallets_before} -> {wallets_after}")


# shared by the scan rows

def wallet_names(driver: Android) -> list[str]:
    """Wallet names listed in the sidebar between `My Wallets` and `Add Wallet`

    The list is lazy, so rows scrolled out of view aren't on screen at all; a list that doesn't
    fit blocks the row, because comparing only the visible part could miss a lost wallet
    """
    driver.tap_label(r"^Menu$")
    top = driver.wait_for(r"^My Wallets$").center[1]
    bottom = driver.wait_for(r"^Add Wallet$").center[1]
    width, _ = driver.screen_size()

    # the home screen stays drawn to the right of the drawer, so only nodes in the drawer's own column count
    drawer = [node for node in driver.app_nodes() if top < node.center[1] < bottom and node.center[0] < width // 2]
    names = [node.label for node in drawer if node.clazz.endswith("TextView") and node.label]
    overflows = any(node.scrollable for node in drawer)
    close_sidebar(driver)
    wait_for_wallet_home(driver, 15)

    if overflows:
        raise RowBlocked(f"the sidebar lists more wallets than fit on screen, so the row can't compare them all; showing {names}")

    return names
