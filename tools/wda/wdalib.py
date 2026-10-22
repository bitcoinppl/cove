"""Tiny WebDriverAgent client behind the ios-* scripts; stdlib only

Every command prints one JSON object with "ok", except ios-vis and ios-source --raw, which
print text for reading or piping.

Every command accepts --dry-run, which prints the HTTP requests it would send
without touching the network.
"""
import base64
import json
import os
import re
import sys
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

BASE = os.environ.get("WDA_BASE_URL") or "http://127.0.0.1:%s" % os.environ.get("WDA_PORT", "8100")
STATE = os.environ.get("WDA_STATE") or os.path.join(os.environ.get("WDA_CACHE") or os.path.expanduser("~/.cache/cove-wda"), "state")
UDID = os.environ.get("IOS_UDID", "")
# one cached session per WDA address, so runners for two phones on two ports keep their own
SESSION_FILE = os.path.join(STATE, "session-" + re.sub(r"\W", "_", BASE.split("://")[-1]))
SWIPE_SECONDS = 0.3


class Fail(Exception):
    def __init__(self, code, message, **extra):
        super().__init__(message)
        self.code, self.message, self.extra = code, message, extra


def emit(ok, **fields):
    out = {"ok": ok, **fields}
    if UDID:
        out["udid"] = UDID
    print(json.dumps(out))


def http(method, path, body=None, timeout=60):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            err = json.loads(raw).get("value", {})
        except ValueError:
            err = {}
        raise Fail("wda_http_%d" % e.code, err.get("message") or raw[:300].decode(errors="replace"), wda_error=err.get("error", ""), path=path)
    except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
        raise Fail("wda_unreachable", "cannot reach WDA at %s (%s); run wda-doctor, then wda-start" % (BASE, e), base_url=BASE)
    return json.loads(raw) if raw else {}


def new_session():
    r = http("POST", "/session", {"capabilities": {}})
    sid = r.get("sessionId") or r.get("value", {}).get("sessionId")
    if not sid:
        raise Fail("no_session", "WDA did not return a session id", response=r)
    os.makedirs(STATE, exist_ok=True)
    with open(SESSION_FILE, "w") as f:
        f.write(sid)
    # Cove has continuous animations, so XCTest's wait-for-idle never finishes: the tree goes
    # stale and swipes get dropped; 0 disables it, WDA_WAIT_FOR_IDLE=2 restores the default
    idle = float(os.environ.get("WDA_WAIT_FOR_IDLE", "0"))
    http("POST", "/session/%s/appium/settings" % sid, {"settings": {"waitForIdleTimeout": idle, "animationCoolOffTimeout": idle}})
    return sid


def session():
    """Return a live session id, reusing the cached one when WDA still knows it"""
    try:
        with open(SESSION_FILE) as f:
            sid = f.read().strip()
    except OSError:
        sid = ""
    if sid:
        try:
            http("GET", "/session/%s/window/size" % sid, timeout=10)
            return sid
        except Fail as e:
            if e.code == "wda_unreachable":
                raise
    return new_session()


def post(path, body):
    return http("POST", "/session/%s%s" % (session(), path), body)


def window_size():
    v = http("GET", "/session/%s/window/size" % session())["value"]
    return float(v["width"]), float(v["height"])


def swipe_points(direction, w, h):
    # swipe the finger from the first point to the second; "up" scrolls content up, like a thumb swipe
    cx, cy = w / 2, h / 2
    return {
        "up": (cx, h * 0.75, cx, h * 0.25),
        "down": (cx, h * 0.25, cx, h * 0.75),
        "left": (w * 0.8, cy, w * 0.2, cy),
        "right": (w * 0.2, cy, w * 0.8, cy),
    }[direction]


