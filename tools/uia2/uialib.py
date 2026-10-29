"""Tiny UiAutomator2 server client behind the android-* scripts; stdlib only

Every command prints one JSON object with "ok", except android-vis and android-source --raw,
which print text for reading or piping. Action commands accept --dry-run, which prints the
requests they would send without touching the network or the device.
"""
import base64
import fcntl
import json
import os
import re
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

BASE = os.environ.get("UIA2_BASE_URL") or "http://127.0.0.1:%s" % os.environ.get("UIA2_PORT", "8200")
STATE = os.environ.get("UIA2_STATE") or os.path.join(os.environ.get("UIA2_CACHE") or os.path.expanduser("~/.cache/cove-uia2"), "state")
SERIAL = os.environ.get("ANDROID_SERIAL", "")
SESSION_FILE = os.path.join(STATE, "session-" + re.sub(r"\W", "_", BASE.split("://")[-1]))
SWIPE_HOLD = 0.05
SWIPE_MOVE_MS = 300
DRAG_MOVE_MS = 500
SECURE_SCREEN = "unable to capture screen"
# Android's package name grammar; android-launch rejects anything else before it reaches `adb shell`
PACKAGE_NAME = re.compile(r"[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+")

# key names accepted by android-key, so agents don't need the numeric codes
KEYCODES = {"home": 3, "back": 4, "tab": 61, "enter": 66, "del": 67, "menu": 82, "escape": 111,
            "app_switch": 187, "paste": 279, "copy": 278, "cut": 277, "wakeup": 224}


class Fail(Exception):
    def __init__(self, code, message, **extra):
        super().__init__(message)
        self.code, self.message, self.extra = code, message, extra


def emit(ok, **fields):
    out = {"ok": ok, **fields}
    if SERIAL:
        out["serial"] = SERIAL
    print(json.dumps(out))


def http(method, path, body=None, timeout=60.0):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            err = json.loads(raw).get("value") or {}
        except ValueError:
            err = {}
        # drop the server's Java stacktrace; it costs tokens and never says what to do next
        code = "uia2_" + (err.get("error") or "http_%d" % e.code).replace(" ", "_")
        raise Fail(code, (err.get("message") or raw[:300].decode(errors="replace"))[:500], path=path)
    except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
        raise Fail("uia2_unreachable", "cannot reach the UiAutomator2 server at %s (%s); run uia2-doctor, then uia2-start" % (BASE, e), base_url=BASE)
    return json.loads(raw) if raw else {}


def new_session():
    r = http("POST", "/session", {"capabilities": {"alwaysMatch": {"platformName": "Android"}}})
    sid = r.get("sessionId") or (r.get("value") or {}).get("sessionId")
    if not sid:
        raise Fail("no_session", "the server did not return a session id", response=r)
    # the default 10 s idle wait stalls every command while a spinner or sync animation runs;
    # a short bound keeps reads fresh without waiting on animations that never settle
    idle_ms = int(float(os.environ.get("UIA2_WAIT_FOR_IDLE", "1")) * 1000)
    http("POST", "/session/%s/appium/settings" % sid, {"settings": {"waitForIdleTimeout": idle_ms}})

    # publish the id only once the session is fully set up, and never as a half-written file
    tmp = "%s.%d.tmp" % (SESSION_FILE, os.getpid())
    with open(tmp, "w") as f:
        f.write(sid)
    os.replace(tmp, SESSION_FILE)
    return sid


_SID = ""


