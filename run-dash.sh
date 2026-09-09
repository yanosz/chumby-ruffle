#!/bin/sh
# Launch the Sony Dash control panel under the forked Ruffle with its own
# fixture tree (fixtures-dash/README.md). Extra args pass through to ruffle.
#
# Reads swf-assets/dash/controlpanel.swf and links
# swf-assets/dash/default_theme.swf in as the theme (both copyrighted, not
# in the repo — see claude/dash-panel-survey.md); override the panel with
# CHUMBY_SWF=<path>.

DIR="$(cd "$(dirname "$0")" && pwd)"
RUFFLE="$DIR/target/debug/ruffle_desktop"
SWF="${CHUMBY_SWF:-$DIR/swf-assets/dash/controlpanel.swf}"
THEME="$DIR/swf-assets/dash/default_theme.swf"
FIXTURES="$DIR/fixtures-dash"
CTL=/tmp/chumby-ctl

if [ ! -f "$SWF" ]; then
    echo "Dash controlpanel.swf not found at $SWF"
    echo "Put it in swf-assets/dash/ or set CHUMBY_SWF=<path>."
    exit 1
fi
if [ ! -f "$FIXTURES/rootfs/psp/theme.swf" ]; then
    if [ -f "$THEME" ]; then
        # A copy, not a link: installing a theme writes /psp/theme.swf, which
        # through a link would overwrite the asset it points at.
        cp "$THEME" "$FIXTURES/rootfs/psp/theme.swf"
    else
        echo "no $THEME — the panel will show no theme until one is installed" >&2
    fi
fi

# Widget SWFs (chumby's, gitignored) sit in swf-assets/dash/widgets/; the
# channel fixture names them as file:////usr/widgets/<name>.swf.
mkdir -p "$FIXTURES/rootfs/usr/widgets"
for w in "$DIR"/swf-assets/dash/widgets/*.swf; do
    [ -f "$w" ] && ln -sf "$w" "$FIXTURES/rootfs/usr/widgets/$(basename "$w")"
done

# Themes for the picker: swf-assets/dash/themes/*.swf, offered through the
# generated catalog (core/src/chumby/dash_theme.rs).
mkdir -p "$FIXTURES/rootfs/psp/themes"
for t in "$DIR"/swf-assets/dash/themes/*.swf; do
    [ -f "$t" ] && ln -sf "$t" "$FIXTURES/rootfs/psp/themes/$(basename "$t")"
done

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
    --chumby-fixtures "$FIXTURES" \
    --chumby-control "$CTL" \
    --width 860 --height 480 \
    "$@" \
    "$SWF" 2>&1 | tee /tmp/chumby-dash-run.log
