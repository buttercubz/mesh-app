#!/usr/bin/env bash
# Idempotent toolchain bootstrap for this Mac (owned by WP-00, DESIGN §2.2).
#
#   bash scripts/bootstrap.sh
#
# Installs only what is missing (a second run installs nothing):
#   - the Rust toolchain pinned in rust-toolchain.toml (a separate rustup toolchain
#     from "stable"), its rustfmt/clippy components and the six targets;
#   - cargo-ndk ^4 (only when absent);
#   - pnpm through corepack (only when pnpm is absent and corepack exists);
#   - once the workspace exists: libopus sources (`cargo xtask fetch-opus`), the pnpm
#     workspace (`pnpm install --frozen-lockfile`) and Playwright Chromium.
# Runs scripts/msrv-probe.sh. Never needs system cmake, wasm-bindgen-cli, wasm-pack or
# uniffi-bindgen. System prerequisites it cannot install (Xcode, Android SDK/NDK/cmake,
# JDK 21, Node, jq) are checked and reported with a fix-it line (exit 1).
#
# Environment:
#   MESH_PNPM_WRAPPER  optional command prefix for pnpm mutations (e.g. a lock wrapper)
#   MESH_SKIP_MSRV=1   skip the MSRV probe (not recommended)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

installs=0
problems=()

ok()      { printf '  ok       %s\n' "$*"; }
act()     { printf '  install  %s\n' "$*"; installs=$((installs + 1)); }
note()    { printf '  note     %s\n' "$*"; }
problem() { printf '  MISSING  %s\n' "$1"; problems+=("$1 -> fix: $2"); }
section() { printf '\n== %s\n' "$*"; }
die()     { printf 'bootstrap: FAIL: %s\n' "$*" >&2; exit 1; }

