# shellcheck shell=sh
# Build environment for mesh-app (owned by WP-00, DESIGN §2.2).
#
#   source scripts/env.sh
#
# Side-effect free: only exports variables. It never installs, downloads, prints,
# changes directory, or alters the caller's shell options (works with `set -eu` in
# bash and in zsh). Sourcing it twice yields the same environment (PATH entries are
# not duplicated). Missing tools are reported by `bash scripts/doctor.sh`.
#
# Exports:
#   ANDROID_HOME (default ~/Library/Android/sdk), ANDROID_NDK_HOME / ANDROID_NDK_ROOT
#   (pinned NDK 27.1.12297006), JAVA_HOME (JDK 21 via /usr/libexec/java_home),
#   PATH += SDK cmake 3.22.1 (cmake + ninja, front) and the NDK llvm bin (end),
#   CC_/AR_ for aarch64-linux-android and x86_64-linux-android (API 31 clang, llvm-ar),
#   CARGO_TARGET_{AARCH64,X86_64}_LINUX_ANDROID_LINKER, MESH_APP_ROOT.

MESH_ANDROID_NDK_VERSION="27.1.12297006"
MESH_ANDROID_CMAKE_VERSION="3.22.1"
MESH_ANDROID_API="31"
MESH_JAVA_VERSION="21"
export MESH_ANDROID_NDK_VERSION MESH_ANDROID_CMAKE_VERSION MESH_ANDROID_API MESH_JAVA_VERSION

# Repository root (bash: BASH_SOURCE; zsh: $0 is the sourced file).
_mesh_env_file="${BASH_SOURCE:-}"
[ -n "$_mesh_env_file" ] || _mesh_env_file="${0:-}"
case "$_mesh_env_file" in
  */scripts/env.sh|scripts/env.sh)
    MESH_APP_ROOT="$(CDPATH='' cd -- "$(dirname -- "$_mesh_env_file")/.." 2>/dev/null && pwd)"
    export MESH_APP_ROOT
    ;;
esac

# Prepend / append a directory to PATH once.
_mesh_path_prepend() {
  case ":${PATH:-}:" in
    *":$1:"*) ;;
    *) PATH="$1${PATH:+:$PATH}" ;;
  esac
}
_mesh_path_append() {
  case ":${PATH:-}:" in
    *":$1:"*) ;;
    *) PATH="${PATH:+$PATH:}$1" ;;
  esac
}

# --- Android SDK / NDK -------------------------------------------------------------
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$MESH_ANDROID_NDK_VERSION"   # pinned (convLink pitfall)
ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export ANDROID_HOME ANDROID_NDK_HOME ANDROID_NDK_ROOT

case "$(uname -s 2>/dev/null)" in
  Linux) _mesh_ndk_host="linux-x86_64" ;;
  *) _mesh_ndk_host="darwin-x86_64" ;;   # macOS NDKs ship universal binaries under this name
esac
_mesh_ndk_bin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$_mesh_ndk_host/bin"

CC_aarch64_linux_android="$_mesh_ndk_bin/aarch64-linux-android${MESH_ANDROID_API}-clang"
AR_aarch64_linux_android="$_mesh_ndk_bin/llvm-ar"
CC_x86_64_linux_android="$_mesh_ndk_bin/x86_64-linux-android${MESH_ANDROID_API}-clang"
AR_x86_64_linux_android="$_mesh_ndk_bin/llvm-ar"
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$CC_x86_64_linux_android"
export CC_aarch64_linux_android AR_aarch64_linux_android
export CC_x86_64_linux_android AR_x86_64_linux_android
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER

# cmake + ninja for the Oboe shim come from the SDK (no system cmake required).
_mesh_path_prepend "$ANDROID_HOME/cmake/$MESH_ANDROID_CMAKE_VERSION/bin"
# NDK tools at the END of PATH so Apple's unprefixed clang/ld keep priority; this
# is what makes the bare linker names in .cargo/config.toml resolve.
_mesh_path_append "$_mesh_ndk_bin"

# --- Rust (rustup proxies honour rust-toolchain.toml) -------------------------------
if [ -d "$HOME/.cargo/bin" ]; then
  _mesh_path_prepend "$HOME/.cargo/bin"
fi

# --- JDK 21 (AGP 8.13 / Capacitor 8 Android) ----------------------------------------
if [ -x /usr/libexec/java_home ]; then
  _mesh_java_home="$(/usr/libexec/java_home -v "$MESH_JAVA_VERSION" 2>/dev/null || true)"
  if [ -n "$_mesh_java_home" ]; then
    JAVA_HOME="$_mesh_java_home"
    export JAVA_HOME
  fi
fi

export PATH
unset _mesh_env_file _mesh_ndk_host _mesh_ndk_bin _mesh_java_home
unset -f _mesh_path_prepend _mesh_path_append
