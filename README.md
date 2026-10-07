# YT Mini Player

A lightweight, always-on-top YouTube player for Windows 10/11. It runs the full YouTube site in a
borderless floating window on top of the system's WebView2 runtime, so you keep playlists,
quality settings and your Premium account without running a full browser.

Built with Rust, [tao](https://crates.io/crates/tao) and [wry](https://crates.io/crates/wry).
Version 0.2.0. The in-app interface is in Spanish.

## Install

1. Download `YT-Mini-Player-0.2.0-win64.zip` from the Releases page and extract it anywhere.
2. Double-click **`Instalar.cmd`**. It copies the app to
   `%LOCALAPPDATA%\Programs\YT Mini Player`, adds Start menu and desktop shortcuts, and launches
   it. No admin rights needed.
3. To uninstall, run **`Desinstalar.cmd`**. Your session and settings are kept.

The executable is not code-signed, so Windows SmartScreen may warn on first launch:
click **More info → Run anyway**.

**Requirements:** Windows 10 or 11 with the Microsoft Edge WebView2 Runtime (preinstalled on
Windows 11). If it's missing, the app offers a download link.

## Usage

| Action | How |
| --- | --- |
| Move | Drag the video · Alt + drag anywhere · the ⠿ grip in the toolbar |
| Resize | Drag an edge (6 px) or a corner (12 px) |
| Toolbar | Move the mouse to the right edge |
| More options | The ⋯ button in the toolbar |
| Hide | The eye button, Ctrl+H, or Ctrl+Alt+Y from anywhere |
| Close | The × button, Ctrl+Q, Alt+F4, or tray → Salir |

A plain click on the video still plays and pauses as usual.

**Side toolbar:** move, back, video-only mode, fullscreen, always on top, hide, more options and
close. In short windows (under 300 px) only the essentials remain; everything else is still in
the ⋯ menu.

**⋯ menu:** size presets (S/M/L), snap to a screen corner, opacity, click-through ("ghost")
mode, YouTube home, open a copied YouTube link, Chrome extensions, launch at startup, keyboard
shortcuts and quit.

**Video-only mode:** on a watch page, the player fills the whole window and the YouTube header
and page content are hidden.

**System tray:** left-click shows or hides the player; right-click opens a menu with always on
top, ghost mode, reset opacity, launch at startup, the extensions folder and quit. While hidden,
audio keeps playing and rendering and memory use are reduced.

**Single instance:** launching the app while it's already running (or hidden) brings the
existing window to the front.

### Keyboard shortcuts

In the window (press F1 to see them on screen):

| Shortcut | Action |
| --- | --- |
| F11 | Fullscreen |
| Ctrl+T | Always on top |
| Ctrl+M | Video-only mode |
| Ctrl+H | Hide to tray |
| Ctrl+L | Open the copied YouTube link |
| Ctrl+1 / 2 / 3 | Small / medium / large |
| Ctrl+Shift+1 … 4 | Snap to corner ↖ ↗ ↙ ↘ |
| Alt+Home | YouTube home |
| Ctrl+Q | Quit |

System-wide, even when hidden:

| Shortcut | Action |
| --- | --- |
| Ctrl+Alt+Y | Show / hide |
| Ctrl+Alt+P | Play / pause |
| Ctrl+Alt+N / B | Next / previous |
| Ctrl+Alt+G | Ghost mode (clicks pass through the window) |
| Ctrl+Alt+PgUp / PgDn | Opacity up / down |
| Ctrl+Alt+0 | Reset opacity to 100% |

Global shortcuts use Ctrl+Alt with letters that don't type symbols through AltGr on Spanish and
Latin American layouts, and avoid Ctrl+Alt+arrows (some Intel drivers use them to rotate the
screen). If another program already uses a shortcut, the help screen shows it struck through.
YouTube's own shortcuts (Space, K, J, L, F, M…) keep working.

### Chrome extensions

Open ⋯ → Extensiones → **Abrir carpeta** (`%LOCALAPPDATA%\yt_mini_player\extensions`) and drop in:

- unpacked extension folders (with `manifest.json`, also when nested in a single wrapper folder),
- `.crx` files (versions 2 and 3) or `.zip` files. They're extracted automatically and the
  original is moved to `.archivos\`. A newer archive with the same name updates the extension.

Click **Recargar**, then press F5. Each extension can be enabled or disabled from the menu, and
its options page opened if it has one. Deleting an extension's folder uninstalls it on the next
reload.

WebView2 limitations: no Chrome Web Store, no extension toolbar and no popup windows, so you need
to obtain the `.crx`/`.zip` yourself (many extensions publish them on GitHub). Manifest V2
extensions will likely not load. A recent WebView2 runtime is required; the menu says so if
yours is too old.

## Building from source

Requirements: Rust (stable) and **Visual Studio Build Tools** with *Desktop development with
C++*. VS Code alone is not enough: the build needs `link.exe`.

```powershell
cargo run                                                   # debug build with console logs
powershell -ExecutionPolicy Bypass -File tools\package.ps1  # release zip in dist\
```

MSVC builds link the WebView2 loader statically into the `.exe`. The prebuilt release zip was
cross-compiled from Linux with MinGW (GNU target), so it ships the redistributable
`WebView2Loader.dll` next to the executable.

## Project layout

| File | Contents |
| --- | --- |
| `src/main.rs` | Entry point, single instance, event loop |
| `src/app.rs` | App state and execution of every command |
| `src/ipc.rs` | Page → host protocol (closed list) and internal commands |
| `src/window.rs` | Borderless window, icon, restored geometry |
| `src/webview.rs` | WebView2 with isolated profile, injected scripts, extensions enabled |
| `src/extensions.rs` | `.crx`/`.zip` handling, manifests, sync with the WebView2 profile |
| `src/tray.rs` | Tray icon and menu |
| `src/hotkeys.rs` | Global shortcuts |
| `src/platform.rs` | Win32: opacity, click-through, clipboard, corners, launch at startup |
| `src/links.rs` | Validation of YouTube links from the clipboard |
| `src/config.rs` | Paths, `settings.json`, `window.json`, `extensions.json` |
| `src/lifecycle.rs` | Lower memory and rendering while minimized or hidden |
| `src/inject.js` | Drag the video, Alt+drag, edge resizing |
| `src/ui.js` | Toolbar, menu, help, toasts, local shortcuts, video-only mode |
| `src/maintenance.js` | Preventive reload during long sessions |
| `dist/` | Installer (`install.ps1`, `Instalar.cmd`, `Desinstalar.cmd`, `LEEME.txt`) |
| `tools/` | `measure.ps1` (memory and CPU), `package.ps1` (packaging) |

User data lives in `%LOCALAPPDATA%\yt_mini_player`: `profile\` (session), `extensions\`,
`settings.json`, `window.json` and `extensions.json`.

## Tests

```powershell
cargo test                               # protocol, links, extensions, settings, corners
node tests/inject.test.js                # drag and edges, simulated DOM
node tests/maintenance.test.js           # preventive reload, simulated clock
python tests/ui_e2e.py screenshots       # full UI in Chromium (pip install playwright)
powershell -ExecutionPolicy Bypass -File tests\measure.test.ps1
```

`ui_e2e.py` loads a page that mimics YouTube's player under its real Trusted Types policy and
checks 40 behaviors: toolbar, menu, dragging without pausing, shortcuts, video-only mode,
fullscreen and no console errors.

Manual checks still pending on real Windows:

- [ ] Dragging the video moves the window without pausing it or leaving it at 2x speed.
- [ ] Hide with the eye button and bring it back with Ctrl+Alt+Y or the tray; audio keeps playing while hidden.
- [ ] Fullscreen (F11 or YouTube's button) fills the monitor; Esc restores the previous size.
- [ ] 70% opacity looks translucent (not black); ghost mode lets clicks through.
- [ ] A `.crx` extension shows up in the menu and works after F5.
- [ ] Launch at startup starts hidden in the tray.

## Known issues

- Google may block sign-in inside embedded webviews despite the user-agent override.
- **Opacity and ghost mode are experimental.** They rely on a Win32 layered window, and how that
  combines with WebView2 rendering hasn't been verified. If the window turns black, Ctrl+Alt+0
  or tray → "Opacidad al 100 %" restores it.
- Video-only mode and drag detection depend on YouTube's page structure and may need updates
  in `ui.js` / `inject.js` if YouTube changes it.
- An extension's options page opens in the same window (Alt+← to go back).
- The right-edge resize strip overlaps part of the page scrollbar.
- The preventive reload drops the temporary "Add to queue" list (playlists are kept).
