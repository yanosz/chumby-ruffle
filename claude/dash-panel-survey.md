# The Sony Dash panel: theme mechanism and feature diff

Step 2 of `chumby-pi/claude/sony-dash-panel-plan.md`, 2026-09-09. Fact base:
the ffdec export in
`chumby-pi-internal/docs/reference/appendix/controlpanel-dash-1.0.0/scripts/__Packages/`
(paths below are relative to it unless stated), the companion export
`…/companions-dash-1.0.0/default_theme/`, and the package scripts in
`chumby-pi-internal/resources/dash-chumby-hidc10-1.0.0/package/`. Classic
facts are cited from `chumby-pi-internal/docs/reference/03-environment-contract.md`
("03") and `05-screens.md`.

## 1. The theme mechanism

### 1.1 What a theme is

A theme is a SWF whose root becomes an instance of a `Theme` subclass. The
shipped `default_theme.swf` does this in its only frame script:

    com.blueocty.themes.Theme.main(com.example.MyTheme, this);

`Theme.main` (`default_theme` `com/blueocty/themes/Theme.as:8-12`) sets
`root.__proto__ = classRef.prototype` and applies the constructor. `Theme`
extends `MovieClip` (line 1) and declares 35 empty handlers (lines 13-127):
`onHello`, `onGoodbye`, `onCallbacks` (stores `this.callbacks`), `onHome`,
`isHome`, `isReallyHome`, `isFullscreenWidget`, `onPopupBar`, `onNavigation`,
`onControls`, `onClocks`, `onClockFormat`, `onAlarms`, `onEvents`,
`onWeather`, `onWeatherFormat`, `onChannel`, `onWidget`,
`onFullScreenWidgets`, `onFullScreenWeather`, `onFullscreenClock`,
`onDestroyTheme`, `onDisconnectEvents`, `onPhotos`, `onPhotoResized`,
`onFullScreenPhotos`, `onBIVLServices`, `onBIVLStatus`, `onMusicServices`,
`onMusicStatus`, `onMusicProgress`, `onVolume`, `onFlip`, `onSpeech`,
`onKeyboard`, `onUSBMount`, `onUSBUnmount`. `BroadcastingTheme.as` re-emits
each as a `broadcastMessage` to the theme's own modules; `MyTheme.as` is the
Space Theme (`com/example/*`: time, weather, photos, music, videos, widgets
modules, 3 108 lines in all).

The panel-side interfaces `com/blueocty/themes/ITheme.as` and
`IThemeCallbacks.as` decompile as empty 3-line interfaces (AS2 interface
methods leave no bytecode), so the contract is defined by the calls, not by
those files.

### 1.2 How the panel loads it

`com/chumby/controlpanel/home/HomeScreenProxy.as:7-12` — the home screen is a
proxy whose only job is `ThemeLoader.instance.loadTheme(this, 5)`.

`com/chumby/controlpanel/dash/themes/ThemeLoader.as`:

- `:12` `DEFAULT_PATHS = ["/tmp/theme.swf", "/mnt/usb/theme.swf",
  "/tmp/alttheme.swf", "/psp/theme.swf", "/usr/widgets/theme.swf"]`; `:13`
  `DEFAULT_PATH = "theme.swf"` (relative, last resort).
- `:61-73` the first path for which `ChumbyNative._fileExists` is true wins.
  **No signature, md5 or catalog check happens here.**
- `:74-75` the theme is loaded into a fresh `_panel` child clip with
  `_lockroot = true`; `:80-83` on a device (`Chumby.isChumby`, i.e.
  `$version` not MAC/WIN, `Chumby.as:140-146`) the path is prefixed
  `file:///`; `:89` `MovieClipLoader.loadClip`.
- `:86-87` `_enableSlaveUpdates(false)` and `_fillFrameBufferBytes(0,0)`
  before the load (unless the browser is active); `:107-115` slave updates
  re-enabled 1 s after `onLoadInit`.
- `:91-106` `onLoadInit`: `putFile("/tmp/cpready","1")`, then the handshake
  in this order: `onHello("Hello from Control Panel")`, `onCallbacks(ThemeCallbacks.instance)`,
  `onClocks(InternationalLocations.instance)`, `onAlarms({alarms})`,
  `onWeather(WeatherLocations.instance)`, `onWeatherFormat("F"|"C")` (from
  `/psp/weatherType`, `:172-176`), `onMusicServices({musicServices})`,
  `onVolume(n)`. Later, on events: `onFlip`, `onVolume`, `onUSBMount/Unmount`
  (`:226-258`), `onMusicStatus` (`:259-266`), `onChannel`, `onWidget`,
  `onEvents`, `onBIVLServices`, `onFullScreenPhotos` (`:180-221`),
  `onPhotos` (`dash/ThemePhotos.as:64,69`), `onPhotoResized` and
  `onKeyboard` (`ThemeCallbacks.as:192,357`), `onFullScreenWeather` /
  `onFullScreenPhotos` from scheduler events (`com/chumby/event/Event.as:1311,1354`).
