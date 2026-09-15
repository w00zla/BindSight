#!/bin/sh
# Point README.md at one published release. Two optional tags whose content
# is replaced (as often as they occur; HTML tags, not comments — MarkText
# strips comments):
#
#   <span id="release_v">v0.13.0</span>        the version string
#   <div id="release_dls">                     the download table, with a
#                                              blank line after the opening
#   | ... |                                    and before the closing tag
#
#   </div>
#
# Run by the readme workflow after a release is published; runnable by hand.
#
# Usage: scripts/readme-updater.sh <version>
set -eu

if [ $# -ne 1 ]; then
	echo "usage: $0 <version>" >&2
	exit 1
fi
version=$1
# shellcheck disable=SC1007  # 'CDPATH= cd' is intentional: clear CDPATH for this cd only
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
readme="$root/README.md"
base="https://github.com/w00zla/BindSight/releases/download/v$version"

table="| Windows | Linux |
| --- | --- |
| **[Installer]($base/BindSight_${version}_x64-setup.exe)** · [Standalone]($base/BindSight_${version}_x64-standalone.zip) | **[AppImage]($base/BindSight_${version}_amd64.AppImage)** · [deb]($base/BindSight_${version}_amd64.deb) · [rpm]($base/BindSight-${version}-1.x86_64.rpm) |"

if ! grep -q 'id="release_v"\|id="release_dls"' "$readme"; then
	echo "README.md has no release_v / release_dls tags, nothing to do"
	exit 0
fi

V="v$version" TABLE="$table" perl -0pi -e '
	s{(<span id="release_v">).*?(</span>)}{$1$ENV{V}$2}sg;
	s{(<div id="release_dls">).*?(</div>)}{$1\n\n$ENV{TABLE}\n\n$2}sg;
' "$readme"
echo "README.md release tags set to v$version"
