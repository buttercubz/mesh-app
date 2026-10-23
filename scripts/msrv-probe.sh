#!/usr/bin/env bash
# MSRV probe (owned by WP-00, DESIGN §2.2 / §2.6).
#
# Builds scripts/msrv-probe/ (every pinned dependency, outside the workspace) with
# `cargo +<pin> check`, where <pin> is the channel of rust-toolchain.toml, then prints
# the highest `rust-version` declared by any locked dependency and the dependencies
# that declare none. Exits non-zero when the pinned dependency set does not build
# with the pinned toolchain or a locked crate declares a newer rust-version.
#
# Usage: bash scripts/msrv-probe.sh [--offline] [--report-only] [--help]
#   --offline      never touch the network (used by doctor.sh)
#   --report-only  skip `cargo check`; only print the rust-version report
# Never installs a toolchain: run scripts/bootstrap.sh for that.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROBE_DIR="$ROOT/scripts/msrv-probe"
MANIFEST="$PROBE_DIR/Cargo.toml"
TOOLCHAIN_FILE="$ROOT/rust-toolchain.toml"

offline=0
report_only=0
for arg in "$@"; do
  case "$arg" in
    --offline) offline=1 ;;
    --report-only) report_only=1 ;;
    -h|--help) sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "msrv-probe: unknown argument '$arg' (see --help)" >&2; exit 2 ;;
  esac
done

say() { printf 'msrv-probe: %s\n' "$*"; }
die() { printf 'msrv-probe: FAIL: %s\n' "$*" >&2; exit 1; }

normalize() { # "1.85" -> "1.85.0" so that "1.85" and "1.85.0" compare equal
  local v="$1"
  case "$v" in *.*.*) ;; *.*) v="$v.0" ;; *) v="$v.0.0" ;; esac
  printf '%s' "$v"
}
version_gt() { # true when dotted version $1 > $2
  local a b
  a="$(normalize "$1")"
  b="$(normalize "$2")"
  [ "$a" != "$b" ] && [ "$(printf '%s\n%s\n' "$a" "$b" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n1)" = "$a" ]
}

command -v cargo >/dev/null 2>&1 || die "cargo not found. Fix: install rustup (https://rustup.rs), then bash scripts/bootstrap.sh"
command -v rustup >/dev/null 2>&1 || die "rustup not found. Fix: install rustup (https://rustup.rs), then bash scripts/bootstrap.sh"
command -v jq >/dev/null 2>&1 || die "jq not found. Fix: brew install jq"
[ -f "$TOOLCHAIN_FILE" ] || die "missing $TOOLCHAIN_FILE"
[ -f "$MANIFEST" ] || die "missing $MANIFEST"

PIN="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$TOOLCHAIN_FILE" | head -n1)"
[ -n "$PIN" ] || die "could not read the channel from rust-toolchain.toml"
PROBE_RV="$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' "$MANIFEST" | head -n1)"
[ "$PROBE_RV" = "$PIN" ] \
  || die "scripts/msrv-probe/Cargo.toml rust-version ($PROBE_RV) != rust-toolchain.toml channel ($PIN). Fix: make them equal"

if ! rustup toolchain list | grep -q "^$PIN-"; then
  die "toolchain $PIN is not installed. Fix: bash scripts/bootstrap.sh (or: rustup toolchain install $PIN --profile minimal)"
fi

say "pinned toolchain: $PIN ($(cargo "+$PIN" --version))"

cargo_flags=(--manifest-path "$MANIFEST")
[ -f "$PROBE_DIR/Cargo.lock" ] && cargo_flags+=(--locked)
[ "$offline" = 1 ] && cargo_flags+=(--offline)
export CARGO_TARGET_DIR="$ROOT/target/msrv-probe"

if [ "$report_only" = 0 ]; then
  say "cargo +$PIN check (all pinned dependencies)"
  if ! cargo "+$PIN" check --quiet "${cargo_flags[@]}"; then
    if [ "$offline" = 1 ]; then
      die "the pinned dependency set does not build offline with $PIN. Fix: bash scripts/msrv-probe.sh (online) or bash scripts/bootstrap.sh"
    fi
    die "the pinned dependency set does not build with $PIN. Fix: move rust-toolchain.toml to the current stable and update DESIGN §2.6 in the same change"
  fi
  say "check: ok"
fi

meta="$(cargo "+$PIN" metadata --quiet --format-version 1 "${cargo_flags[@]}")" \
  || die "cargo metadata failed (run online once: bash scripts/msrv-probe.sh)"

# Every locked third-party package (all platforms), sorted by name.
rows="$(printf '%s' "$meta" | jq -r '.packages[] | select(.source != null)
  | [.name, .version, (.rust_version // "-")] | @tsv' | sort -f)"
total="$(printf '%s\n' "$rows" | grep -c . || true)"

highest="0"
highest_who=""
none_list=""
while IFS=$'\t' read -r name version rv; do
  [ -n "$name" ] || continue
  if [ "$rv" = "-" ]; then
    none_list="${none_list:+$none_list, }$name $version"
    continue
  fi
  if [ "$(normalize "$rv")" = "$(normalize "$highest")" ]; then
    highest_who="$highest_who, $name $version"
  elif version_gt "$rv" "$highest"; then
    highest="$rv"
    highest_who="$name $version"
  fi
done <<< "$rows"

say "locked third-party crates: $total"
say "highest declared rust-version: $highest ($highest_who)"
say "crates declaring no rust-version: ${none_list:-none}"

if version_gt "$highest" "$PIN"; then
  die "a locked dependency declares rust-version $highest > pin $PIN. Fix: move the pin to the current stable and update DESIGN §2.6"
fi
if [ "$report_only" = 1 ]; then
  say "REPORT (check skipped)"
else
  say "PASS: the pinned dependency set builds with $PIN"
fi