def session():
    """Return a live session id, reusing the cached one when the server still knows it"""
    global _SID
    if _SID:
        return _SID

    # a new session replaces the server's current one, so two commands that both find the
    # cache missing or stale would cut each other off; check and create under one lock
    os.makedirs(STATE, exist_ok=True)
    with open(SESSION_FILE + ".lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            with open(SESSION_FILE) as f:
                sid = f.read().strip()
        except OSError:
            sid = ""
        if sid:
            try:
                http("GET", "/session/%s" % sid, timeout=10)
                _SID = sid
                return sid
            except Fail as e:
                if e.code == "uia2_unreachable":
                    raise
        _SID = new_session()
    return _SID


def get(path):
    return http("GET", "/session/%s%s" % (session(), path)).get("value")


def post(path, body, timeout=60.0):
    return http("POST", "/session/%s%s" % (session(), path), body, timeout).get("value")


def window_size():
    v = get("/window/current/size")
    return float(v["width"]), float(v["height"])


def options_end(args):
    """Options end at the first `--`; every argument after it is literal, even one that looks like a flag"""
    return args.index("--") if "--" in args else len(args)


def pop_flag(args, name):
    """Remove the option `name` and return (args, whether it was given)"""
    if name not in args[:options_end(args)]:
        return args, False
    i = args.index(name)
    return args[:i] + args[i + 1:], True


def operands(args):
    """The arguments left once the options are popped, without the `--` that ended them"""
    i = options_end(args)
    return args[:i] + args[i + 1:]


def pop_opt(args, name, default, cast=float, usage=""):
    """Remove `name VALUE` from args and return (args, value)"""
    if name not in args[:options_end(args)]:
        return args, default
    i = args.index(name)
    try:
        value = cast(args[i + 1])
    except (IndexError, ValueError):
        raise Fail("usage", "%s needs a value; %s" % (name, usage))
    return args[:i] + args[i + 2:], value


def pointer(points, hold, move_ms):
    """One-finger W3C action: press at the first point, rest `hold` seconds, then move through the rest"""
    x, y = points[0]
    acts = [{"type": "pointerMove", "duration": 0, "x": x, "y": y}, {"type": "pointerDown", "button": 0},
            {"type": "pause", "duration": int(hold * 1000)}]
    acts += [{"type": "pointerMove", "duration": move_ms, "x": px, "y": py} for px, py in points[1:]]
    acts.append({"type": "pointerUp", "button": 0})
    return {"actions": [{"type": "pointer", "id": "finger1", "parameters": {"pointerType": "touch"}, "actions": acts}]}


def swipe_points(direction, w, h):
    # swipe the finger from the first point to the second; "up" scrolls content up, like a thumb swipe
    cx, cy = int(w / 2), int(h / 2)
    return {
        "up": ((cx, int(h * 0.75)), (cx, int(h * 0.25))),
        "down": ((cx, int(h * 0.25)), (cx, int(h * 0.75))),
        "left": ((int(w * 0.8), cy), (int(w * 0.2), cy)),
        "right": ((int(w * 0.2), cy), (int(w * 0.8), cy)),
    }[direction]


def cmd_swipe(args, dry):
    usage = "usage: android-swipe [--dry-run] [--hold SECONDS] up|down|left|right"
    args, hold = pop_opt(args, "--hold", SWIPE_HOLD, usage=usage)
    args = operands(args)
    if len(args) != 1 or args[0] not in ("up", "down", "left", "right"):
        raise Fail("usage", usage)
    d = args[0]
    if dry:
        return {"would_send": [{"method": "GET", "path": "/session/<id>/window/current/size"},
                               {"method": "POST", "path": "/session/<id>/actions", "body": "press, rest, move computed from window size"}],
                "direction": d, "hold": hold}
    w, h = window_size()
    start, end = swipe_points(d, w, h)
    post("/actions", pointer([start, end], hold, SWIPE_MOVE_MS))
    return {"direction": d, "from": list(start), "to": list(end), "hold": hold}


def cmd_drag(args, dry):
    usage = "usage: android-drag [--dry-run] [--hold SECONDS] FROM_X FROM_Y TO_X TO_Y  (pixels; default hold 1 s)"
    args, hold = pop_opt(args, "--hold", 1.0, usage=usage)
    args = operands(args)
    try:
        fx, fy, tx, ty = (int(float(a)) for a in args)
    except ValueError:
        raise Fail("usage", usage)
    body = pointer([(fx, fy), (tx, ty)], hold, DRAG_MOVE_MS)
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/actions", "body": body}]}
    post("/actions", body)
    return {"from": [fx, fy], "to": [tx, ty], "hold": hold}


def parse_point(args, usage):
    try:
        if len(args) != 2:
            raise ValueError
        return int(float(args[0])), int(float(args[1]))
    except ValueError:
        raise Fail("usage", usage)


def cmd_tap(args, dry):
    x, y = parse_point(operands(args), "usage: android-tap [--dry-run] X Y (pixels, as printed by android-vis)")
    body = {"offset": {"x": x, "y": y}}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/appium/gestures/click", "body": body}]}
    post("/appium/gestures/click", body)
    return {"x": x, "y": y}


def cmd_hold(args, dry):
    usage = "usage: android-hold [--dry-run] X Y [SECONDS]  (pixels; default 1 second)"
    args = operands(args)
    secs = 1.0
    if len(args) == 3:
        try:
            secs = float(args.pop())
        except ValueError:
            raise Fail("usage", usage)
    x, y = parse_point(args, usage)
    body = {"offset": {"x": x, "y": y}, "duration": int(secs * 1000)}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/appium/gestures/long_click", "body": body}]}
    post("/appium/gestures/long_click", body)
    return {"x": x, "y": y, "seconds": secs}


def selectors(label):
    """UiSelector queries tried in order: Compose test tag, visible text, content description"""
    q = label.replace("\\", "\\\\").replace('"', '\\"')
    return [("tag", 'new UiSelector().resourceId("%s")' % q),
            ("text", 'new UiSelector().text("%s")' % q),
            ("desc", 'new UiSelector().description("%s")' % q)]


def element_id(el):
    return el.get("ELEMENT") or el.get("element-6066-11e4-a52e-4f735466cecf")


def find_label(label, nth=1, wait=0.0):
    """Return (element id, matched_by), polling up to `wait` seconds for the element to appear

    The first round of lookups always completes; after that, no request outlives the deadline.
    """
    deadline = time.monotonic() + wait
    first = True
    while True:
        counts = {"tag": 0, "text": 0, "desc": 0}
        for by, sel in selectors(label):
            remaining = deadline - time.monotonic()
            if remaining <= 0 and not first:
                break
            found = post("/elements", {"strategy": "-android uiautomator", "selector": sel},
                         timeout=60.0 if first else max(remaining, 0.5)) or []
            counts[by] = len(found)
            if len(found) >= nth:
                return element_id(found[nth - 1]), by
        if time.monotonic() >= deadline:
            raise Fail("label_not_found", "no element #%d with test tag, text or content-desc %r%s; check android-vis" %
                       (nth, label, " after %gs" % wait if wait else ""), matches=counts)
        first = False
        time.sleep(min(0.3, max(deadline - time.monotonic(), 0)))


def cmd_tap_label(args, dry):
    usage = "usage: android-tap-label [--dry-run] [--nth N] [--wait SECONDS] [--] LABEL (exact test tag, text or content-desc)"
    args, nth = pop_opt(args, "--nth", 1, int, usage)
    args, wait = pop_opt(args, "--wait", float(os.environ.get("UIA2_WAIT", "5")), usage=usage)
    args = operands(args)
    if len(args) != 1 or not args[0] or nth < 1:
        raise Fail("usage", usage)
    label = args[0]
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/elements", "body": {"strategy": "-android uiautomator", "selector": s}}
                               for _, s in selectors(label)] + [{"method": "POST", "path": "/session/<id>/element/<element-id>/click", "body": {}}],
                "nth": nth, "wait": wait}
    eid, by = find_label(label, nth, wait)
    post("/element/%s/click" % eid, {})
    return {"label": label, "matched_by": by, "nth": nth}