- `:14,23-30` the display name comes from `/psp/theme_name.txt`, default
  `"Dashboard"`.
- `:31-35,54` a `ClassWatcher` captures `_global` class namespaces before
  each load; its `diff()` body is empty (`com/chumby/util/ClassWatcher.as:45-68`),
  so the panel does **not** unload a previous theme's classes. The theme does
  it itself: `MyTheme.onGoodbye` deletes `_global.com.example`,
  `com.chumby.calendar`, `Theme`, `BroadcastingTheme` (`MyTheme.as:22-29`).
  `ThemeLoader.as:48` resets `_global.com.blueocty.themes.Theme._init` first.

### 1.3 What a theme can ask of the panel

`com/chumby/controlpanel/dash/themes/ThemeCallbacks.as` (379 lines) is the
object handed over in `onCallbacks`. Groups, with the line of the first
method: popup bar and navigation (`:25-36`, `navigateTo(key,param)` drives
`PopupMenuMain`), clocks and date (`:37-50`), alarms and snooze (`:51-62`),
weather (`:63-70`), channel (`:71-74`), widgets (`:75-179`: pause/unpause,
next/previous, `setWidgetPosition(x,y,w,h)`, pin, rate, customize, delete,
add, select), photos (`:180-201`, `resizePhoto` runs the `chumbthumb`
exec via `ImageResizer2`, `deleteTemporaryFile` strips a `sys:/` prefix),
BIVL and music (`:202-278`; `musicPlayUrl(url, useHW)` selects the
`themehwurl` / `themeswurl` player sources), volume (`:279-294`), flip
(`:295-298`), cookies (`:302-346`: `/psp/cookies/cookie_<md5(selector+"chumby"+key)>`,
value blowfish-encrypted with the GUID and base64-encoded via natives
5,160-164), keyboard (`:347-358`), `alertOK`, `inDialog`, interval pool
(`:359-374`). The Space Theme uses 25 of these (grep `callbacks\.` in its
export): `setWidgetPosition`, `isWidgetPinned`, `clockFormat`,
`showHidePopupBar`, `setToWidget`, `sendWidget`, `sendWeather(Format)`,
`sendPhotos`, `sendMusicStatus`, `sendMusicServices`, `sendClocks`,
`sendChannel`, `sendBIVLServices`, `sendAlarms`, `selectWidget`,
`resizePhoto`, `previousWidget`, `pinWidget`, `nextWidget`, `navigateTo`,
`launchMusicService`, `launchBIVLService`, `dateForLocation`.

**The widget rectangle is the theme's decision.** `setWidgetPosition`
(`ThemeCallbacks.as:109-113`) forwards to `WidgetMask` and
`WidgetSequencer.setPositionAndSize`, which on a device calls
`_setDisplayRect(MAP_SLAVE, …)` and `_setDisplayRectEventTranslate`
(`widgetbrowser/WidgetSequencer.as:606-607`) and sets the slave vars
`_chumby_widget_stage_width/height` (`:609-610`). Widgets stay 320x240
content (`:364` fills a 320x240 rect) composed inside the theme.

### 1.4 Choosing a theme: the settings UI

Entry: popup menu "Content" › "Theme Selection" (`dash/popup/PopupMenuMain.as:24`,
key `themes`) → `PopupBar.hitThemesButton` (`dash/popup/PopupBar.as:168-179`,
gated by `/psp/parentalControlLevel` and
`/psp/parentalControlsDisable_Change theme`) → `ThemeSelectorDialog`
(`settings/themes/ThemeSelectorDialog.as:15-23`) → `ThemesPanel` states
`loadingThemes → chooseTheme → loadingTheme` (`ThemesPanel.as:33-58`).

- `ThemesPanelLoadingThemes.as:10-11` loads the catalog (§1.5).
- `ThemesPanelChooseTheme` shows it as a 3-per-row grid
  (`ThemesPanelChooseThemeGridItems.as:12-23`); each cell loads
  `thumbnailURL` with a `MovieClipLoader` (`…GridItemIcon.as:21-25`).
- `ThemesPanelLoadingTheme.as:13-32` fetches the pick: with
  `Chumby.localThemes` it runs `cp <url.slice(8)> /psp/theme.swf; sync; echo $?`,
  otherwise `/psp/download_theme <url> <md5>`; on
  `<download_theme error="success"/>` it runs
  `cp /tmp/theme.swf /psp/theme.swf; rm /tmp/theme.swf /tmp/theme.swf.sig; sync; echo $?`
  (`:69`), writes `/psp/theme_name.txt` (`:76`), and `ThemesPanel.next`
  reloads the home screen (`ThemesPanel.as:55-56`).
- All of that goes through `com/chumby/util/AsynchronousCommand.as:40-72`:
  `XML.load("exec://nice -n 10 " + escape(cmd))`.

