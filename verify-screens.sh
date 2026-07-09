#!/bin/sh
# Automated screen verification for the chumby control panel.
#
# Navigates to Alarms, Music → My Streams, and Settings → Volume,
# taking a screenshot at each screen and saving to claude-docs/images/.
#
# Requires: xdotool, import (ImageMagick), mpv (for audio screens)
# The panel must NOT be running before you call this — the script starts it.
#
# Click method: `xdotool mousemove --window` (real pointer warp, window-
# relative) followed by a plain XTEST `click 1`. Two pitfalls make the
# obvious alternatives fail silently: winit ignores XSendEvent input, so
# `click --window` never reaches Ruffle; and getwindowgeometry can report
# frame-relative coordinates under a reparenting WM, so absolute-coordinate
# clicks can miss the window entirely.
# All coordinates below are window-relative (640x480 stage + ~24px Ruffle
# menu bar) and were verified against the chumby_pick=debug trace on
# 2026-07-02; the trap kills ruffle_desktop by name because killing the
# launcher shell leaves the ruffle|tee pipeline running.
#
# Usage:
#   ./verify-screens.sh              # navigate all three screens
#   ./verify-screens.sh alarms       # just the Alarms panel
#   ./verify-screens.sh streams      # just Music → My Streams
#   ./verify-screens.sh volume       # just Settings → Volume

set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
IMGDIR="$DIR/claude-docs/images"
CTL=/tmp/chumby-ctl
LOG=/tmp/chumby-verify.log

WANT="${1:-all}"

# ---- helpers ----------------------------------------------------------------

die() { echo "ERROR: $*" >&2; exit 1; }

wait_window() {
    echo "Waiting for Ruffle window..."
    WID=$(xdotool search --sync --class ruffle 2>/dev/null | head -1)
    [ -n "$WID" ] || die "window never appeared"
    echo "Window: $WID"
}

focus() {
    xdotool windowfocus --sync "$WID"
    sleep 0.15
}

click() {
    # click X Y in window coordinates: warp pointer, then XTEST click
    focus
    xdotool mousemove --window "$WID" "$1" "$2"
    sleep 0.1
    xdotool click 1
    sleep 0.15
}

shot() {
    # shot FILENAME_WITHOUT_EXTENSION
    focus
    sleep 0.5
    import -window "$WID" "$IMGDIR/$1.png"
    echo "  screenshot → $IMGDIR/$1.png"
}

bend() {
    # Bend presses are momentary and silently lost if sent before the SWF
    # restarts its bend polling after a panel closes — so confirm via the
    # avm_trace log and retry instead of trusting a fixed sleep.
    for _ in 1 2 3; do
        n=$(grep -c 'pressBendSensor' "$LOG" || true)
        echo bend > "$CTL"
        sleep 1.2   # give B2 time to animate in
        m=$(grep -c 'pressBendSensor' "$LOG" || true)
        [ "$m" -gt "$n" ] && return 0
        sleep 1.0
    done
    die "bend never registered (see $LOG)"
}

# B2 button positions (icon circle centers, verified via chumby_pick trace):
B2_MUSIC_X=570;    B2_MUSIC_Y=335
B2_SETTINGS_X=448; B2_SETTINGS_Y=458
B2_ALARMS_X=570;   B2_ALARMS_Y=458

# ---- launch panel -----------------------------------------------------------

[ -p "$CTL" ] || mkfifo -m 600 "$CTL"

echo "Starting control panel..."
RUST_LOG="${RUST_LOG:-warn,chumby_host=info,avm_trace=info,chumby_pick=debug}" \
    "$DIR/run-controlpanel.sh" >"$LOG" 2>&1 &
trap 'pkill -f ruffle_desktop 2>/dev/null || true' EXIT

wait_window
echo "Booting (~8 s)..."
sleep 8   # startup + Authorize + normal operation

mkdir -p "$IMGDIR"

# ---- Alarms panel -----------------------------------------------------------

do_alarms() {
    echo "--- Alarms panel ---"
    bend
    shot "m2-b2-for-alarms"
    click $B2_ALARMS_X $B2_ALARMS_Y
    sleep 1.2
    shot "m2-alarms-b5"
    click 557 463    # DONE → widget mode
}

# ---- Music → My Streams -----------------------------------------------------

do_streams() {
    echo "--- Music → My Streams ---"
    bend
    click $B2_MUSIC_X $B2_MUSIC_Y
    sleep 1.2
    shot "m2-music-c0"
    click 588 378    # scroll source list down (My Streams is entry 7)
    sleep 0.8
    click 150 368    # select "My Streams"
    sleep 0.8
    click 559 463    # GO TO
    sleep 1.2
    shot "m2-streams-c2"
    click 559 463    # DONE → C0
    sleep 1.0
    click 406 463    # DONE → widget mode
}

# ---- Settings → Volume ------------------------------------------------------

do_volume() {
    echo "--- Settings → Volume ---"
    bend
    click $B2_SETTINGS_X $B2_SETTINGS_Y
    sleep 1.2
    shot "m2-settings-e0"
    click 323 163    # VOLUME icon
    sleep 1.2
    shot "m2-volume-e1"
    click 557 463    # DONE → E0
    sleep 1.0
    click 557 463    # DONE → widget mode
}

# ---- run selected screens ---------------------------------------------------

case "$WANT" in
    alarms)  do_alarms ;;
    streams) do_streams ;;
    volume)  do_volume ;;
    all)
        do_streams
        do_volume
        do_alarms
        ;;
    *) die "unknown target: $WANT (use alarms|streams|volume|all)" ;;
esac

echo ""
echo "Screenshots saved to $IMGDIR"
echo "Log: $LOG"
