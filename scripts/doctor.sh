#!/usr/bin/env bash
# Toolchain doctor (owned by WP-00, DESIGN §2.2). Prints every required item with
# "ok" or "MISSING" plus a fix-it line, the MSRV report of scripts/msrv-probe.sh and
# the Playwright Chromium state. NEVER installs or downloads anything (rustup
# auto-install is disabled and the MSRV probe runs --offline).
# Exit 0 when every required item is present, 1 otherwise.
#
# Usage: bash scripts/doctor.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT" || exit 1
export RUSTUP_AUTO_INSTALL=0

missing=0
warnings=0
ok()      { printf '  ok       %s\n' "$*"; }
miss()    { printf '  MISSING  %s\n           fix: %s\n' "$1" "$2"; missing=$((missing + 1)); }
warn()    { printf '  warn     %s\n           fix: %s\n' "$1" "$2"; warnings=$((warnings + 1)); }
info()    { printf '  info     %s\n' "$*"; }
section() { printf '\n== %s\n' "$*"; }
version_ge() { # dotted $1 >= $2
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | head -n1)" = "$2" ]
}
toml_array() {
  awk -v key="$1" '
    $0 ~ "^"key" *= *\\[" { on = 1 }
    on { line = $0; while (match(line, /"[^"]+"/)) { print substr(line, RSTART + 1, RLENGTH - 2); line = substr(line, RSTART + RLENGTH) } }
    on && /\]/ { exit }' rust-toolchain.toml
}

# shellcheck source=scripts/env.sh
source scripts/env.sh

# ------------------------------------------------------------------------- Rust
section "Rust"
PIN="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml 2>/dev/null | head -n1)"
if ! command -v rustup >/dev/null 2>&1; then
  miss "rustup" "install rustup from https://rustup.rs, then bash scripts/bootstrap.sh"
elif [ -z "$PIN" ]; then
  miss "rust-toolchain.toml channel" "restore rust-toolchain.toml (WP-00)"
elif ! rustup toolchain list | grep -q "^$PIN-"; then
  miss "Rust toolchain $PIN (pinned by rust-toolchain.toml)" "bash scripts/bootstrap.sh (or: rustup toolchain install $PIN --profile minimal)"
else
  ok "Rust toolchain $PIN ($(rustc "+$PIN" --version 2>/dev/null))"
  comps="$(rustup component list --installed --toolchain "$PIN" 2>/dev/null)"
  while IFS= read -r c; do
    [ -n "$c" ] || continue
    if printf '%s\n' "$comps" | grep -Eq "^$c(-|$)"; then ok "component $c"
    else miss "component $c ($PIN)" "rustup component add $c --toolchain $PIN"; fi
  done < <(toml_array components)
  tgts="$(rustup target list --installed --toolchain "$PIN" 2>/dev/null)"
  while IFS= read -r t; do
    [ -n "$t" ] || continue
    if printf '%s\n' "$tgts" | grep -qx "$t"; then ok "target $t"
    else miss "target $t ($PIN)" "rustup target add $t --toolchain $PIN"; fi
  done < <(toml_array targets)
fi
ndk_bin="$(command -v cargo-ndk 2>/dev/null || echo "$HOME/.cargo/bin/cargo-ndk")"
if [ -x "$ndk_bin" ] && [ -n "$PIN" ] && rustup toolchain list | grep -q "^$PIN-"; then
  ndk_v="$(PATH="$(dirname "$ndk_bin"):$PATH" cargo ndk --version 2>/dev/null | awk '/^cargo-ndk /{print $2}')"
  if [ "${ndk_v%%.*}" = "4" ]; then ok "cargo-ndk $ndk_v"
  else miss "cargo-ndk ^4 (found ${ndk_v:-unknown})" "cargo install cargo-ndk --version ^4 --locked"; fi
elif [ -x "$ndk_bin" ]; then
  info "cargo-ndk present (version check needs the pinned toolchain)"
else
  miss "cargo-ndk ^4" "cargo install cargo-ndk --version ^4 --locked (or bash scripts/bootstrap.sh)"
fi
for tool in wasm-pack wasm-bindgen uniffi-bindgen; do
  command -v "$tool" >/dev/null 2>&1 && info "$tool is installed globally but not used (the build never needs it)"
done

# ---------------------------------------------------------------------- Android
section "Android (scripts/env.sh)"
sdkm="\$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
if [ -d "$ANDROID_HOME" ]; then ok "ANDROID_HOME=$ANDROID_HOME"
else miss "Android SDK at $ANDROID_HOME" "install Android Studio (SDK Manager) or export ANDROID_HOME"; fi
if [ -d "$ANDROID_NDK_HOME" ]; then ok "ANDROID_NDK_HOME=$ANDROID_NDK_HOME"
else miss "NDK $MESH_ANDROID_NDK_VERSION" "$sdkm \"ndk;$MESH_ANDROID_NDK_VERSION\""; fi
for v in CC_aarch64_linux_android AR_aarch64_linux_android CC_x86_64_linux_android AR_x86_64_linux_android; do
  p="${!v}"
  if [ -x "$p" ]; then ok "$v"
  else miss "$v ($p)" "$sdkm \"ndk;$MESH_ANDROID_NDK_VERSION\""; fi
done
cm="$ANDROID_HOME/cmake/$MESH_ANDROID_CMAKE_VERSION/bin"
if [ -x "$cm/cmake" ] && [ -x "$cm/ninja" ]; then ok "SDK cmake $MESH_ANDROID_CMAKE_VERSION + ninja on PATH"
else miss "SDK cmake $MESH_ANDROID_CMAKE_VERSION (+ ninja)" "$sdkm \"cmake;$MESH_ANDROID_CMAKE_VERSION\""; fi

# ------------------------------------------------------------------------- Java
section "Java"
if [ -n "${JAVA_HOME:-}" ] && "$JAVA_HOME/bin/java" -version 2>&1 | grep -q "version \"$MESH_JAVA_VERSION[.\"]"; then
  ok "JDK $MESH_JAVA_VERSION (JAVA_HOME=$JAVA_HOME)"
else
  miss "JDK $MESH_JAVA_VERSION (via /usr/libexec/java_home -v $MESH_JAVA_VERSION)" "brew install openjdk@$MESH_JAVA_VERSION"
fi

# ------------------------------------------------------------------------ Apple
section "Xcode / iOS simulators"
if command -v xcodebuild >/dev/null 2>&1 && xv="$(xcodebuild -version 2>/dev/null | head -n1)"; then ok "$xv"
else miss "Xcode" "install Xcode 27.1, then: sudo xcode-select -s /Applications/Xcode.app"; fi
if command -v jq >/dev/null 2>&1; then ok "jq"; else miss "jq" "brew install jq"; fi
if udid="$(bash scripts/pick-sim.sh 2>/dev/null)"; then ok "iPhone simulator (scripts/pick-sim.sh): $udid"
else miss "available iPhone simulator, iOS >= 26.0" "Xcode > Settings > Components: install an iOS 26+ runtime"; fi

# -------------------------------------------------------------------- Node/pnpm
section "Node / pnpm"
NODE_MIN="22.18.0"
if command -v node >/dev/null 2>&1; then
  node_v="$(node --version | sed 's/^v//')"
  if version_ge "$node_v" "$NODE_MIN"; then ok "node $node_v"
  else miss "node >= $NODE_MIN (found $node_v)" "brew install node"; fi
else
  miss "node >= $NODE_MIN" "brew install node"
fi
want=""
[ -f package.json ] && command -v jq >/dev/null 2>&1 \
  && want="$(jq -r '.packageManager // "" | select(startswith("pnpm@")) | sub("^pnpm@"; "") | sub("\\+.*$"; "")' package.json)"
if command -v pnpm >/dev/null 2>&1; then
  pv="$(pnpm --version 2>/dev/null)"
  ok "pnpm $pv"
  [ -n "$want" ] && [ "$pv" != "$want" ] && info "package.json pins pnpm@$want (pnpm switches per packageManager)"
else
  miss "pnpm" "corepack enable pnpm, or: npm install -g pnpm@${want:-11.5.3}"
fi

# ------------------------------------------------------------------- MSRV probe
section "MSRV probe (scripts/msrv-probe.sh --offline)"
if [ -z "$PIN" ] || ! rustup toolchain list 2>/dev/null | grep -q "^$PIN-"; then
  miss "MSRV probe (pinned toolchain absent)" "bash scripts/bootstrap.sh"
elif ! command -v jq >/dev/null 2>&1; then
  miss "MSRV probe (needs jq)" "brew install jq"
elif probe_out="$(bash scripts/msrv-probe.sh --offline 2>&1)"; then
  printf '%s\n' "$probe_out" | grep -E 'highest declared|declaring no|locked third-party' | sed 's/^msrv-probe: /  info     /'
  ok "pinned dependency set builds with $PIN"
else
  printf '%s\n' "$probe_out" | tail -n 5 | sed 's/^/           /'
  miss "MSRV probe" "bash scripts/msrv-probe.sh (online; moves crates into the cache) or bash scripts/bootstrap.sh"
fi

# -------------------------------------------------------------------- Workspace
section "Workspace (items appear as work packages land)"
if [ -f tools/xtask/Cargo.toml ]; then
  if ls third_party/opus/opus-1.6.1/.ok >/dev/null 2>&1 || ls third_party/opus/.ok >/dev/null 2>&1; then
    ok "libopus 1.6.1 sources fetched (stamp present)"
  else
    warn "libopus 1.6.1 stamp not found (expected once WP-06 lands)" "source scripts/env.sh && cargo xtask fetch-opus"
  fi
else
  info "tools/xtask not present yet (WP-01): fetch-opus check skipped"
fi
playwright_chromium_present() {
  local locs loc
  locs="$(pnpm --filter @meshapp/mobile exec playwright install --dry-run chromium 2>/dev/null \
    | sed -n 's/^ *Install location: *//p')" || return 1
  [ -n "$locs" ] || return 1
  while IFS= read -r loc; do [ -d "$loc" ] || return 1; done <<< "$locs"
}
if [ -f apps/mobile/package.json ]; then
  if [ -d node_modules ] && [ -d apps/mobile/node_modules ]; then ok "pnpm workspace installed"
  else miss "pnpm workspace dependencies" "pnpm install --frozen-lockfile"; fi
  if command -v pnpm >/dev/null 2>&1 && playwright_chromium_present; then ok "Playwright Chromium"
  else miss "Playwright Chromium (e2e)" "pnpm --filter @meshapp/mobile exec playwright install chromium"; fi
else
  info "apps/mobile not present yet (WP-18): pnpm workspace and Playwright Chromium checks skipped"
fi

# ---------------------------------------------------------------- Optional items
section "Optional (nightly fuzz job only)"
if rustup toolchain list 2>/dev/null | grep -q '^nightly-'; then info "nightly toolchain present"
else info "nightly toolchain absent (optional: rustup toolchain install nightly)"; fi
if command -v cargo-fuzz >/dev/null 2>&1; then info "cargo-fuzz present"
else info "cargo-fuzz absent (optional: cargo install cargo-fuzz --locked)"; fi

# ----------------------------------------------------------------------- Result
section "Result"
if [ "$missing" -gt 0 ]; then
  echo "doctor: $missing required item(s) missing, $warnings warning(s)"
  exit 1
fi
echo "doctor: all required items present ($warnings warning(s))"