The "New" button opens `ThemeWizard` (`chooseLayout → chooseBackground →
loadingChannels → chooseChannel → enterName`, `ThemeWizard.as:27-50`).
Every panel of it only calls `next()` (`ThemeWizardChooseLayout.as` 16
lines, `…ChooseBackground.as` 12, `…ChooseChannel.as` 16,
`…EnterName.as:26-30`); `done()` dismisses the dialog
(`ThemeWizardDialog.as:23-29`). **Nothing is written anywhere: the wizard is
a stub.** Step 1's note that the panel can "build" a theme was wrong.

A theme can also be switched by a scheduler event: `Event.as:1333-1346`
(`ACTION_THEME`) constructs `ThemesPanelUpdateTheme` and calls
`changeTheme(name)`.

### 1.5 The catalog, and the USB variant

`dash/themes/themenetwork/ThemeCatalogService.as:3`
`baseURL = "http://files.chumby.com/dash/$RELEASE$themes/"`, `$RELEASE$`
replaced by `Chumby.releaseMode` (`Chumby.as:34,52-59`: default
`"production/"`, overridden by `/psp/release_mode`, then
`/mnt/usb/release_mode`; the package ships `release_mode` = `production/`).

`ThemeCatalog.load` (`ThemeCatalog.as:24-56`): URL `…/themes.xml`; off a
device the same host; **if `Chumby.localThemes`, `file:////mnt/usb/externalthemes.xml`**
(`:51-54`). `localThemes` is true iff `/mnt/usb/externalthemes.xml` exists
when `Chumby` is constructed (`Chumby.as:60-64`), i.e. the stick must be in
at panel start.

Schema, from `ThemeCatalogItem.fromXML` (`:19-41`) and
`ThemeCatalogItemLayout.fromXML` (`:12-17`):

    <themes>
      <theme id="1" version="1">
        <name/> <description/> <author/> <thumbnailURL/> <url/> <md5/>
        <layouts> <layout id="1"> <name/> <thumbnailURL/> </layout> … </layouts>
      </theme>
    </themes>

In the USB variant `url` must be `file:///<absolute path>`: `url.slice(8)`
strips exactly eight characters (`ThemeCatalogItem.as:55`,
`ThemesPanelLoadingTheme.as:21`) and the remainder is the source of a `cp`.
`md5` is unused there. `thumbnailURL` is loaded as-is by `MovieClipLoader`.
`findByName` / `findByID("1")` (`ThemeCatalog.as:57-86`) are the lookups the
updaters use; id `1` is the fallback.

Whether the live catalog still answers is **not established**: the Wayback
CDX index for `files.chumby.com/dash/*` holds only the package zip, the
unbrick kit and `dashpix/`; one probe of
`files.chumby.com/dash/production/themes/themes.xml` from this host on
2026-09-09 failed without an HTTP status (the host has blocked this IP
before — plan, operational note). `download_theme` itself
(`package/download_theme:33-51`) is `wget -T 240 --writefpevent` plus an md5
compare, nothing else, despite its header comment.

### 1.6 What checks the signature — and when

Two sites, identical: `startup/StartupPanelUpdateTheme.as:119-146` and
`settings/themes/ThemesPanelUpdateTheme.as:102-125`, both running

    /usr/bin/verify /psp/theme.swf /psp/theme.swf.sig /etc/sony.pub > /dev/null; echo $?

and deleting theme and `.sig` on a non-zero exit. The gate in front of it
(`:86-99` / `:69-83`): `/psp/theme.swf` exists **and** its `md5sum` equals
the catalog item's `md5` **and** `/psp/theme.swf.sig` exists; any other
combination goes to `loadTheme()`, which re-downloads from the catalog.
The `deleteInvalidTheme` branch inside `validateTheme` (`:127`) is
unreachable, guarded by the same `fileExists`.

Reachability:

- `StartupPanel.CHECK_THEME_STATE` is declared (`startup/StartupPanel.as:28`)
  and has a `case` (`:236-238`) that only forwards to `NORMAL_MODE_STATE`,
  but **no `gotoState(CHECK_THEME_STATE)` exists anywhere** (grep over the
  export). The startup validation never runs.
- `ThemesPanelUpdateTheme` is constructed only by `Event.as:1342-1344`. So
  the only signature check in 1.0.0 fires when a scheduled event switches
  themes, and only for a theme that already matches a catalog entry by md5.
- `ThemeLoader.loadTheme` and the selector's copy path (§1.4) verify
  nothing.
- `/usr/bin/verify` and `/etc/sony.pub` are Sony firmware components; the
  package does not carry them. The two shipped `.sig` files
  (`movie.swf.sig`, `factorytest.swf.sig`, PEM public-key blocks) are
  referenced by no panel code (grep `.sig`); `movie.swf` is what Sony's
  launcher runs before `more_startup.sh` kills it (`package/more_startup.sh:9-10`),
  so whatever reads them is on Sony's side.

