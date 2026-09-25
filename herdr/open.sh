#!/bin/sh
# herdr's open action: bdi in a split beside the focused pane, started in that
# pane's directory so it reads the project the reader is looking at. A plugin
# pane otherwise starts in the plugin root, which no tracker covers.
set -eu

# herdr writes the context as one line of JSON. A directory whose name needs
# escaping in JSON matches neither key, and bdi then starts in the plugin root
# and says what it could not find.
context_field() {
  printf '%s' "$HERDR_PLUGIN_CONTEXT_JSON" | sed -n "s/.*\"$1\":\"\([^\"\\\\]*\)\".*/\1/p"
}

cwd="$(context_field focused_pane_cwd)"
[ -n "$cwd" ] || cwd="$(context_field workspace_cwd)"

set -- --plugin "$HERDR_PLUGIN_ID" --entrypoint eye --focus
[ -n "$cwd" ] && set -- "$@" --cwd "$cwd"
[ -n "${HERDR_PANE_ID:-}" ] && set -- "$@" --target-pane "$HERDR_PANE_ID"

exec "$HERDR_BIN_PATH" plugin pane open "$@"
