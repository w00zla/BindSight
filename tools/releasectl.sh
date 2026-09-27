#!/bin/sh
# Start releasectl, the TUI for BindSight's channels and releases.
# Creates the venv at tools/releasectl/.venv on first use and reinstalls
# the requirements whenever requirements.txt changes.
#
# Usage: tools/releasectl.sh [--dry-run]
set -eu

# shellcheck disable=SC1007  # 'CDPATH= cd' is intentional: clear CDPATH for this cd only
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tool="$root/tools/releasectl"
venv="$tool/.venv"
stamp="$venv/.requirements"

if [ ! -x "$venv/bin/python" ]; then
	python3 -m venv "$venv"
fi
if ! cmp -s "$tool/requirements.txt" "$stamp"; then
	"$venv/bin/python" -m pip install -q --disable-pip-version-check -r "$tool/requirements.txt"
	cp "$tool/requirements.txt" "$stamp"
fi

cd "$root"
PYTHONPATH="$tool" exec "$venv/bin/python" -m releasectl "$@"