def cmd_launch(args, dry):
    args, activate = pop_flag(args, "--activate")
    ids = operands(args)
    if len(ids) != 1:
        raise Fail("usage", "usage: ios-launch [--dry-run] [--activate] BUNDLE_ID")
    path = "/wda/apps/activate" if activate else "/wda/apps/launch"
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>" + path, "body": {"bundleId": ids[0]}}]}
    post(path, {"bundleId": ids[0]})
    return {"bundle_id": ids[0], "action": "activate" if activate else "launch"}


def options_end(args):
    """Index of the `--` that ends the options, or the end; text after it is never an option"""
    return args.index("--") if "--" in args else len(args)


def pop_flag(args, name):
    """Remove the option `name` and return (args, whether it was given)"""
    if name not in args[:options_end(args)]:
        return args, False
    i = args.index(name)
    return args[:i] + args[i + 1:], True


def pop_value(args, name, usage):
    """Remove `name VALUE` from the options and return (args, VALUE or None)"""
    if name not in args[:options_end(args)]:
        return args, None
    i = args.index(name)
    if i + 1 >= options_end(args):
        raise Fail("usage", usage)
    return args[:i] + args[i + 2:], args[i + 1]


def operands(args):
    """The arguments left once the options are popped, without the `--` that ended them"""
    i = options_end(args)
    return args[:i] + args[i + 1:]


def pop_hold(args, default):
    # --hold is how long the finger rests before moving; SwiftUI starts drag-to-reorder after
    # a long enough rest, so scrolls over reorderable rows (Cove's sidebar) want a short one
    if "--hold" not in args[:options_end(args)]:
        return args, default
    i = args.index("--hold")
    try:
        secs = float(args[i + 1])
    except (IndexError, ValueError):
        raise Fail("usage", "--hold needs a number of seconds")
    return args[:i] + args[i + 2:], secs


def cmd_swipe(args, dry):
    args, hold = pop_hold(args, SWIPE_SECONDS)
    args = operands(args)
    if len(args) != 1 or args[0] not in ("up", "down", "left", "right"):
        raise Fail("usage", "usage: ios-swipe [--dry-run] [--hold SECONDS] up|down|left|right")
    d = args[0]
    if dry:
        return {"would_send": [{"method": "GET", "path": "/session/<id>/window/size"},
                               {"method": "POST", "path": "/session/<id>/wda/dragfromtoforduration", "body": "{fromX,fromY,toX,toY,duration} computed from window size"}],
                "direction": d, "hold": hold}
    w, h = window_size()
    fx, fy, tx, ty = swipe_points(d, w, h)
    post("/wda/dragfromtoforduration", {"fromX": fx, "fromY": fy, "toX": tx, "toY": ty, "duration": hold})
    return {"direction": d, "from": [fx, fy], "to": [tx, ty], "hold": hold}


def cmd_drag(args, dry):
    args, hold = pop_hold(args, 1.0)
    args = operands(args)
    try:
        fx, fy, tx, ty = (float(a) for a in args)
    except ValueError:
        raise Fail("usage", "usage: ios-drag [--dry-run] [--hold SECONDS] FROM_X FROM_Y TO_X TO_Y  (points; default hold 1 s)")
    body = {"fromX": fx, "fromY": fy, "toX": tx, "toY": ty, "duration": hold}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/wda/dragfromtoforduration", "body": body}]}
    post("/wda/dragfromtoforduration", body)
    return {"from": [fx, fy], "to": [tx, ty], "hold": hold}


def cmd_tap(args, dry):
    args = operands(args)
    try:
        if len(args) != 2:
            raise ValueError
        x, y = float(args[0]), float(args[1])
    except ValueError:
        raise Fail("usage", "usage: ios-tap [--dry-run] X Y (points, not pixels)")
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/wda/tap", "body": {"x": x, "y": y}}]}
    post("/wda/tap", {"x": x, "y": y})
    return {"x": x, "y": y}