Also in the package: `install_chumby.sh:7-8` seeds `/psp/theme.swf` from
`default_theme.swf` and `/psp/theme_name.txt` ("Space Theme");
`uninstall_chumby.sh:12-13` removes `/psp/themes.xml` and `/psp/themes/`,
paths no 1.0.0 code writes.

### 1.7 Answer: what it takes to show a theme of our own

Nothing needs signing and no server is involved. A theme is an 860x480 AVM1
SWF whose frame 1 calls `Theme.main(<class>, this)` with a subclass of
`com.blueocty.themes.Theme` (a copy of the two base classes from the Space
Theme export gives the full handler set), placed at one of the five paths in
`ThemeLoader.DEFAULT_PATHS`. `/mnt/usb/theme.swf` beats `/psp/theme.swf`,
`/tmp/theme.swf` beats both, so a USB stick with a bare `theme.swf` already
overrides the installed theme without any catalog. The catalog and
`externalthemes.xml` are only needed for *choosing among several* from the
panel UI, which then copies the pick to `/psp/theme.swf`.

What the player must provide for that path (against the fork today):

| need | Dash site | fork today |
|---|---|---|
| `_fileExists` on the five paths | `ThemeLoader.as:63` | dispatched (5,53), rootfs-backed |
| `MovieClipLoader.loadClip("file:///psp/theme.swf")` into a `_lockroot` child | `ThemeLoader.as:74-89` | `core/src/chumby/navigator.rs:47-67` resolves `file://` and scheme-less absolute URLs against the virtual rootfs, falling through to disk on a miss — reads as covered, **unverified** until the panel reaches the home screen |
| `_enableSlaveUpdates`, `_fillFrameBufferBytes`, `putFile /tmp/cpready` | `:86-93` | dispatched (5,119 / 5,385 / 5,51) |
| widget sub-rectangle via `_setDisplayRect` / `_setDisplayRectEventTranslate` and slave vars | `WidgetSequencer.as:606-610` | ids dispatched; **semantics** (widget composed inside a rectangle, not full screen) are new to the fork's in-process widget loader |
| `sys://` local-file scheme for photos | theme `PhotoHolder.as:25`, panel `CacheManager.as:132`, `USBPhotoPanelItemInfo.as:22` | not handled (grep `sys:` in `core/src/chumby/`: none) |
| `chumbthumb` exec (photo resize) | `image/ImageResizer2.as:8` | no fixture |
| cookies: natives 5,160-164 | `ThemeCallbacks.as:309-346` | dispatched |

## 2. Feature diff against the classic 2.8.87b3

### 2.1 Shape

| | classic 2.8.87b3 | Dash 1.0.0 |
|---|---|---|
| code | frame scripts (`frame_2/DoAction.as`) | 777 AS2 classes under `__Packages/`, 1 frame |
| stage / SWF | 320x240, v6, 10 frames | 860x480, v8, 1 frame; `ScreenDimensions.as:3-4` hardcodes 860x480 |
| scaling | — | `Stage.scaleMode` left at the default except safe mode and the browser (`ControlPanel.as:93`, `browser/BrowserPanel.as:63` set `noScale`) — so the panel scales to the window; on 1024x600 that is 1024x571 letterboxed |
| home screen | the panel's own | a loaded theme SWF (§1) |
| widgets | full screen, `_startSlave` | `_startSlave` on device (`WidgetSequencer.as:357`), `loadClip` into `__widgetProxy` with `_lockroot` off device (`:365-370`); placed in a theme-chosen rectangle (§1.3) |
| device natives | — | `com/blueocty/DashNative.as`: flip state (5,390/391), logo LED (5,392/393), `_fadeBacklight` (5,394); `_getWidgetNumber` (5,445) |
| vendor extras | — | Sony BIVL video stack (`com/blueocty/bivl/*`), `useBIVL = false` and `killall bivlcored` at start (`Chumby.as:51,66,175`); Sony browser at `/mnt/storage/chumbrowser/` (`browser/BrowserManager.as:136-151`); AdelaVoice speech pipes (`/tmp/adelaVoice*`); AccuWeather and TWC services |

### 2.2 Screens

Dash top level is `PopupMenuMain.MAIN_ITEMS` (`dash/popup/PopupMenuMain.as:21-31`):
Clock (Quick Alarm, Screen Scheduler, Time/Date, Weather), Content (Browse
Internet, **Theme Selection**, Browse Widgets, Change Channels, Edit
channels), Video (USB, PC Videos), Music (SHOUTcast, blue octy radio, Sleep
Sounds, My Streams, My Music Files, PC Music), Photos (Photobucket, USB, PC
Photos), System (Screen Controls, Network Configuration). Startup wizard
states (`startup/StartupPanel.as:4-32`): blank fb0/fb1, touchscreen
calibration, timezone, check/configure network, disconnected,
check-authorize, not-authorized, activate, check-date, set-time,
(check-theme, dead), normal, force-update. Settings packages present:
`apps`, `channels`, `deviceinfo`, `geek`, `network` (incl. delink/WPS),
`safemode`, `themes`, `touchscreen`, `usbphotos`, `nightmode`.