def cmd_type(args, dry):
    # the server sets the focused field's whole text through accessibility, so characters can't
    # reorder the way fast `adb shell input text` keystrokes do; it replaces rather than appends
    args, enter = pop_flag(args, "--enter")
    args = operands(args)
    if len(args) != 1:
        raise Fail("usage", "usage: android-type [--dry-run] [--enter] [--] TEXT (replaces the focused field's text; quote it, and put -- before text that starts with -)")
    text = args[0]
    # the server presses Enter after setting text that ends in a literal backslash-n, and keeps the
    # backslash-n in the field (SendKeysToElement in v10.6.6); both routes to it do this, and a split
    # send would need the field's text read back, which password fields hide, so refuse the text
    if text.endswith("\\n"):
        raise Fail("usage", "android-type can't type text that ends in a literal \\n: the server reads it as an Enter request; "
                            "type it without the trailing \\n and pass --enter to press Enter")
    body = {"text": text, "replace": True}
    enter_req = ("/appium/device/press_keycode", {"keycode": KEYCODES["enter"]})
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>" + p, "body": b}
                               for p, b in [("/keys", body)] + ([enter_req] if enter else [])]}
    try:
        post("/keys", body)
    except Fail as e:
        # the server's error quotes the text it was given, which may be a secret
        raise Fail(e.code, "the server could not set the focused field's text; focus a text field first (android-vis shows F)")
    # report only whether the field took the text, so secrets typed into it stay out of the output;
    # read back before Enter, which may submit the form and move focus
    try:
        active = element_id(get("/element/active"))
        matches = get("/element/%s/text" % active) == text
    except Fail:
        matches = None
    # the server's own trailing-"\n" Enter leaves a literal backslash-n in the field, so press it here
    if enter:
        post(*enter_req)
    return {"typed_chars": len(text), "enter": enter, "readback_matches": matches}


