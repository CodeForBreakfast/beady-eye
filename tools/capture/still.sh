#!/usr/bin/env bash
# The README's still of `bdi web`, taken from the `bdi` in this tree over the
# ground record.sh records the animation in.
#
#   nix develop -c cargo build --release
#   nix shell nixpkgs#chromium -c tools/capture/still.sh
#
# Run it after a change to what the page draws, because nothing else in this
# repository checks the still against the binary.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/docs/images/bdi-web.png"
address=127.0.0.1:47380
page="/arkham/ark-1.1?at=arkham%3Aark-1%2Farkham%3Aark-1.1#shown"

ground=$(mktemp -d /tmp/bdi-ground.XXXX)
runtime=$(mktemp -d /tmp/bdi-rt.XXXX)
web=
trap '[ -n "$web" ] && kill "$web"; rm -rf "$ground" "$runtime"' EXIT

"$repo/tools/capture/ground.sh" "$ground"
# shellcheck source=/dev/null
. "$ground/isolate"
export XDG_RUNTIME_DIR="$runtime"
export PATH="$ground/bin:$repo/target/release:$PATH"

# --poll reads dunwich as well, which the ground's config leaves unpolled.
# HOME stays the caller's for Chromium, which hangs in headless mode on a
# profile it has to make new.
(cd "$ground" && HOME="$ground" exec bdi web --listen "$address" --poll) &
web=$!
for _ in $(seq 100); do
  curl -sf "http://$address/healthz" >/dev/null && break
  sleep 0.1
done

chromium --headless=new --disable-gpu --hide-scrollbars \
  --blink-settings=preferredColorScheme=0 --force-device-scale-factor=2 \
  --window-size=1440,640 --screenshot="$out" "http://$address$page" 2>/dev/null
echo "wrote docs/images/bdi-web.png, $(wc -c <"$out") bytes"