# Reads a TOML string array (one level, e.g. `targets = [ ... ]`) from rust-toolchain.toml.
toml_array() {
  awk -v key="$1" '
    $0 ~ "^"key" *= *\\[" { on = 1 }
    on { line = $0; while (match(line, /"[^"]+"/)) { print substr(line, RSTART + 1, RLENGTH - 2); line = substr(line, RSTART + RLENGTH) } }
    on && /\]/ { exit }' rust-toolchain.toml
}

# ---------------------------------------------------------------------------- Rust
section "Rust toolchain"
command -v rustup >/dev/null 2>&1 \
  || die "rustup not found. Fix: install rustup from https://rustup.rs, then re-run bash scripts/bootstrap.sh"
[ -f rust-toolchain.toml ] || die "rust-toolchain.toml missing"

PIN="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml | head -n1)"
[ -n "$PIN" ] || die "could not read the channel from rust-toolchain.toml"
TARGETS=()
while IFS= read -r t; do [ -n "$t" ] && TARGETS+=("$t"); done < <(toml_array targets)
COMPONENTS=()
while IFS= read -r c; do [ -n "$c" ] && COMPONENTS+=("$c"); done < <(toml_array components)

if rustup toolchain list | grep -q "^$PIN-"; then
  ok "toolchain $PIN"
else
  act "toolchain $PIN (rustup toolchain install $PIN --profile minimal)"
  rustup toolchain install "$PIN" --profile minimal --no-self-update
fi

installed_components="$(rustup component list --installed --toolchain "$PIN")"
for c in "${COMPONENTS[@]}"; do
  if printf '%s\n' "$installed_components" | grep -Eq "^$c(-|$)"; then
    ok "component $c"
  else
    act "component $c"
    rustup component add "$c" --toolchain "$PIN"
  fi
done

installed_targets="$(rustup target list --installed --toolchain "$PIN")"
for t in "${TARGETS[@]}"; do
  if printf '%s\n' "$installed_targets" | grep -qx "$t"; then
    ok "target $t"
  else
    act "target $t"
    rustup target add "$t" --toolchain "$PIN"
  fi
done

# cargo-ndk ^4 (present today at ~/.cargo/bin; kept for fresh machines).
cargo_ndk_version() {
  local bin
  bin="$(command -v cargo-ndk 2>/dev/null || true)"
  [ -n "$bin" ] || bin="$HOME/.cargo/bin/cargo-ndk"
  [ -x "$bin" ] || return 1
  # The binary refuses direct calls; it must run as a cargo subcommand.
  PATH="$(dirname "$bin"):$PATH" cargo ndk --version 2>/dev/null | awk '/^cargo-ndk /{print $2}'
}
ndk_v="$(cargo_ndk_version || true)"
if [ "${ndk_v%%.*}" = "4" ]; then
  ok "cargo-ndk $ndk_v"
else
  act "cargo-ndk ^4 (found: ${ndk_v:-none})"
  cargo "+$PIN" install cargo-ndk --version '^4' --locked
fi

# ---------------------------------------------------------------- platform SDKs
section "Platform SDKs (checked, not installed)"
# shellcheck source=scripts/env.sh
source scripts/env.sh

if [ -d "$ANDROID_HOME" ]; then ok "ANDROID_HOME $ANDROID_HOME"; else
  problem "Android SDK at $ANDROID_HOME" "install Android Studio (SDK Manager) or set ANDROID_HOME"; fi
sdkm="sdkmanager"
[ -x "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" ] && sdkm="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
if [ -x "$CC_aarch64_linux_android" ] && [ -x "$CC_x86_64_linux_android" ] && [ -x "$AR_aarch64_linux_android" ]; then
  ok "NDK $MESH_ANDROID_NDK_VERSION (API $MESH_ANDROID_API clang, llvm-ar)"
else
  problem "NDK $MESH_ANDROID_NDK_VERSION" "$sdkm \"ndk;$MESH_ANDROID_NDK_VERSION\""
fi
if [ -x "$ANDROID_HOME/cmake/$MESH_ANDROID_CMAKE_VERSION/bin/cmake" ] && [ -x "$ANDROID_HOME/cmake/$MESH_ANDROID_CMAKE_VERSION/bin/ninja" ]; then
  ok "SDK cmake $MESH_ANDROID_CMAKE_VERSION (+ ninja)"
else
  problem "SDK cmake $MESH_ANDROID_CMAKE_VERSION" "$sdkm \"cmake;$MESH_ANDROID_CMAKE_VERSION\""
fi
if [ -n "${JAVA_HOME:-}" ] && "$JAVA_HOME/bin/java" -version 2>&1 | grep -q "version \"$MESH_JAVA_VERSION[.\"]"; then
  ok "JDK $MESH_JAVA_VERSION at $JAVA_HOME"
else
  problem "JDK $MESH_JAVA_VERSION" "brew install openjdk@$MESH_JAVA_VERSION (must be listed by /usr/libexec/java_home -V)"
fi
if command -v xcodebuild >/dev/null 2>&1 && xcodebuild -version >/dev/null 2>&1; then
  ok "$(xcodebuild -version | head -n1)"
else
  problem "Xcode" "install Xcode 27.1 and run: sudo xcode-select -s /Applications/Xcode.app"
fi
if command -v jq >/dev/null 2>&1; then ok "jq"; else problem "jq" "brew install jq"; fi
if sim="$(bash scripts/pick-sim.sh 2>/dev/null)"; then ok "iPhone simulator $sim"; else
  problem "iPhone simulator (iOS >= 26.0)" "Xcode > Settings > Components: install an iOS 26+ runtime"; fi

# ------------------------------------------------------------------ Node / pnpm
section "Node and pnpm"
version_ge() { # dotted $1 >= $2
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | head -n1)" = "$2" ]
}
NODE_MIN="22.18.0"
if command -v node >/dev/null 2>&1; then
  node_v="$(node --version | sed 's/^v//')"
  if version_ge "$node_v" "$NODE_MIN"; then ok "node $node_v"; else
    problem "node >= $NODE_MIN (found $node_v)" "brew install node (or nvm install 22)"; fi