def cmd_back(args, dry):
    if operands(args):
        raise Fail("usage", "usage: android-back [--dry-run]")
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/back", "body": {}}]}
    post("/back", {})
    return {}


def cmd_key(args, dry):
    usage = "usage: android-key [--dry-run] NAME|KEYCODE  (names: %s)" % ", ".join(sorted(KEYCODES))
    args = operands(args)
    if len(args) != 1:
        raise Fail("usage", usage)
    name = args[0].lower().removeprefix("keycode_")
    code = KEYCODES.get(name) if not name.isdigit() else int(name)
    if code is None:
        raise Fail("usage", usage)
    body = {"keycode": code}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/appium/device/press_keycode", "body": body}]}
    post("/appium/device/press_keycode", body)
    return {"keycode": code}


def source_xml():
    return get("/source")


def cmd_source(args, dry):
    args, raw = pop_flag(args, "--raw")
    if operands(args):
        raise Fail("usage", "usage: android-source [--dry-run] [--raw]")
    if dry:
        return {"would_send": [{"method": "GET", "path": "/session/<id>/source"}]}
    xml = source_xml()
    if raw:
        # print only the tree so it can be piped to grep or a file
        print_text(xml)
    return {"format": "xml", "source": xml}


def print_text(text):
    # text output is often piped to head; a closed pipe is not an error
    try:
        print(text)
        sys.stdout.flush()
    except BrokenPipeError:
        os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
    sys.exit(0)


def bounds(node):
    x1, y1, x2, y2 = map(int, re.findall(r"-?\d+", node.get("bounds") or "[0,0][0,0]"))
    return x1, y1, x2, y2


def tag_of(a):
    # keep a view id's package prefix: android-tap-label matches resource ids exactly, and
    # Compose test tags have no prefix to begin with
    rid = a.get("resource-id", "")
    return "" if rid.startswith("android:id/") else rid


def covering_layers(nodes, ancestors, window):
    """Map each node index to the index of a later full-screen layer drawn over its center

    Compose draws later siblings on top, so a PIN pad or scrim listed after the screen it covers
    hides that screen even though both stay in the tree. A layer is a visible node with a
    Compose test tag or a content description covering most of the window; earlier nodes that aren't its ancestors and whose center falls
    inside it are listed as under it. A translucent layer would also match, so this is a hint.
    """
    covered = {}
    for j, n in enumerate(nodes):
        x1, y1, x2, y2 = bounds(n)
        # view ids (pkg:id/name) mark ordinary layout containers, not overlays, so only Compose
        # tags and content descriptions name a layer
        named = (tag_of(n.attrib) and ":id/" not in tag_of(n.attrib)) or n.get("content-desc")
        if n.get("displayed") == "false" or (x2 - x1) * (y2 - y1) < 0.7 * window or not named:
            continue
        for i in range(j):
            if i in covered or nodes[i] in ancestors[j]:
                continue
            a1, b1, a2, b2 = bounds(nodes[i])
            cx, cy = (a1 + a2) // 2, (b1 + b2) // 2
            if x1 <= cx <= x2 and y1 <= cy <= y2:
                covered[i] = j
    return covered


