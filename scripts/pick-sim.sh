#!/usr/bin/env bash
# Prints exactly one UDID of an available iPhone simulator on stdout (owned by WP-00).
# Exit 1 (message on stderr) when none is available. Device names are never
# hard-coded: the choice comes from `xcrun simctl list devices available -j`.
#
# Order of preference:
#   1. $MESH_SIM_UDID, when it names an available iPhone simulator;
#   2. a booted iPhone simulator;
#   3. the newest iOS runtime >= $MESH_SIM_MIN_IOS (default 26.0, the deployment target),
#      then device name, then UDID (deterministic).
#
# Usage: bash scripts/pick-sim.sh            (e.g. xcodebuild -destination "id=$(bash scripts/pick-sim.sh)")
set -euo pipefail
# With pipefail, never pipe a command into a reader that may exit early (head, grep -q):
# the writer can die of SIGPIPE (exit 141) and fail the check (xcodebuild -version writes
# its two lines ~8 ms apart; CI reported Xcode MISSING). Match on captured output instead.

MIN_IOS="${MESH_SIM_MIN_IOS:-26.0}"
min_major="${MIN_IOS%%.*}"
min_minor="${MIN_IOS#*.}"
[ "$min_minor" = "$MIN_IOS" ] && min_minor=0

fail() { printf 'pick-sim: %s\n' "$*" >&2; exit 1; }

command -v xcrun >/dev/null 2>&1 || fail "xcrun not found. Fix: install Xcode and run: sudo xcode-select -s /Applications/Xcode.app"
command -v jq >/dev/null 2>&1 || fail "jq not found. Fix: brew install jq"

json="$(xcrun simctl list devices available -j 2>/dev/null)" \
  || fail "xcrun simctl failed. Fix: open Xcode once (first-launch setup) or run: xcodebuild -runFirstLaunch"

# One TSV row per candidate, already sorted by preference.
candidates="$(printf '%s' "$json" | jq -r \
  --argjson min_major "$min_major" --argjson min_minor "$min_minor" '
  [ .devices | to_entries[]
    | .key as $rt
    | ($rt | capture("SimRuntime\\.iOS-(?<maj>[0-9]+)-(?<min>[0-9]+)(-(?<pat>[0-9]+))?")?) as $v
    | .value[]
    | select(.isAvailable == true)
    | select((.deviceTypeIdentifier // "") | test("\\.iPhone"))
    | { udid, name,
        booted: (.state == "Booted"),
        maj: ($v.maj | tonumber), min: ($v.min | tonumber), pat: (($v.pat // "0") | tonumber) }
    | select(.maj > $min_major or (.maj == $min_major and .min >= $min_minor))
  ]
  | sort_by([ (if .booted then 0 else 1 end), -.maj, -.min, -.pat, .name, .udid ])
  | .[] | [.udid, .name, "iOS \(.maj).\(.min)", (if .booted then "booted" else "shutdown" end)] | @tsv')"

[ -n "$candidates" ] \
  || fail "no available iPhone simulator with iOS >= $MIN_IOS. Fix: Xcode > Settings > Components (install an iOS runtime), or: xcrun simctl create \"iPhone\" <device-type> <runtime>"

if [ -n "${MESH_SIM_UDID:-}" ]; then
  if grep -qx "$MESH_SIM_UDID" <<<"$(cut -f1 <<<"$candidates")"; then
    printf '%s\n' "$MESH_SIM_UDID"
    exit 0
  fi
  printf 'pick-sim: MESH_SIM_UDID=%s is not an available iPhone simulator; picking another\n' "$MESH_SIM_UDID" >&2
fi

sed -n 1p <<<"$candidates" | cut -f1
