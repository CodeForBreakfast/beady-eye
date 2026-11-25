#!/usr/bin/env bash
# The README's still of `bdi web`, taken from the `bdi` in this tree over the
# ground record.sh records the animation in.
#
#   nix develop -c cargo build --release
#   nix develop -c tools/capture/still.sh
#
# Run it after a change to what the page draws, because nothing else in this
# repository checks the still against the binary.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo/docs/images/bdi-web.png"
page="/arkham/ark-1.1?at=arkham%3Aark-1%2Farkham%3Aark-1.1&open=b%3Adunwich%3Adun-1#shown"

ground=$(mktemp -d /tmp/bdi-ground.XXXX)
runtime=$(mktemp -d /tmp/bdi-rt.XXXX)
web=
trap '[ -n "$web" ] && kill "$web"; rm -rf "$ground" "$runtime"' EXIT

"$repo/tools/capture/ground.sh" "$ground"
# The ground's clock moves arkham's work on for the animation, and the still
# has nothing for it to move.
kill "$(cat "$ground/clock.pid")"
# shellcheck source=/dev/null
. "$ground/isolate"
export XDG_RUNTIME_DIR="$runtime"
export PATH="$ground/bin:$repo/target/release:$PATH"

# --poll reads dunwich as well, which the ground's config leaves unpolled.
# HOME stays the caller's for Chromium, which hangs in headless mode on a
# profile it has to make new. Port 0 and the address bdi web says it took
# keep the shot off any other server already listening here.
(cd "$ground" && HOME="$ground" exec bdi web --listen 127.0.0.1:0 --poll) \
  >"$ground/web.out" &
web=$!
url=
for _ in $(seq 300); do
  url=$(sed -n 's/^bdi web is answering on //p' "$ground/web.out")
  [ -n "$url" ] && curl -sf "$url/healthz" >/dev/null && break
  url=
  sleep 0.1
done
if [ -z "$url" ]; then
  echo "bdi web never answered, so the still is left as it was" >&2
  exit 1
fi

chromium --headless=new --disable-gpu --hide-scrollbars \
  --blink-settings=preferredColorScheme=0 --force-device-scale-factor=2 \
  --window-size=1440,640 --screenshot="$out" "$url$page" 2>/dev/null
echo "wrote docs/images/bdi-web.png, $(wc -c <"$out") bytes"
