#!/bin/sh
# Extracts the SC config files needed by BindSight from the Data.p4k.
# Usage: ./extract_sc_datafiles.sh <path/to/Data.p4k>
set -eu

# Script-relative paths so the script works from any working directory.
# shellcheck disable=SC1007  # 'CDPATH= cd' is intentional: clear CDPATH for this cd only
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
STARBREAKER="$SCRIPT_DIR/starbreaker"
OUT="$SCRIPT_DIR/extracted"

P4K=${1:-}

if [ -z "$P4K" ]; then
	echo "Usage: $0 <path/to/Data.p4k>" >&2
	exit 1
fi
if [ ! -f "$P4K" ]; then
	echo "Error: p4k file not found: $P4K" >&2
	exit 1
fi
if [ ! -x "$STARBREAKER" ]; then
	echo "Error: starbreaker not found or not executable: $STARBREAKER" >&2
	exit 1
fi

# Extracts one file from the p4k and stores it flat in $OUT.
# $1: p4k-internal path (backslash-separated), rest: optional extra args (e.g. --convert cryxml)
extract_one() {
	filter=$1
	shift
	# Convert backslash path to slash path -> yields the on-disk location.
	# shellcheck disable=SC1003  # '\\' is intentional: literal backslash for tr
	relpath=$(printf '%s' "$filter" | tr '\\' '/')
	name=$(basename "$relpath")

	echo
	echo "--== EXTRACTING $name ==--"
	"$STARBREAKER" p4k extract --p4k "$P4K" -o "$OUT" --filter "$filter" "$@"
	mv "$OUT/$relpath" "$OUT/$name"
}

mkdir -p "$OUT"

extract_one 'Data\Libs\Config\defaultProfile.xml'          --convert cryxml
extract_one 'Data\Libs\Config\keybinding_localization.xml' --convert cryxml
extract_one 'Data\Localization\english\global.ini'

# Remove the leftover directory tree from the extraction.
rm -rf "$OUT/Data"

echo
echo "~~~~ EXTRACTION DONE! ~~~~"
