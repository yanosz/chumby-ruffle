# fixtures-dash — the Sony Dash panel's answer corpus

The Dash panel (`claude/dash-panel-survey.md`) gets its own tree because
the two panels write different things into the same `/psp` names —
the Dash rewrites `/psp/alarms` in its own schema at first start — and
because the classic tree's `/tmp/nightmode` puts the Dash straight into
night mode. Same layout and rules as `fixtures/` (see its README):
`rootfs/` is the virtual filesystem, `exec/manifest.txt` the command
answers, `http/` the chumby.com answers, keyed by the panel's own request
strings; a `MISSING` line in the log names the file to add.

Launcher: `./run-dash.sh` (repo root). It expects the panel at
`swf-assets/dash/controlpanel.swf` and links
`swf-assets/dash/default_theme.swf` to `rootfs/psp/theme.swf` (both
chumby's, both gitignored). `/psp/securityQuestion` and
`/psp/securityAnswer` exist only so the startup wizard's
`needStartNetwork()` is false; their content is never checked offline.

The channel comes through the Dash's XAPI: `http/xml.chumby.com/xapis/`
holds `auth/create`, `device/index/_`, `profile/show/1` and
`profile/list/_`. A file named `_` answers any last path segment — the
device GUID sits there and differs per box — while an exact file still
wins. The widget movie href is `file:////usr/widgets/builtinclock.swf`,
resolved through the virtual rootfs like the theme.

Answered in Rust rather than from `exec/`: `tzdump <zone>` (DST transitions
from the system tzdata, `core/src/chumby/tzdump.rs`), `list_mounts` (the
`rootfs/mnt/usb*` entries that resolve) and the slave-player memory poll
(empty: there is no slave). `chumbthumb` is a manifest stub answering its
failure status.

Widget SWFs live in `swf-assets/dash/widgets/` (gitignored); `run-dash.sh`
links them into `rootfs/usr/widgets/`, where the channel fixture's
`file:////usr/widgets/<name>.swf` hrefs resolve.

Tones: the Dash asks for `/usr/chumby/alarmtones/<name>.raw` and
`/usr/chumby/eventtones/<name>.raw`; the player serves each from the `.mp3`
of the same name (fork issue 28). Put MP3s there by hand for a desktop run
— `alarm1`-`alarm6`, `Birds`, `Wave` and `event1`-`event6`, `Birds`,
`Wave` — they are copyrighted and gitignored, like the classic's.

Themes offered by the in-panel picker come from `rootfs/psp/themes/`;
`run-dash.sh` links whatever is in `swf-assets/dash/themes/` there, and the
catalog the panel fetches is generated from that directory
(`core/src/chumby/dash_theme.rs`), so a theme dropped in is offered without
any server. The catalog names a theme after its file, underscores as spaces
(`Space_Theme.swf` → "Space Theme"). Installing one writes
`rootfs/psp/theme.swf`, which is why the launcher seeds that path with a
copy and never a link.