Classic (05 §A-E): boot/provisioning, main panel DS1766, music panel
DS1524, channel/widget DS1627, settings DS1748. Gone on the Dash: intercom,
FM radio, iPod as a music *panel* (an `ipod/IPodService` client to
`127.0.0.1:8080` remains), MP3tunes, NPR/Internode/CBS sources; new: themes,
scheduler events (`com/chumby/event/*`, `/psp/events`), parental controls
(`/psp/parentalControl*`), Photobucket / PC (SMB, `mount … -o username=`)
photos and music, USB video, browser, night mode.

### 2.3 `ASnative(5,N)`

Counted over the export (`grep -o 'ASnative(5,[0-9]+)'`, unique ids):

| | ids |
|---|---|
| Dash bound | 147 (146 names; 5,93 is bound twice as `_keyboardGetScanCode` and `_keyboardGetString`, `ChumbyNative.as:34,42`) |
| classic bound | 155 |
| common | 141 |
| Dash only | 5,390 `_getFlipState`, 5,391 `_setFlipState`, 5,392 `_getLogoLEDState`, 5,393 `_setLogoLEDState`, 5,394 `_fadeBacklight` (`DashNative.as:3-11`); 5,445 `_getWidgetNumber` (`ChumbyNative.as:287`) |
| classic only | 5,14 5,16 5,23 5,24 5,26 5,27 5,28 (unnamed in F2), 5,39 `_batteryVolts`, 5,41 `_powerSource`, 5,207 `_getpid`, 5,210, 5,211 `_setURLEncodedVars`, 5,220 `_enableMasterUpdatesPriv`, 5,333 `_getLastGesture` |
| fork dispatches (`core/src/chumby/avm.rs`, `N =>` arms) | 157 — every common id; none of the six Dash-only ids |

Of the 146 Dash names, 76 have a static call site outside their declaration.
Called Dash-only ids: 5,390 (1 site), 5,391 (1), 5,393 (2), 5,445 (3);
5,392 and 5,394 are bound only. Heavy hitters: `_fileExists` 123 sites,
`_unlink` 29, `_backtick` 19, `_enableSlaveUpdates` 11, `_routeUIEvents` 9,
`_getAudioPlayerState` 7. Inline `_global.ASnative(5,74)(1)` and `(5,72)(0)`
at `WidgetSequencer.as:377-378` (both in the common set). Volume, balance,
mute, time zone and system time go through natives 5,176-185 here, not
through `chumby_set_*` scripts as on the classic (03 §2b).

### 2.4 Shell commands

Mechanisms: `AsynchronousCommand` (`exec://nice -n 10 …`, `util/AsynchronousCommand.as:44`),
raw `XML.load("exec://…")`, and `_backtick` (5,52). Every distinct command
in the export, by site:

| command | site |
|---|---|
| `guidgen.sh`, `macgen.sh`, `chumby_version -h/-s/-f` (backtick), `chumby_version -m` (exec) | `Chumby.as:40-45,103` |
| `network_status.sh`, `start_network` | `network/NetworkStatus.as:11-12,63,100` |
| `signal_strength` | `network/WifiStatus.as:8,21` |
| `ap_scan` | `network/AccessPoints.as:10,46` |
| `network_adapter_list.sh` | `network/NetworkAdapters.as:34` |
| `delink_request`, `delink_reboot.sh`, `delink_refactory.sh` | `settings/network/NetworkPanelDelinkDoDelink.as:7,29`, `…DelinkResult.as:24`, `settings/deviceinfo/DeviceInfoPanel.as:28` |
| `list_mounts` | `usb/USBMediaEvents.as:69`, `controlpanel/music/usb/USBMusicPanel.as:122`, `…usbphotos/USBPhotosPanel.as:104`, `…video/usb/USBVideoPanel.as:148` |
| `metadb --prune <mount>` | `usb/USBVolume.as:130` (MetaDB service also on `127.0.0.1:8085`, `util/MetaDBService.as`) |
| `imgtool --fb=<n> --fill=0,0,0` | `display/ScreenManager.as:311` |
| `chumbthumb` | `image/ImageResizer2.as:8` |
| `du -s`, `ls -1`, `ls -1rut`, `mkdir`, `cd` (cache) | `util/CacheManager.as:30-33,68,93,108` |
| `dcid -o` | `DaughterCardID.as:6` |
| `tzdump <zone>` | `time/TimeZoneTransitions.as:30` |
| `/bin/chumby_haptic 0 50` | `gui/Haptic.as:18` |
| `bash /usr/bin/hide_gfx_layer0`, `/sbin/reboot`, `/usr/chumby/scripts/update_now.sh`, `killall bivlcored`, `start_sshd.sh`, `/usr/chumby/scripts/fb_cgi.sh`, `update.sh update1 REPAIR` | `Chumby.as:27-28,150-190` |
| `reboot`, `halt`, `start_sshd.sh` | `settings/geek/GeekPanel.as:41-50` |
| `reboot_normal.sh`, `restore_factory_defaults.sh`, `update_network.sh`, `update_usb.sh` | `safemode/SafeModePanel*.as` |
| `/mnt/storage/chumbrowser/{start,stop}_chumbrowser`, `browserpid=\`ps\|grep …\`` | `browser/BrowserManager.as:136-178` |
| `curl -L <url>`, `mv`, `mount \| grep` | `photos/photobucket/PBAlbumRSS.as:21`, `PBAlbumManager.as:199`, `photos/mypc/PBAccountManager.as:219` |
| `mount … /mnt/ipod -o username=`, `umount /mnt/ipod` | `ipod/*` |
| `rm <file>` | `alarm/AlarmSet.as:201`, `event/EventSet.as:200` |
| `sync` | `ChumbyNative.as:324`, `DeviceInfoPanel.as:138` |
| theme path: `md5sum`, `cp`, `rm`, `/psp/download_theme`, `/usr/bin/verify` | §1 |

