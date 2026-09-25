#!/bin/sh
# herdr's build step: download the released bdi for this machine into bin/,
# checked against the sha256 published beside it. The release is the tag for
# the manifest's version, so a checkout always fetches its own release.
#
# herdr runs this from the plugin checkout without the runtime environment, so
# the root is found from where the script lives.
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$root/herdr-plugin.toml" | head -n 1)"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *)
    echo "beady-eye: no released bdi for $(uname -s) $(uname -m)." >&2
    echo "Install it with 'cargo install beady-eye' and copy it to $root/bin/bdi." >&2
    exit 1
    ;;
esac

asset="bdi-$target"
base="https://github.com/CodeForBreakfast/beady-eye/releases/download/v$version"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# GitHub's CDN can answer 404 for a few minutes after a release publishes, so a
# 404 is retried along with everything else.
fetch() { curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors "$1" -o "$2"; }

echo "beady-eye: fetching $asset from v$version"
fetch "$base/$asset" "$tmp/bdi"
fetch "$base/$asset.sha256" "$tmp/bdi.sha256"

expected="$(awk '{ print $1 }' "$tmp/bdi.sha256")"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/bdi" | awk '{ print $1 }')"
else
  actual="$(shasum -a 256 "$tmp/bdi" | awk '{ print $1 }')"
fi
if [ "$expected" != "$actual" ]; then
  echo "beady-eye: $asset does not match its published sha256." >&2
  echo "Expected $expected, got $actual." >&2
  exit 1
fi

mkdir -p "$root/bin"
install -m 0755 "$tmp/bdi" "$root/bin/bdi"
echo "beady-eye: installed $root/bin/bdi"