def find_by_label(label):
    """Find an element by accessibility id, then by exact label (SwiftUI often differs: name BackButton, label Back)"""
    quoted = label.replace("\\", "\\\\").replace('"', '\\"')
    attempts = [{"using": "accessibility id", "value": label},
                {"using": "predicate string", "value": 'label == "%s" OR name == "%s"' % (quoted, quoted)}]
    for q in attempts:
        try:
            return post("/element", q)["value"]
        except Fail as e:
            if not e.code.startswith("wda_http_404"):
                raise
    raise Fail("label_not_found", "no element with accessibility id, label or name %r; check ios-source" % label)


def cmd_tap_label(args, dry):
    args = operands(args)
    if len(args) != 1 or not args[0]:
        raise Fail("usage", "usage: ios-tap-label [--dry-run] [--] LABEL (exact accessibility id, label or name)")
    label = args[0]
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/element", "body": {"using": "accessibility id", "value": label}},
                               {"method": "POST", "path": "/session/<id>/element", "body": {"using": "predicate string", "value": "label == LABEL OR name == LABEL (fallback)"}},
                               {"method": "POST", "path": "/session/<id>/element/<element-id>/click", "body": {}}]}
    found = find_by_label(label)
    eid = found.get("ELEMENT") or found.get("element-6066-11e4-a52e-4f735466cecf")
    post("/element/%s/click" % eid, {})
    return {"label": label}


def cmd_type(args, dry):
    args = operands(args)
    if len(args) != 1:
        raise Fail("usage", "usage: ios-type [--dry-run] [--] TEXT (one argument; quote it, and put -- before text that starts with -)")
    text = args[0]
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/wda/keys", "body": {"value": list(text)}}]}
    post("/wda/keys", {"value": list(text)})
    return {"typed_chars": len(text)}


def cmd_source(args, dry):
    fmt = "xml"
    args, raw = pop_flag(args, "--raw")
    for a in operands(args):
        if a in ("--json", "json"):
            fmt = "json"
        elif a in ("--xml", "xml"):
            fmt = "xml"
        else:
            raise Fail("usage", "usage: ios-source [--dry-run] [--raw] [--json|--xml]")
    path = "/source?format=%s" % fmt
    if dry:
        return {"would_send": [{"method": "GET", "path": path}], "format": fmt}
    value = http("GET", path)["value"]
    if raw:
        # print only the tree so it can be piped to grep or a file
        print(value if isinstance(value, str) else json.dumps(value, indent=1))
        sys.exit(0)
    return {"format": fmt, "source": value}