Against the classic (03 §2): new here are `imgtool`, `metadb`, `chumbthumb`,
`tzdump`, `chumby_haptic`, `hide_gfx_layer0`, the chumbrowser pair,
`delink_*`, `list_mounts`, `download_theme`, `verify`; gone are
`chumby_set_volume/pan/mute`, `service_control` (inetradio, chumbipodd,
chumbradiod), the intercom cluster, `sync_time_state.sh`,
`reload_backup_alarm`, the `wget … crossdomain.xml` probe, `headphone_mgr`,
`md5sum /tmp/.guidhash`. The fork's fixture manifest answers `guidgen.sh`,
`macgen.sh`, `chumby_version -*`, `network_status.sh`, `signal_strength`,
`dcid -o` and friends today (`fixtures/exec/manifest.txt`); the desktop run
below reported `ap_scan`, `network_adapter_list.sh` and
`killall bivlcored; echo $?` as MISSING.

### 2.5 Files

Distinct paths named by the export (`grep` for quoted `/psp/`, `/tmp/`,
`/mnt/`, `/usr/`, `/etc/`, `/var/` strings):

- **/psp**, shared with the classic (03 §3): `firsttime`, `clock_format`,
  `dimlevel`, `alarms`, `alarm_volume`, `ifalarm`, `music_timer_duration`,
  `shoutcast_search`, `timezone_city`, `use_ntp`, `url_streams`,
  `network_config(s)`, `demo_mode`, `translation.xml`.
- **/psp**, Dash only: `theme.swf`, `theme.swf.sig`, `theme_name.txt`,
  `download_theme`, `release_mode`, `cookies/cookie_*`, `parentalControlLevel`,
  `parentalControlsPassword`, `parentalControlsDisable_<Action>` (Change
  theme, Change channels, Browse Internet, Add apps, Delete apps, Customize
  apps, Add mature apps, Add channels, Delete channels), `securityQuestion`,
  `securityAnswer`, `no_cq`, `running_delink`, `weatherType`,
  `weather_locations`, `clock_locations`, `brightness_high`, `flip_time`,
  `haptic`, `scheduler`, `events`, `ifevent`, `mypc`, `mypc_{server,domain,
  username,password,folder}`, `photobucket`, `browserHomepage`,
  `currentBrowserVersion.txt`, `sslIgnoreInitialWarning`, `skip_network`,
  `show_cursor`, `disable_intro`, `disable_updates`, `disable_eventing`,
  `disable_force_update`, `enable_proxy`, `update_baseurl`,
  `memory_whitelist`, `network_wps`, `network_generic_ascii`, `start_sshd`,
  `sample_photos/sample_content_*`. Plus what the desktop run wrote at
  first start: `weatherType`, `clock_locations`, `brightness_high`, `alarms`.
- **/tmp**: `cpready`, `theme.swf`, `alttheme.swf`,
  `themeDoesNotSupportWidgetBrowser`, `movieheartbeat`, `nightmode`,
  `musicsource`, `lcd_off`, `hidden_ssid`, `flashplayer.event`,
  `browser.xml`, `send_browser_event`, `recv_browser_event`,
  `challengeCodeForBrowserRegistration`, `mp_playback.info`,
  `usbmediaevents_searchresults`, `tmpusbthmb`, `photo_list_*`,
  `dl_photo.jpg`, `photo.jpg` (theme), `bivlcore*`, `bivlapps.{in,out}`,
  `adelaVoice{Commands,Responses}`.
