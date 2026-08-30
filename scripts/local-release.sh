#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if ! command -v cargo >/dev/null 2>&1 && [[ -n "${HOME:-}" && -r "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

binary_name="devdeck"
release_binary="$repo_root/target/release/$binary_name"
platform="$(uname -s)"
codesign_identity="${CODESIGN_IDENTITY:--}"

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: $1 was not found on PATH" >&2
    exit 1
  fi
}

default_install_dir() {
  if [[ -n "${DEVDECK_INSTALL_DIR:-}" ]]; then
    printf '%s\n' "$DEVDECK_INSTALL_DIR"
  elif [[ -n "${HOME:-}" ]]; then
    printf '%s\n' "$HOME/.local/bin"
  else
    echo "error: set DEVDECK_INSTALL_DIR or HOME before running this script" >&2
    exit 1
  fi
}

install_dir="$(default_install_dir)"
install_path="$install_dir/$binary_name"

check_quality() {
  run cargo fmt --all -- --check
  run cargo clippy --all-targets --all-features --locked -- -D warnings
  run cargo test --locked
}

build_release() {
  run cargo build --release --locked
}

sign_release_binary() {
  if [[ "$platform" == "Darwin" ]]; then
    require_command codesign

    run codesign --force --sign "$codesign_identity" "$release_binary"
    run codesign --verify --verbose "$release_binary"
  else
    echo "info: skipping codesign on $platform" >&2
  fi
}

install_release_binary() {
  run mkdir -p "$install_dir"
  run install -m 755 "$release_binary" "$install_path"

  if [[ "$platform" == "Darwin" ]]; then
    run codesign --verify --verbose "$install_path"
  fi
}

main() {
  require_command cargo
  require_command install

  check_quality
  build_release
  sign_release_binary
  install_release_binary

  printf '\nInstalled %s\n' "$install_path"
}

main "$@"
