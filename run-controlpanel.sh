#!/bin/sh
# Launch the chumby control panel under the forked Ruffle with fixtures.
#
# Usage:
#   ./run-controlpanel.sh              # normal run (device path, 2x window)
#   ./run-controlpanel.sh -Pbuiltin=1  # extra args are passed to ruffle,
#                                      # e.g. other FlashVars or --width
#
# Reads swf-assets/controlpanel.swf (copyrighted, not in the repo — see
# claude-docs/development.md §4); override with CHUMBY_SWF=<path>.
#
# Logs go to the terminal AND /tmp/chumby-run.log (chumby_host lines show
# every environment request; "MISSING" = fixture to add).
#
# Simulated bend sensor (squeeze button) — three equivalent ways:
#   type `bend` (or `tap`) + Enter in this terminal
#   echo bend > /tmp/chumby-ctl        (from any shell)
#   press Home with the ruffle window focused
# `bend down` / `bend up` give explicit press/hold/release.

DIR="$(cd "$(dirname "$0")" && pwd)"
RUFFLE="$DIR/target/debug/ruffle_desktop"
SWF="${CHUMBY_SWF:-$DIR/swf-assets/controlpanel.swf}"
CTL=/tmp/chumby-ctl

if [ ! -f "$SWF" ]; then
    echo "controlpanel.swf not found at $SWF"
    echo "Put it in swf-assets/ or set CHUMBY_SWF=<path>."
    exit 1
fi

# The panel fetches its widget channel from this profile. The generator
# (./chumby-widget-channel) produces it from fixtures/widgets/*.widget.xml;
# a committed copy ships in the tree, so debugging runs need not regenerate
# first. Just confirm the definition-list is present before launching.
PROFILE="$DIR/fixtures/http/xml.chumby.com/xml/profiles"
if [ ! -s "$PROFILE" ]; then
    echo "widget channel profile missing/empty at $PROFILE"
    echo "Generate it with: ./chumby-widget-channel"
    exit 1
fi

[ -p "$CTL" ] || mkfifo -m 600 "$CTL" || exit 1

if [ ! -x "$RUFFLE" ]; then
    echo "ruffle_desktop not built. Build it with:"
    echo "  cargo build -p ruffle_desktop"
    exit 1
fi

RUST_LOG="${RUST_LOG:-warn,chumby_host=info,avm_trace=info}" \
"$RUFFLE" \
    --load-behavior blocking \
    --filesystem-access-mode allow \
    --chumby-fixtures "$DIR/fixtures" \
    --chumby-control "$CTL" \
    --width 640 --height 480 \
    -PlocalCache=1 \
    "$@" \
    "$SWF" 2>&1 | tee /tmp/chumby-run.log