- **/mnt**: `usb`, `usb2`…`usb4`, `ipod`, `storage/chumbrowser/*`,
  `usb/theme.swf`, `usb/externalthemes.xml`, `usb/release_mode`,
  `usb/mypc`, `usb/update1`, `usb/start_sshd`; scripts add
  `usb/controlpanel.swf`, `usb/opening.swf`, `usb/debugchumby`,
  `usb/start_httpd`.
- **elsewhere**: `/var/run/btplay.pid`, `/var/run/btplay.properties`,
  `/var/run/chumbyflashplayer.pid`, `/usr/widgets/theme.swf`,
  `/usr/chumby/alarmtones/`, `/usr/chumby/eventtones/`,
  `/usr/chumby/scripts/*`, `/usr/bin/verify`, `/usr/bin/hide_gfx_layer0`,
  `/etc/sony.pub`, `/psp/cacert.pem` (launcher `-E`).

### 2.6 Endpoints

`Chumby.DEFAULT_BASE_URL` and `DEFAULT_WIDGETS_URL` are both
`http://xml.chumby.com` (`Chumby.as:30-31`; the classic's widgets host is
`widgets.chumby.com`, 03 §5). Off a device both become `www.chumby.com`
(`:70-71`).

| endpoint | site |
|---|---|
| `{base}/xml/authorize?id=…`, `{base}/xml/registerchumby?id=…` | startup / activation |
| `{base}/xml/widgetinstances/<id>` | `Chumby.as:136` |
| `{base}/xapis/auth/create`, `{base}/xapis/…` (OAuth-style, `oauth_signature_method = MD5-HEX`) | `chumbynetwork/XAPI.as:164,350` |
| `files.chumby.com/dash/$RELEASE$controlpanel/controlpanel.xml` — **panel self-update**, 30 min after start then daily, unless `_root.alternate == 1` | `structure/UpdateMonitorCP.as:5-27` |
| `files.chumby.com/dash/$RELEASE$externalmusic/sources.xml` | `music/ExternalMusicSources.as` |
| `files.chumby.com/dash/$RELEASE$themes/themes.xml` | §1.5 |
| `files.chumby.com/dash/icons/*.png` (music source icons) | `music/MusicPlayer.as` |
| `files.chumby.com/yume/photoimages/photos.xml` | `photos/simple/SimplePhotos.as` |
| `content.chumby.com/shoutcast/{list,search/…,show?id=}`, `content.chumby.com/podcast/ny_times/list`, `content.chumby.com/` (NOAA) | `music/shoutcast/ShoutcastPlayer.as`, `music/nyt/NYTPodcastsPlayer.as`, `gov/noaa/NOAAWeatherService.as` |
| `music.chumby.com` (chumbcast, sleepcast) | `music/chumbcast/*`, `music/sleepcast/*` |
| `chumby.weather.com`, `images.weather.com` | `com/weather/TWC*.as` |
| `sonyyume.accu-weather.com/widget/sonyyume/` | `com/accuweather/AccuWeatherService.as` |
| `feed757.photobucket.com/…/dashphotobucket/feed.rss` (sample photos) | `photos/sample/SamplePhotos.as` |
| `127.0.0.1:8080` (iPod daemon), `127.0.0.1:8085` (MetaDB) | `ipod/IPodService.as`, `util/MetaDBService.as` |
| browser bookmarks: `sony.com/mydash`, `sony.com/dashsupport`, `facebook.com/sonydash`, `sonystyle.com`, google, yahoo, gmail, `{www,wiki,forum}.chumby.com` | `browser/BrowserLinksSet.as`, `…ChooseLinksListItems.as` |
| `192.81.128.180 ssm.internet.sony.tv` | `package/default_hosts` |

NFR6 consequence: the self-updater would replace the panel from
`files.chumby.com` if that host ever answered; it must be intercepted like
the classic's `update.chumby.com`.

### 2.7 FlashVars, launch, fscommand

`_root.*` read by the export (count of sites): `useParentalControls` 28,
`clockFormat` 25 (state, not a var), `demo_mode` 20, `useScheduler` 12,
`enable_events` 8, `fast` 7, `force_update` 6, `useMyPc` 5, `haptic` 5,
`skip_network` 2, `simplebend` 2, `firstTime`/`firsttime` 2+2,
`builtin`/`builtIn` 2+2, `alternate` 2, `eventReloadInterval`,
`alarmReloadInterval`, `defaultWidgetTime`, `defaultExtendTime` 2 each,
`safeMode`/`safemode`, `enable_updates`, `enable_bivl`, `cp_updater`,
`useParentalControlsOnBivl`, `test`, `baseURL`, `widgetsURL` 1 each.

