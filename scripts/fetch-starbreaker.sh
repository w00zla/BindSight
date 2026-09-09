#!/bin/sh
# Fetch the StarBreaker CLI binaries BindSight bundles as a Tauri sidecar
# (used at runtime to pull SC's config files out of Data.p4k).
#
# Pinned to one release; every binary is verified against its SHA256 before
# it is installed. Re-run after bumping the version/hashes below.
#
# Usage: scripts/fetch-starbreaker.sh
#
# Source / releases: https://github.com/diogotr7/StarBreaker (MIT)
set -eu

VERSION=0.3.2
BASE_URL="https://github.com/diogotr7/StarBreaker/releases/download/v$VERSION"

# shellcheck disable=SC1007  # 'CDPATH= cd' is intentional: clear CDPATH for this cd only
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
OUT="$SCRIPT_DIR/../src-tauri/binaries"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	else
		shasum -a 256 "$1" | cut -d' ' -f1
	fi
}

# $1: release asset, $2: file inside the archive, $3: sidecar name (Tauri
# target-triple naming), $4: expected SHA256 of the extracted binary
fetch_one() {
	asset=$1
	inner=$2
	target=$3
	expected=$4

	echo "--== $asset ==--"
	curl -fsSL --retry 3 -o "$TMP/$asset" "$BASE_URL/$asset"
	case $asset in
	*.tar.gz) tar -xzf "$TMP/$asset" -C "$TMP" "$inner" ;;
	*.zip) unzip -oq "$TMP/$asset" "$inner" -d "$TMP" ;;
	*)
		echo "Error: unknown archive type: $asset" >&2
		exit 1
		;;
	esac

	actual=$(sha256 "$TMP/$inner")
	if [ "$actual" != "$expected" ]; then
		echo "Error: SHA256 mismatch for $inner" >&2
		echo "  expected $expected" >&2
		echo "  actual   $actual" >&2
		exit 1
	fi
	mv "$TMP/$inner" "$OUT/$target"
	chmod +x "$OUT/$target"
	echo "  -> $OUT/$target"
}

mkdir -p "$OUT"

fetch_one "starbreaker-cli-v$VERSION-linux-x86_64.tar.gz" starbreaker \
	starbreaker-x86_64-unknown-linux-gnu \
	93cd5a7b756131a900e3131c05c994c1de17ad4f6cf2e47321ca5967a071990d
fetch_one "starbreaker-cli-v$VERSION-windows-x86_64.zip" starbreaker.exe \
	starbreaker-x86_64-pc-windows-msvc.exe \
	82439e45cd5337f058f06ded63ee633bea8de8e4d75525f5a083aa7955b91a10

echo
echo "~~~~ DONE: StarBreaker v$VERSION installed to src-tauri/binaries/ ~~~~"