def is_plain(node):
    """True for a node with nothing to act on or check: no tag, not tappable, editable or toggleable"""
    a = node.attrib
    return not (tag_of(a) or a.get("class", "").endswith("EditText") or
                any(a.get(k) == "true" for k in ("clickable", "long-clickable", "scrollable", "checkable", "checked")))


def row_label(node, parent, cx, cy):
    """Name a switch or checkbox by the visible text level with it inside the same row container"""
    container = parent.get(node)
    for _ in range(3):
        if container is None:
            break
        row = [(abs((bounds(m)[0] + bounds(m)[2]) // 2 - cx), m.get("text")) for m in container.iter()
               if m is not node and m.get("text") and m.get("displayed") != "false" and bounds(m)[1] <= cy <= bounds(m)[3]]
        if row:
            return ["row=%r" % min(row)[1]]
        container = parent.get(container)
    return []


def vis_lines(xml, show_all=False):
    """One line per visible element worth acting on: labelled, tagged, clickable, editable or scrollable"""
    root = ET.fromstring(xml.encode())
    nodes, ancestors, parent = [], [], {}

    def walk(node, chain):
        if "bounds" in node.attrib:
            nodes.append(node)
            ancestors.append(set(chain))
        for child in node:
            parent[child] = node
            walk(child, chain + ([node] if "bounds" in node.attrib else []))

    walk(root, [])
    window = int(root.get("width", "0") or 0) * int(root.get("height", "0") or 0) or max(
        ((bounds(n)[2] - bounds(n)[0]) * (bounds(n)[3] - bounds(n)[1]) for n in nodes), default=1)
    covered = covering_layers(nodes, ancestors, window)

    lines, summarized = [], set()
    for i, n in enumerate(nodes):
        a = n.attrib
        if (a.get("displayed") == "false" and not show_all) or n in summarized:
            continue
        tag = tag_of(a)
        text, desc = a.get("text", ""), a.get("content-desc", "")
        cls = a.get("class", n.tag).rsplit(".", 1)[-1]
        flags = "".join(f for f, on in (("C", a.get("clickable") == "true" or a.get("long-clickable") == "true"),
                                          ("E", cls == "EditText"), ("S", a.get("scrollable") == "true"),
                                          ("F", a.get("focused") == "true"), ("X", a.get("checked") == "true"),
                                          ("D", a.get("enabled") == "false"), ("U", i in covered)) if on)
        # Compose test tags are always worth a line; view ids (pkg:id/name) mostly name layout
        # containers, so they only count on elements that can be acted on
        actionable = flags.strip("FDXU")
        if not (text or desc or (tag and ":id/" not in tag) or actionable):
            continue
        x1, y1, x2, y2 = bounds(n)
        if x2 <= x1 or y2 <= y1:
            continue
        parts = [p for p in (
            "text=%r" % (text if len(text) <= 120 else text[:100] + "...[%d chars]" % len(text)) if text else "",
            "desc=%r" % desc if desc and desc != text else "",
            "tag=%r" % tag if tag else "") if p]
        if not parts and "C" in flags:
            # Compose puts a row's words on child nodes; name the row by them
            kids = [c for c in n.iter() if c is not n and c.get("displayed") != "false"]
            inner = []
            for c in kids:
                for t in (c.get("text"), c.get("content-desc")):
                    if t and t not in inner:
                        inner.append(t)
            joined = " | ".join(inner)
            shown = joined if len(joined) <= 160 else joined[:150]
            parts = ["contains=%r" % (shown + ("..." if shown != joined else ""))] if inner else []

            # skip child lines that would only repeat the summary; anything an agent could act
            # on, or text cut from a long summary, still gets its own line
            for c in kids:
                texts = [t for t in (c.get("text"), c.get("content-desc")) if t]
                if is_plain(c) and all(t in shown for t in texts):
                    summarized.add(c)

            if not inner:
                parts = row_label(n, parent, (x1 + x2) // 2, (y1 + y2) // 2)
        lines.append("%-14s (%d,%d) %dx%d %-3s %s" % (cls[:14], (x1 + x2) // 2, (y1 + y2) // 2, x2 - x1, y2 - y1, flags, " ".join(parts) or "[unlabeled]"))

    if covered:
        layers = sorted(set(covered.values()))
        names = ", ".join(tag_of(nodes[j].attrib) or nodes[j].get("content-desc") for j in layers)
        lines.insert(0, "# U: %d elements sit under a later full-screen layer (%s); they are probably hidden, check with android-shot" % (len(covered), names))
    return lines


def cmd_vis(args, dry):
    args, show_all = pop_flag(args, "-a")
    if operands(args):
        raise Fail("usage", "usage: android-vis [-a]  (flags: C clickable, E editable, S scrollable, F focused, X checked, D disabled, U under a later layer)")
    print_text("\n".join(vis_lines(source_xml(), show_all)))


def cmd_pin(args, dry):
    # one tree read, then coordinate taps: six find-and-click round trips take about 2 s
    args = operands(args)
    if len(args) != 1 or not args[0].isdigit():
        raise Fail("usage", "usage: android-pin [--dry-run] DIGITS")
    digits = args[0]
    if dry:
        return {"would_send": [{"method": "GET", "path": "/session/<id>/source"}] +
                [{"method": "POST", "path": "/session/<id>/appium/gestures/click", "body": "center of numberPadPin.digit.%s" % d} for d in digits]}
    # only Cove's tagged keypad counts: a lone "1" elsewhere on screen is not a key
    keys = {}
    for n in ET.fromstring(source_xml().encode()).iter():
        m = re.fullmatch(r"numberPadPin\.digit\.(\d)", n.get("resource-id", ""))
        x1, y1, x2, y2 = bounds(n)
        if m and n.get("displayed") != "false" and x2 > x1 and y2 > y1:
            keys[m.group(1)] = n
    missing = sorted(set(d for d in digits if d not in keys))
    if missing:
        raise Fail("keypad_not_visible", "no keypad digits %s on screen; check android-vis" % ",".join(missing), found=sorted(keys))
    for d in digits:
        x1, y1, x2, y2 = bounds(keys[d])
        post("/appium/gestures/click", {"offset": {"x": (x1 + x2) // 2, "y": (y1 + y2) // 2}})
        time.sleep(0.1)
    return {"typed_digits": len(digits)}


def cmd_shot(args, dry):
    usage = "usage: android-shot [--dry-run] [-s MAX_PIXELS] [--element LABEL] [--] PATH.png"
    args, max_px = pop_opt(args, "-s", 0, int, usage)
    args, label = pop_opt(args, "--element", "", str, usage)
    args = operands(args)
    if len(args) != 1:
        raise Fail("usage", usage)
    path = os.path.abspath(os.path.expanduser(args[0]))
    if dry:
        return {"would_send": [{"method": "GET", "path": "/session/<id>/" + ("element/<id>/screenshot" if label else "screenshot")}], "path": path}
    try:
        b64 = get("/element/%s/screenshot" % find_label(label)[0]) if label else get("/screenshot")
    except Fail as e:
        if SECURE_SCREEN not in e.code.replace("_", " "):
            raise
        # FLAG_SECURE screens can't be captured; that is the expected result on secret screens
        return {"path": None, "protected": True}
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(base64.b64decode(b64))
    if max_px:
        # a 1080x2400 capture costs about four times the image tokens of a 1200-pixel-tall copy
        subprocess.run(["sips", "-Z", str(max_px), path], capture_output=True, check=True)
    return {"path": path, "bytes": os.path.getsize(path), "protected": False, "max_pixels": max_px or None}


def cmd_clipboard(args, dry):
    usage = "usage: android-clipboard [--dry-run] get | set [--] TEXT"
    args = operands(args)
    if not args or args[0] not in ("get", "set") or len(args) != (2 if args[0] == "set" else 1):
        raise Fail("usage", usage)
    if args[0] == "set":
        body = {"content": base64.b64encode(args[1].encode()).decode(), "contentType": "plaintext"}
        if dry:
            return {"would_send": [{"method": "POST", "path": "/session/<id>/appium/device/set_clipboard", "body": body}]}
        post("/appium/device/set_clipboard", body)
        return {"op": "set", "chars": len(args[1])}
    if dry:
        return {"would_send": [{"method": "POST", "path": "/session/<id>/appium/device/get_clipboard", "body": {"contentType": "plaintext"}}]}
    text = base64.b64decode(post("/appium/device/get_clipboard", {"contentType": "plaintext"}) or "").decode(errors="replace")
    if not text and int(get("/appium/device/info").get("apiVersion", "0")) >= 29:
        # Android 10+ returns nothing to a background reader, so empty can't be trusted
        raise Fail("clipboard_read_blocked", "Android 10+ only lets the foreground app or keyboard read the clipboard; paste into a field and read it with android-vis instead")
    return {"op": "get", "text": text}


def cmd_launch(args, dry):
    # launching needs adb, not the server: the server has no app start endpoint
    # an unknown flag must not fall through to a force-stop that wipes the app's in-memory state
    args, activate = pop_flag(args, "--activate")
    pkgs = operands(args)
    # adb joins `shell` arguments with spaces for the device shell, so `pkg; am force-stop other`
    # would run a second command; an argument list doesn't prevent that, the grammar check does
    if len(pkgs) != 1 or not PACKAGE_NAME.fullmatch(pkgs[0]):
        raise Fail("usage", "usage: android-launch [--dry-run] [--activate] PACKAGE (an Android package name, such as org.bitcoinppl.cove)")
    pkg = pkgs[0]
    shell_pkg = shlex.quote(pkg)
    adb = ["adb"] + (["-s", SERIAL] if SERIAL else [])
    resolve = adb + ["shell", "cmd", "package", "resolve-activity", "--brief", "-c", "android.intent.category.LAUNCHER", shell_pkg]
    if dry:
        return {"would_run": ([] if activate else [" ".join(adb + ["shell", "am", "force-stop", shell_pkg])]) +
                [" ".join(resolve), " ".join(adb + ["shell", "am", "start", "-W", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-n", "<activity>"])]}
    r = subprocess.run(resolve, capture_output=True, text=True)
    if r.returncode != 0:
        raise Fail("adb_failed", (r.stderr or r.stdout).strip()[-300:] or "adb exited %d" % r.returncode,
                   fix="set ANDROID_SERIAL when several devices are attached; run uia2-doctor")
    out = r.stdout.strip().splitlines()
    activity = out[-1].strip() if out and "/" in out[-1] else ""
    if not activity:
        raise Fail("not_installed", "%s has no launcher activity on the device" % pkg)
    if not activate:
        subprocess.run(adb + ["shell", "am", "force-stop", shell_pkg], capture_output=True)
    # a MAIN/LAUNCHER intent brings an existing task forward like the home screen icon does;
    # the activity comes from the device, and a nested class name has a `$` the shell would expand
    r = subprocess.run(adb + ["shell", "am", "start", "-W", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-n", shlex.quote(activity)],
                       capture_output=True, text=True)
    if "Status: ok" not in r.stdout:
        raise Fail("launch_failed", (r.stdout + r.stderr).strip()[-300:])
    return {"package": pkg, "activity": activity, "action": "activate" if activate else "launch"}


COMMANDS = {"launch": cmd_launch, "swipe": cmd_swipe, "drag": cmd_drag, "tap": cmd_tap, "hold": cmd_hold,
            "tap-label": cmd_tap_label, "type": cmd_type, "back": cmd_back, "key": cmd_key,
            "source": cmd_source, "vis": cmd_vis, "pin": cmd_pin, "shot": cmd_shot, "clipboard": cmd_clipboard}


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
    emit(True, command="android-" + name, dry_run=dry, **result)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
