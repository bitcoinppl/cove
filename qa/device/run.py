#!/usr/bin/env python3
"""Run release-checklist device scripts against one Android device or iPhone and write a JSON result per row

Examples:
  python3 -I qa/device/run.py android --serial emulator-5554 --package org.bitcoinppl.cove.dev
  python3 -I qa/device/run.py ios --rows s16-remove-animations-keeps-navigation-usable
  python3 -I qa/device/run.py android --serial emulator-5554 --list
  python3 -I qa/device/run.py android --serial emulator-5554 --upgrade-apk app-dev-debug.apk --loops 30

Exit status is 0 only when every selected row passes. Results go to --out (default
_scratch/device-results/<platform>-<timestamp>/), one `<platform>-<row>.json` per row plus
`summary.json`, so a scheduler can keep or upload them
"""

import argparse
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from common import run_row  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("platform", choices=["android", "ios"])
    parser.add_argument("--serial", help="adb serial; required for android so an attached phone is never picked by accident")
    parser.add_argument("--package", default="org.bitcoinppl.cove.dev", help="android application id")
    parser.add_argument("--bundle-id", default="org.bitcoinppl.cove", help="ios bundle id")
    parser.add_argument("--rows", help="comma-separated row ids; default is every row for the platform")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--upgrade-apk", type=Path, help="android apk to install over the existing install for the upgrade row")
    parser.add_argument("--loops", type=int, default=10, help="repetitions for rows that loop, such as repeated background and foreground")
    parser.add_argument("--list", action="store_true", help="print the row ids and exit")
    args = parser.parse_args()

    out = args.out or HERE.parents[1] / "_scratch" / "device-results" / f"{args.platform}-{time.strftime('%Y%m%d-%H%M%S')}"

    if args.platform == "android":
        if not args.serial:
            parser.error("--serial is required for android")

        from android.driver import Android
        from android.rows import ROWS, preflight

        driver = Android(args.serial, args.package, args.upgrade_apk, out, args.loops)
    else:
        from ios.driver import Ios
        from ios.rows import ROWS, preflight

        driver = Ios(args.bundle_id, out, args.loops)

    if args.list:
        print("\n".join(ROWS))
        return 0

    selected = args.rows.split(",") if args.rows else list(ROWS)
    unknown = [row for row in selected if row not in ROWS]
    if unknown:
        parser.error(f"unknown rows: {', '.join(unknown)}")

    results = []

    # a failed preflight blocks every row instead of producing misleading failures
    blocked = run_row("preflight", args.platform, lambda: preflight(driver), {})
    blocked.write(out)

    for row in selected:
        evidence: dict = {}

        if blocked.status != "pass":
            result = run_row(row, args.platform, lambda: None, evidence)
            result.status, result.message = "blocked", f"preflight {blocked.status}: {blocked.message}"
        else:
            result = run_row(row, args.platform, lambda row=row, evidence=evidence: ROWS[row](driver, evidence), evidence)

        result.write(out)
        results.append(result)
        print(json.dumps({"row": row, "status": result.status, "message": result.message, "seconds": result.seconds}), flush=True)

    summary = {
        "platform": args.platform,
        "preflight": blocked.status,
        "counts": {status: sum(r.status == status for r in results) for status in ("pass", "fail", "blocked", "error")},
        "rows": {r.row: r.status for r in results},
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"out": str(out), **summary["counts"]}))

    return 0 if all(r.status == "pass" for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
