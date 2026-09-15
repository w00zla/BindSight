#!/bin/sh
# Set the app version in every place it lives (Cargo.toml is the source the
# app reads; Cargo.lock and package.json follow), commit it and tag it.
# Pushing main + tag is offered, never done unasked.
#
# Usage: scripts/bump-version.sh <version>      e.g. 0.14.0
set -eu

if [ $# -ne 1 ]; then
	echo "usage: $0 <version>" >&2
	exit 1
fi
version=$1
case $version in
*[!0-9.]* | *..* | .* | *. | "") # digits and dots only, no empty parts
	echo "Error: '$version' is not a plain x.y.z version" >&2
	exit 1
	;;
esac
case $version in
*.*.*) ;;
*)
	echo "Error: '$version' is not a plain x.y.z version" >&2
	exit 1
	;;
esac

# shellcheck disable=SC1007  # 'CDPATH= cd' is intentional: clear CDPATH for this cd only
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ -n "$(git status --porcelain)" ]; then
	echo "Error: the working tree is not clean, commit or stash first" >&2
	exit 1
fi
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
	echo "Error: tag v$version exists already" >&2
	exit 1
fi

current=$(sed -n 's/^version = "\(.*\)"$/\1/p' src-tauri/Cargo.toml | head -n 1)
echo "$current -> $version"

# Cargo.toml: the first `version = ` line is the package's.
sed -i "0,/^version = \"$current\"$/s//version = \"$version\"/" src-tauri/Cargo.toml
# Cargo.lock: the version line right after our own package's name.
sed -i "/^name = \"bindsight\"$/{n;s/^version = \"$current\"$/version = \"$version\"/}" src-tauri/Cargo.lock
# package.json: the top-level version.
sed -i "0,/^  \"version\": \"$current\",$/s//  \"version\": \"$version\",/" package.json

for f in src-tauri/Cargo.toml src-tauri/Cargo.lock package.json; do
	if ! grep -q "\"$version\"" "$f"; then
		echo "Error: $f was not updated" >&2
		git checkout -- src-tauri/Cargo.toml src-tauri/Cargo.lock package.json
		exit 1
	fi
done

git add src-tauri/Cargo.toml src-tauri/Cargo.lock package.json
git commit -q -m "Bump version to $version"
git tag -a "v$version" -m "BindSight $version"
echo "Committed and tagged v$version."

# Watch the build run the v-tag triggers and, on success, print the draft
# release URL. Offered only after a push (nothing runs otherwise), like the
# push itself.
watch_build() {
	if ! command -v gh >/dev/null 2>&1; then
		echo "gh not found; watch manually: gh run watch && gh release view v$version --json url -q .url" >&2
		return
	fi
	echo "Waiting for the build run to appear..."
	run_id=""
	i=0
	while [ "$i" -lt 20 ]; do
		run_id=$(gh run list --workflow build.yml --event push \
			--json databaseId,headBranch \
			--jq "map(select(.headBranch == \"v$version\")) | .[0].databaseId // empty" 2>/dev/null || true)
		[ -n "$run_id" ] && break
		i=$((i + 1))
		sleep 3
	done
	if [ -z "$run_id" ]; then
		echo "Could not find the build run for v$version; check it with: gh run list" >&2
		return
	fi
	if gh run watch "$run_id" --exit-status; then
		url=$(gh release view "v$version" --json url --jq .url 2>/dev/null || true)
		if [ -n "$url" ]; then
			echo "Draft release: $url"
		else
			echo "Run succeeded, but the draft release is not visible yet: gh release view v$version --web" >&2
		fi
	else
		echo "The build run failed; see: gh run view $run_id --web" >&2
	fi
}

printf 'Push main and v%s to origin now? [y/N] ' "$version"
read -r answer
case $answer in
y | Y | yes | YES)
	git push origin HEAD
	git push origin "v$version"
	echo "Pushed. The release workflow builds a draft release; publish it on GitHub."
	printf 'Watch the build run and print the draft release URL when it succeeds? [y/N] '
	read -r watch
	case $watch in
	y | Y | yes | YES)
		watch_build
		;;
	*)
		echo "Not watching. Later: gh run watch && gh release view v$version --json url -q .url"
		;;
	esac
	;;
*)
	echo "Not pushed. Later: git push origin HEAD && git push origin v$version"
	;;
esac