def vis_lines(xml, show_hidden):
    """One line per element worth acting on: type, tap center and size in points, then its text"""
    try:
        root = ET.fromstring(xml)
    except ET.ParseError as e:
        raise Fail("bad_source", "WDA returned a tree that is not valid XML (%s); retry, or check ios-shot" % e)

    lines = []
    for el in root.iter():
        a = el.attrib
        visible = a.get("visible") == "true"
        if not visible and not show_hidden:
            continue

        label, name, value = a.get("label"), a.get("name"), a.get("value")
        kind = a.get("type", el.tag).replace("XCUIElementType", "")
        if not (label or name or value) or kind in ("Application", "Window"):
            continue

        x, y, w, h = (int(float(a.get(k) or 0)) for k in ("x", "y", "width", "height"))
        # name is the accessibility id; show it and the value only when they add to the label
        text = ["label=%r" % label] if label else []
        text += ["name=%r" % name] if name and name != label else []
        text += ["value=%r" % value] if value and value != label else []
        flags = ("" if visible else " [hidden]") + ("" if a.get("enabled", "true") == "true" else " [disabled]")
        lines.append("%-14s (%d,%d) %dx%d %s%s" % (kind, x + w // 2, y + h // 2, w, h, " ".join(text), flags))
    return lines


def cmd_vis(args, dry):
    args, show_hidden = pop_flag(args, "-a")
    if operands(args):
        raise Fail("usage", "usage: ios-vis [--dry-run] [-a]  (-a includes hidden elements)")
    if dry:
        return {"would_send": [{"method": "GET", "path": "/source?format=xml"}]}
    # SwiftUI keeps hidden layers in the tree, so only visible elements are printed by default
    lines = vis_lines(http("GET", "/source?format=xml")["value"], show_hidden)
    if lines:
        print("\n".join(lines))
    sys.exit(0)


def cmd_shot(args, dry):
    if len(args) != 1:
        raise Fail("usage", "usage: ios-shot [--dry-run] PATH.png")
    path = os.path.abspath(os.path.expanduser(args[0]))
    if dry:
        return {"would_send": [{"method": "GET", "path": "/screenshot"}], "path": path}
    b64 = http("GET", "/screenshot")["value"]
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(base64.b64decode(b64))
    return {"path": path, "bytes": os.path.getsize(path)}


RUNNER_ID = os.environ.get("WDA_RUNNER_ID") or os.environ.get("WDA_BUNDLE_ID", "com.avencera.WebDriverAgentRunner") + ".xctrunner"


def cmd_pasteboard(args, dry):
    # iOS only lets the foreground app touch the pasteboard, so the runner comes forward for
    # the read or write, then the app named by --app (default Cove) is reactivated
    usage = "usage: ios-pasteboard [--dry-run] [--app BUNDLE_ID] get | set [--] TEXT"
    args, app = pop_value(args, "--app", usage)
    app = app or "org.bitcoinppl.cove"
    args = operands(args)
    if not args or args[0] not in ("get", "set") or (args[0] == "set") != (len(args) == 2) or len(args) > 2:
        raise Fail("usage", usage)
    op = args[0]
    body = {"contentType": "plaintext"}
    if op == "set":
        body["content"] = base64.b64encode(args[1].encode()).decode()
    steps = [("/wda/apps/activate", {"bundleId": RUNNER_ID}), ("/wda/%sPasteboard" % op, body), ("/wda/apps/activate", {"bundleId": app})]
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>" + path, "body": b} for path, b in steps]}
    post(*steps[0])
    try:
        value = post(*steps[1]).get("value")
    finally:
        post(*steps[2])
    if op == "set":
        return {"op": "set", "chars": len(args[1]), "app": app}
    return {"op": "get", "text": base64.b64decode(value or "").decode(errors="replace"), "app": app}


def cmd_hold(args, dry):
    args = operands(args)
    try:
        x, y = float(args[0]), float(args[1])
        secs = float(args[2]) if len(args) == 3 else 1.0
        if len(args) not in (2, 3):
            raise ValueError
    except (ValueError, IndexError):
        raise Fail("usage", "usage: ios-hold [--dry-run] X Y [SECONDS]  (points; default 1 second)")
    body = {"x": x, "y": y, "duration": secs}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/wda/touchAndHold", "body": body}]}
    post("/wda/touchAndHold", body)
    return {"x": x, "y": y, "seconds": secs}


COMMANDS = {"launch": cmd_launch, "swipe": cmd_swipe, "tap": cmd_tap, "tap-label": cmd_tap_label,
            "type": cmd_type, "source": cmd_source, "shot": cmd_shot,
            "pasteboard": cmd_pasteboard, "hold": cmd_hold, "drag": cmd_drag, "vis": cmd_vis}


def main(argv):
    name, rest = argv[0], argv[1:]
    rest, dry = pop_flag(rest, "--dry-run")
    try:
        result = COMMANDS[name](rest, dry)
    except Fail as e:
        emit(False, error=e.code, message=e.message, **e.extra)
        return 2 if e.code == "usage" else 1
    except Exception as e:
        # keep the one-JSON-object contract for callers that parse every result
        emit(False, error="internal", message=repr(e)[:300])
        return 1
    emit(True, command="ios-" + name, dry_run=dry, **result)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