else
  problem "node >= $NODE_MIN" "brew install node"
fi

PNPM_WANT=""
if [ -f package.json ] && command -v jq >/dev/null 2>&1; then
  PNPM_WANT="$(jq -r '.packageManager // "" | select(startswith("pnpm@")) | sub("^pnpm@"; "") | sub("\\+.*$"; "")' package.json)"
fi
if ! command -v pnpm >/dev/null 2>&1 && command -v corepack >/dev/null 2>&1; then
  act "pnpm via corepack (corepack enable pnpm)"
  corepack enable pnpm
fi
if command -v pnpm >/dev/null 2>&1; then
  pnpm_v="$(pnpm --version)"
  ok "pnpm $pnpm_v"
  if [ -n "$PNPM_WANT" ] && [ "$pnpm_v" != "$PNPM_WANT" ]; then
    note "package.json pins pnpm@$PNPM_WANT (pnpm switches to it per packageManager)"
  fi
else
  problem "pnpm" "npm install -g pnpm@${PNPM_WANT:-11.5.3} (Node >= 25 ships without corepack)"
fi

# ------------------------------------------------------------------ MSRV probe
section "MSRV probe"
if [ "${MESH_SKIP_MSRV:-0}" = "1" ]; then
  note "skipped (MESH_SKIP_MSRV=1)"
elif command -v jq >/dev/null 2>&1; then
  bash scripts/msrv-probe.sh || die "msrv-probe failed (see above)"
else
  problem "MSRV probe (needs jq)" "brew install jq, then bash scripts/msrv-probe.sh"
fi

# ------------------------------------------------- workspace-dependent steps
section "Workspace"
if [ -f tools/xtask/Cargo.toml ]; then
  cargo xtask fetch-opus || die "cargo xtask fetch-opus failed"
  ok "cargo xtask fetch-opus"
else
  note "tools/xtask not present yet (WP-01): skipping cargo xtask fetch-opus"
fi

# Playwright Chromium is installed when every browser directory reported by
# `playwright install --dry-run chromium` exists.
playwright_chromium_present() {
  local locs loc
  locs="$(pnpm --filter @meshapp/mobile exec playwright install --dry-run chromium 2>/dev/null \
    | sed -n 's/^ *Install location: *//p')" || return 1
  [ -n "$locs" ] || return 1
  while IFS= read -r loc; do [ -d "$loc" ] || return 1; done <<< "$locs"
}

if [ -f apps/mobile/package.json ]; then
  if ! command -v pnpm >/dev/null 2>&1; then
    problem "pnpm install (apps/mobile present)" "install pnpm, then re-run bootstrap"
  else
    wrapper=()
    if [ -n "${MESH_PNPM_WRAPPER:-}" ]; then read -r -a wrapper <<< "$MESH_PNPM_WRAPPER"; fi
    # A no-op when node_modules already matches the lockfile.
    ${wrapper[@]+"${wrapper[@]}"} pnpm install --frozen-lockfile || die "pnpm install --frozen-lockfile failed"
    ok "pnpm install --frozen-lockfile"
    if playwright_chromium_present; then
      ok "Playwright Chromium"
    else
      act "Playwright Chromium"
      pnpm --filter @meshapp/mobile exec playwright install chromium \
        || die "playwright install chromium failed"
    fi
  fi
else
  note "apps/mobile not present yet (WP-18): skipping pnpm install and Playwright"
fi

# ----------------------------------------------------------------------- summary
section "Summary"
if [ "${#problems[@]}" -gt 0 ]; then
  printf 'bootstrap: %d prerequisite(s) missing:\n' "${#problems[@]}"
  for p in "${problems[@]}"; do printf '  - %s\n' "$p"; done
  exit 1
fi
if [ "$installs" -eq 0 ]; then
  echo "bootstrap: done, nothing to install (already bootstrapped)"
else
  echo "bootstrap: done, $installs item(s) installed"
fi
echo "next: source scripts/env.sh   (then bash scripts/doctor.sh)"
