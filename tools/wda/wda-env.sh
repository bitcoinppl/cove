# shared config for the wda-* and ios-* scripts; sourced, not executed
WDA_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WDA_VERSION="${WDA_VERSION:-v16.15.3}"
WDA_COMMIT="${WDA_COMMIT:-02cb92f0e93c8b021443bcbb8eeb80932c113220}"

# shared across worktrees: the checkout and build products are large, and the phone runs one
# runner at a time anyway
WDA_CACHE="${WDA_CACHE:-$HOME/.cache/cove-wda}"
WDA_REPO="${WDA_REPO:-$WDA_CACHE/WebDriverAgent-$WDA_VERSION}"
WDA_DERIVED="${WDA_DERIVED:-$WDA_CACHE/DerivedData}"
WDA_STATE="${WDA_STATE:-$WDA_CACHE/state}"

# Cove's development team, so the runner signs with the same account as the app
WDA_TEAM="${WDA_TEAM:-Q8UP8C53Y8}"
WDA_BUNDLE_ID="${WDA_BUNDLE_ID:-com.avencera.WebDriverAgentRunner}"
WDA_PORT="${WDA_PORT:-8100}"
WDA_BASE_URL="${WDA_BASE_URL:-http://127.0.0.1:$WDA_PORT}"
IOS_UDID="${IOS_UDID:-}"

# print a JSON object built from key=value pairs; values are strings unless the key ends in :raw
json() {
  python3 -I - "$@" <<'PY'
import json, sys
out = {}
for a in sys.argv[1:]:
    k, _, v = a.partition("=")
    if k.endswith(":raw"):
        out[k[:-4]] = json.loads(v)
    else:
        out[k] = v
print(json.dumps(out))
PY
}

die() { # die ERROR_CODE MESSAGE [extra key=value...]
  local code="$1" msg="$2"; shift 2
  json ok:raw=false "error=$code" "message=$msg" "$@"
  exit 1
}

need_udid() {
  [ -n "$IOS_UDID" ] || die missing_udid "set IOS_UDID to the device UDID (see README)"
}