The launcher (`package/start_control_panel:31,70-185`) runs
`chumbyflashplayer.x -E /psp/cacert.pem -i <cp> -3 --max32kblocksmaster=1024
--max32kblocksslave=512 --maxlocalmemblocksmaster=128 -x $DFB_X_RES -y $DFB_Y_RES`
plus `-dfirstTime=1` (first boot), `-dbuiltin=1 -dforce_update=1` (built-in
panel), `-dalternate=1` (`/mnt/usb/controlpanel.swf`), `-Q`. On the Dash
package `more_startup.sh:42` always points `/tmp/cp_path` at
`/psp/controlpanel.swf`, so `DOWNLOADED=1` and no `download_cp` runs.

`fscommand("quit")` at `startup/StartupPanel.as:262`,
`settings/network/NetworkPanel.as:430,440`,
`structure/ControlPanelUpdater.as:101`, `WidgetSequencer.as:471,493`;
`_fscommand2("GetTotalPlayerMemory"/"GetFreePlayerMemory")` from the
heartbeat (`structure/Heartbeat.as:13-14`) every 15 s, alongside
`putFile("/tmp/movieheartbeat","1")`.

## 3. First contact under the fork (desktop, 2026-09-09)

`target/release/ruffle_desktop` (built 2026-07-26 from an older dev),
classic fixtures, `--load-behavior blocking --filesystem-access-mode allow`,
no FlashVars, 45 s timeout:

- exit 124, no `panicked`, ~40 host calls. The panel ran `Chumby()`
  (backticks answered from fixtures, `_fileExists` on 25 `/psp` and
  `/mnt/usb` paths, `killall bivlcored` MISSING), wrote the four first-start
  files (§2.5), went `BLANK_FB0 → BLANK_FB1 → CHECK_NETWORK`
  (`network_status.sh` answered), then `CONFIGURE_NETWORK`: `ap_scan` and
  `network_adapter_list.sh` MISSING, and stopped on the wizard page
  "Network Configuration — Choose a wireless connection" (screenshot sent
  to Jan). Why `Chumby.hasNetwork` was false with a healthy
  `network_status.sh` fixture is not diagnosed.
- heartbeat every 15 s (`_fscommand2` pair + `/tmp/movieheartbeat`), so the
  main loop is alive.
- **2 267 `Avm1::pop: Stack underflow` warnings** in 45 s (64 for the theme
  alone), nearly all during class initialisation. Not diagnosed; noted as a
  candidate fork issue.
- one `stub: AVM1 System.security.allowDomain()`.

`default_theme.swf` alone: runs, renders its layout (TIME / WEATHER /
PHOTOS panels, HUMIDITY / BAROMETER / WINDSPEED column), no panic, waits
for callbacks that never come.

## 4. 860x480 on the DSI box

Box: hostname `chumby-pi-3`, 192.168.42.24 on 2026-09-09; DSI-1 connected at
1024x600, chumby-player 0.9.5, `CHUMBY_RENDERER=tiny-skia`, `CHUMBY_QUALITY=low`.
Method as in chumby-pi `claude-docs/development.md` (2026-07-26 entry): the
panel started by the service (a `CHUMBY_SWF` drop-in under
`/run/systemd/system/chumby-player.service.d/`, removed afterwards), 45 s
settle, `DRM_IOCTL_MODE_ATOMIC` counted over 6 s of `strace -p $(pidof cage)`,
CPU from `top -b -n 2 -d 5`.

| SWF (fullscreen, `showAll` scaling) | commits / 6 s | fps | `ruffle_desktop` CPU | RSS |
|---|---|---|---|---|
| classic `controlpanel.swf` 320x240, before | 71 | 11.8 | 82 % | 130 MB |
| **Dash `controlpanel.swf` 860x480** | **72** | **12.0** | **53 %** | 125 MB |
| **Dash `default_theme.swf` 860x480, standalone** | **57** | **9.5** | **105 %** | 73 MB |
| classic `controlpanel.swf`, restored | 72 | 12.0 | — | — |

So the 860x480 stage does not sink the DSI box: the Dash panel holds the
movie's 12 fps at less CPU than the classic (the plan's "5.4x pixel area"
fear was misplaced, see the note below). The theme alone is the expensive
one — renderer-bound at ~9.5 fps and a full core — and a live theme is what
the home screen will show most of the time. That is the number to carry into
step 3, and the first thing to understand there: what in the Space Theme
costs a core (its modules run `onEnterFrame` continuously,
`com/example/{ListItem,Module,Controls,OtherTime,Pushback}.as`, and its
weather module a `setInterval`), and whether the panel's own
`_enableSlaveUpdates(false)` / mask handling changes that once it hosts the
theme.

Caveats: what each SWF showed on the box was not observed (no `grim` on the
card, and the journal query for the run window returned no host lines) — the
desktop run in §3 is the only evidence that the panel sits in the network
wizard and the theme in its static layout. A hosted theme with a running
clock and a widget may commit more or less than either test.

With `showAll` scaling the Dash panel rasterises at 1024x571, only ~1.2x the
classic's 800x600 letterbox, so the cost driver is shape complexity per
frame, not stage area.
