#!/bin/sh
# Write the updater feed (`latest.json`) for one release to stdout, from the
# signed bundles `tauri build` produced: the Windows installer
# (`BindSight_<version>_x64-setup.exe` + `.sig`) and the Linux AppImage
# (`BindSight_<version>_amd64.AppImage` + `.sig`), collected into one folder.
# A platform whose files are missing is left out of the feed.
#
# Usage: scripts/latest-json.sh <version> <assets dir> [notes file] > latest.json
#
# Upload latest.json to the GitHub release `v<version>` next to the assets;
# the app reads it from releases/latest/download/latest.json (stable) or via
# the beta-version release (beta).
set -eu

if [ $# -lt 2 ]; then
	echo "usage: $0 <version> <assets dir> [notes file]" >&2
	exit 1
fi
version=$1
dir=$2
notes=${3:-}
base="https://github.com/w00zla/BindSight/releases/download/v$version"

# JSON string escaping for the notes: backslash, quote, newline, tab.
escape() {
	sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/	/\\t/g' | awk 'NR > 1 { printf "\\n" } { printf "%s", $0 }'
}

if [ -n "$notes" ]; then
	notes_json=$(escape <"$notes")
else
	notes_json=""
fi

platforms=""
add_platform() {
	key=$1
	asset=$2
	if [ ! -f "$dir/$asset" ] || [ ! -f "$dir/$asset.sig" ]; then
		echo "skipping $key: $asset or its .sig not in $dir" >&2
		return
	fi
	sig=$(cat "$dir/$asset.sig")
	entry="    \"$key\": { \"signature\": \"$sig\", \"url\": \"$base/$asset\" }"
	if [ -n "$platforms" ]; then
		platforms="$platforms,
$entry"
	else
		platforms="$entry"
	fi
}

add_platform windows-x86_64 "BindSight_${version}_x64-setup.exe"
add_platform linux-x86_64 "BindSight_${version}_amd64.AppImage"

if [ -z "$platforms" ]; then
	echo "no signed bundle found in $dir" >&2
	exit 1
fi

cat <<JSON
{
  "version": "$version",
  "notes": "$notes_json",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
$platforms
  }
}
JSON
