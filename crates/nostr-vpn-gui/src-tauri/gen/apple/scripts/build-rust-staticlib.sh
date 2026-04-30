#!/bin/bash

set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

case "${PLATFORM_NAME:?}" in
  iphoneos)
    rust_target="aarch64-apple-ios"
    externals_arch="arm64"
    ;;
  iphonesimulator)
    case "${NATIVE_ARCH_ACTUAL:-${CURRENT_ARCH:-arm64}}" in
      x86_64)
        rust_target="x86_64-apple-ios"
        externals_arch="x86_64"
        ;;
      *)
        rust_target="aarch64-apple-ios-sim"
        externals_arch="arm64"
        ;;
    esac
    ;;
  *)
    echo "Unsupported PLATFORM_NAME: ${PLATFORM_NAME}" >&2
    exit 1
    ;;
esac

profile_dir="debug"
if [ "${CONFIGURATION:?}" = "Release" ]; then
  profile_dir="release"
fi

repo_root="$(cd "${SRCROOT}/../../../../.." && pwd)"
target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
case "${target_root}" in
  /*) ;;
  *) target_root="${repo_root}/${target_root}" ;;
esac
artifact_dir="${SRCROOT}/Externals/${externals_arch}/${CONFIGURATION}"
artifact_path="${artifact_dir}/libapp.a"
source_path="${target_root}/${rust_target}/${profile_dir}/libnostr_vpn_gui_lib.a"

mkdir -p "${artifact_dir}"

cd "${repo_root}"
cargo_args=(build -p nostr-vpn-gui --target "${rust_target}" --features custom-protocol)
if [ "${profile_dir}" = "release" ]; then
  cargo_args+=(--release)
fi
cargo "${cargo_args[@]}"
cp "${source_path}" "${artifact_path}"
