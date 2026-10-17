# YT Mini Player

A lightweight, always-on-top streaming player for Windows 10/11. It opens YouTube, YouTube Music,
Netflix, Crunchyroll and other services in a borderless floating window on top of the system's
WebView2 runtime, so you keep each site's full interface and your accounts without running a
full browser.

It also has **My music** (*Mi música*): a local music library and player with playlists,
duplicate detection and downloads of openly licensed music from the Internet Archive, with
optional paid plans sold through Lemon Squeezy.

Built with Rust, [tao](https://crates.io/crates/tao) and [wry](https://crates.io/crates/wry).
Version 0.7.0. The in-app interface is in Spanish.

## Install

1. Download `YT-Mini-Player-0.7.0-win64.zip` from the Releases page and extract it anywhere.
2. Pick one:
   - **`Instalar.cmd`** (normal): copies the app to `%LOCALAPPDATA%\Programs\YT Mini Player`,
     adds Start menu and desktop shortcuts, and launches it. No admin rights needed.
   - **`Instalar-superposicion.cmd`** (maximum overlay): the player can stay above fullscreen
     apps and games. Needs admin rights once; see [Maximum overlay](#maximum-overlay).
3. To uninstall either one, run **`Desinstalar.cmd`**. Your sessions and settings are kept.

The installer checks every file against `SHA256SUMS.txt` before installing (and, for maximum
overlay, checks the copy already inside Program Files before signing it). Compare the zip's own
SHA-256 with the one published on the release page.

The executables are not code-signed by a public authority, so Windows SmartScreen may warn on
first launch: click **More info → Run anyway**.

**Requirements:** Windows 10 or 11 with the Microsoft Edge WebView2 Runtime (preinstalled on
Windows 11). If it's missing, the app offers a download link.

## Usage

The app opens on a **service picker**: press a number key (1–9) or click a row. To come back,
use the house button in the toolbar, **Alt+Home**, or the tray menu → *Menú principal*.

| # | Service | DRM |
| --- | --- | --- |
| 1 | YouTube | No |
| 2 | YouTube Music | No |
| 3 | Netflix | Yes |
| 4 | Crunchyroll | Yes |
| 5 | Prime Video | Yes |
| 6 | Disney+ | Yes |
| 7 | HBO Max | Yes |
| 8 | Twitch | No |
| 9 | **Mi música** (your library, see [My music](#my-music)) | No |

The picker also opens **any other site** (e.g. Plex or Jellyfin) from the address field, and can
skip itself at startup ("Abrir el último al iniciar") to go straight to the last service used.

**DRM:** Netflix, Crunchyroll and the other paid services need DRM (Widevine or PlayReady). The
picker checks what this computer's WebView2 runtime offers and says so at the bottom. On a fresh
install the runtime may take a few minutes to download its DRM module; use "Comprobar de nuevo".
Expect the same quality limits as Edge or Chrome on the same PC (often 720p–1080p on the web).

| Action | How |
| --- | --- |
| Move | Drag the video (any service) · Alt + drag anywhere · the ⠿ grip in the toolbar |
| Resize | Drag an edge (6 px) or a corner (12 px) |
| Toolbar | Move the mouse to the right edge |
| Main menu | House button, Alt+Home, or tray → Menú principal |
| More options | The ⋯ button in the toolbar |
| Hide | The eye button, Ctrl+H, or Ctrl+Alt+Y from anywhere |
| Close | The × button, Ctrl+Q, Alt+F4, or tray → Salir |

A plain click on the video still plays and pauses as usual.

**Side toolbar:** move, main menu, back, video-only mode (YouTube), fullscreen, always on top,
hide, more options and close. In short windows (under 300 px) only the essentials remain;
everything else is still in the ⋯ menu.

**⋯ menu:** size presets (S/M/L), snap to a screen corner, opacity, click-through ("ghost")
mode, main menu, open a copied link, Chrome extensions, launch at startup, keyboard shortcuts
and quit.

**Video-only mode (YouTube):** on a watch page, the player fills the whole window and everything
else on the page is hidden. While YouTube is in fullscreen, video-only mode steps aside.

**Over fullscreen apps and games:** with "always on top" enabled, the normal install raises the
player back above borderless/windowed-fullscreen games that mark themselves as topmost: instantly
on focus and z-order changes, and every 15–100 ms while a game keeps re-asserting itself. It
never covers the Start menu, Alt+Tab, the lock screen, screenshot tools or GPU overlays. For apps
that still cover it, use the [maximum overlay](#maximum-overlay) install.

**Game mode:** while a game (or any app) is fullscreen in front, **clicking the player doesn't
take focus away from the game** (the window becomes non-activating for that time). Keyboard and
focus stay with the game, so games that minimize or pause when they lose focus, like Minecraft
and everything built on GLFW or SDL, keep running while you pause or seek the video. Use the
global shortcuts (Ctrl+Alt+P, N, B…) for the keyboard. Alt+Tab back to the player to type in it.
A game that minimizes itself when covered *while it's still in front* is treated as exclusive
fullscreen and left alone; one that minimizes because you Alt+Tabbed away is covered again when
you come back.

**Minecraft (Java Edition):** F11 fullscreen works. If the video settings use a *Fullscreen
Resolution* different from the desktop's, Windows switches display modes and may treat it as
exclusive fullscreen; set it to *Current* (or play maximized in a window).

**System tray:** left-click hides the player, or opens the music bar when it's already hidden;
double-click shows it. Right-click opens a dark menu with what's playing, play/pause, previous,
next, *Mostrar barra de controles*, show/hide, main menu, an *Ajustes* submenu (always on top,
ghost mode, reset opacity, music bar in the taskbar, launch at startup, extensions folder) and
quit. The icon's tooltip shows the current title. While
hidden, audio keeps playing and rendering and memory use are reduced.

### Music bar in the taskbar

A slim media bar that sits **inside the Windows taskbar**, just left of the notification area:
thumbnail, title and artist, previous / play-pause / next, an animated audio visualizer and a
thin lavender progress line along the bottom edge.

- **Music services** (YouTube Music today; Spotify, SoundCloud, Apple Music, Deezer, TIDAL and
  Amazon Music when opened through *Otro sitio*): the bar appears on its own while the service is
  open, whether the player window is visible or hidden.
- **Any other service:** click the tray icon while the player is hidden (or tray menu →
  *Mostrar barra de controles*), or use a media shortcut while hidden. The bar shows for a few
  seconds and hides itself after the mouse leaves.

On the bar:

- Click or drag the progress line to seek (hovering shows the time).
- Mouse wheel anywhere on the bar: volume. The visualizer turns into a volume meter and the
  artist line shows the level for a moment; clicking the visualizer mutes.
- Click the thumbnail to show the player. Right-click anywhere to close the bar (on a music
  service it stays closed until you switch sites or click the tray icon).
- Drag the title to move the bar along the taskbar; the position is remembered. Drop it next to
  the notification area to go back to automatic placement.

Placement details:

- It never takes focus, so you can use it in the middle of a game without leaving it.
- It doesn't cover the app icons: it asks Windows (UI Automation) where the taskbar buttons end
  and narrows itself (down to 250 px, without the visualizer) to fit the gap.
- It follows the taskbar's height (Windows 11: 48 px, Windows 10: 40 px) and DPI, moves with a
  top-docked taskbar, hides when the taskbar auto-hides or a fullscreen app is in front, and
  floats next to the tray if the taskbar is vertical.
- Clicking the taskbar makes Windows raise the taskbar above everything; the bar climbs back on
  top immediately (and within 0.3 s in any other case).
- Tray menu → *Ajustes* → *Barra de música en la barra de tareas* turns off the automatic bar
  for music services (the on-demand bar keeps working).

### Controls while hidden

With the player hidden in the tray, you can still drive playback:

- **Music bar** (above).
- **Global shortcuts** (below): play/pause, next, previous, ±10 s, volume and mute. While the
  player is hidden, each one briefly shows the music bar as an on-screen indicator.
- **Tray menu**: the current title on top, then play/pause, previous and next.

This works on every service. Previous/next and seeking use the controls the site itself declares
to the browser (Media Session API), so they do whatever the site's own buttons do: next episode
on Netflix or Crunchyroll, next video on YouTube. Where a site declares nothing, the app falls
back to YouTube's player API or the `<video>` element; on Netflix it never seeks the video
directly, because Netflix stops playback with an error when that happens.

**Single instance:** launching the app while it's already running (or hidden) brings the
existing window to the front.

### Maximum overlay

Windows keeps a special window layer above all normal windows for accessibility tools such as
the On-Screen Keyboard (*UIAccess*). An app may use it only if its executable is **signed** and
installed under **Program Files**. `Instalar-superposicion.cmd` sets that up, asking for admin
rights once:

1. Installs `yt-mini-player-uia.exe` (same app, manifest with `uiAccess="true"`) to
   `C:\Program Files\YT Mini Player`.
2. Creates a local code-signing certificate, *YT Mini Player (firma local)*, trusts it on this
   computer, signs the executable, and **deletes the private key** so the certificate can't sign
   anything else.
3. Removes the normal per-user install so there's only one copy.

With "always on top" on, the player then stays above fullscreen apps, borderless games and most
"fullscreen" games (those using Windows' fullscreen optimizations), and also above the Start
menu and Task Manager, like the On-Screen Keyboard. `Desinstalar.cmd` removes everything,
certificate included; running `Instalar.cmd` switches back to the normal install. The ⋯ menu
shows "Superposición máxima activa" when it's in effect.

Limits: true **exclusive** fullscreen (some DirectX 9, OpenGL or Vulkan games, or games with
fullscreen optimizations turned off) can't be covered by any window. Only overlays injected into
the game's renderer (Steam, Discord) can do that, and this app deliberately doesn't inject code:
anti-cheat systems treat it as cheating. Use borderless or windowed fullscreen in those games.

### My music

Key **9** on the picker (or the *Mi música* row) opens a music player for **your own files**,
inside the same window and with the same music bar in the taskbar, tray controls and global
shortcuts as any music service.

- **Library:** *Carpeta* adds a folder (MP3, FLAC, M4A/AAC, Ogg, Opus, WAV). Files are read in
  place, never copied: tags, duration, embedded cover art. Folders are re-checked when the page
  opens (new and deleted files) and with ⟳. Search ignores accents and case; sort by recent,
  title, artist or album; play all or shuffle. Each song's ⋯ menu: play next, add to a playlist,
  show in folder, remove from the library, or send to the Recycle Bin.
- **Player:** previous/next, seek, shuffle, repeat (all / one), volume, Space to play/pause,
  Media Session (so the taskbar bar, tray and media keys show and control it).
- **Duplicates** (*Duplicados*): finds the same song three ways and suggests keeping the best
  copy (lossless, then bitrate, then size): **same file** (identical content), **same song**
  (artist and title match once normalized: accents, "feat.", "(Remastered)", track numbers… and
  the length is within 3 s) and **same audio** (an acoustic fingerprint, Chromaprint, the
  algorithm behind AcoustID, matches even when the format or the tags differ). Nothing is deleted
  without confirming, and only to the Recycle Bin. "No son iguales" hides a group for good.
- **Download** (*Descargar*): search the [Internet Archive](https://archive.org) for music under
  a license that allows keeping a copy: **Creative Commons BY, BY-SA, BY-ND, CC0 and public
  domain**. *NonCommercial* (NC) licenses, audiobooks, radio and podcasts are filtered out (in the
  query and again in the app). Preview any song online for free; with a plan, download single songs or a whole album
  as a playlist, in compact MP3 or best quality (FLAC when there is one). Downloads go to
  `Music\YT Mini Player\<artist>\<album>`, three at a time, resume if interrupted, and are
  verified against the Archive's SHA-1. **Songs you already have are not downloaded again:**
  before downloading (same Archive file, or same artist + title + length) and after (identical
  content or same acoustic fingerprint: the copy is discarded and the playlist points to the
  one you had). Each song keeps its license and author; credit them if you share the music.
- **Plans:**

  | Plan | Price | Includes | Playlists |
  | --- | --- | --- | --- |
  | Gratis | free | Library, player, folders, duplicates, searching and previewing free music | 2 |
  | Descargas | 2 USD / month | + downloading songs and whole albums | 5 |
  | De por vida | 10 USD once | everything, for good | unlimited |

  *Plan* opens the checkout in your browser (Lemon Squeezy); paste the license key from the
  receipt e-mail into *Activar*. *Desactivar en este equipo* frees the activation for another PC.

  **Non-invasive by design:**
  - No ads, no pop-ups, no "upgrade" dialogs. Limits are counts (2 / 5 / unlimited playlists),
    shown where they apply: "2 de 2 playlists" next to *Nueva playlist*, and a one-line note
    under the header (closed with ×) if you hit one.
  - Nothing you made is taken away. If a subscription ends, extra playlists stay: you can still
    play, edit and delete them; you just can't create new ones above the free limit. Downloaded
    songs are yours.
  - Without a download plan there are no download buttons: previews stay free, and the album view
    just mentions the plans once.
  - Almost no contact with the store, and no accounts or telemetry: the key is activated once,
    then checked every 7 days (monthly plan, which can be cancelled) or every 30 days (lifetime,
    in case of a refund). Offline, the monthly plan keeps working for 14 days; lifetime never
    expires offline.

### Keyboard shortcuts

In the window (press F1 to see them on screen):

| Shortcut | Action |
| --- | --- |
| 1–9 | Open a service or My music (on the main menu) |
| Space | Play / pause (in My music) |
| Alt+Home | Main menu |
| F11 | Fullscreen |
| Ctrl+T | Always on top |
| Ctrl+M | Video-only mode (YouTube) |
| Ctrl+H | Hide to tray |
| Ctrl+L | Open the copied link (any service in the list) |
| Ctrl+1 / 2 / 3 | Small / medium / large |
| Ctrl+Shift+1 … 4 | Snap to corner ↖ ↗ ↙ ↘ |
| Ctrl+Q | Quit |

System-wide, even when hidden:

| Shortcut | Action |
| --- | --- |
| Ctrl+Alt+Y | Show / hide |
| Ctrl+Alt+P | Play / pause |
| Ctrl+Alt+N / B | Next / previous |
| Ctrl+Alt+→ / ← | Forward / back 10 s |
| Ctrl+Alt+↑ / ↓ | Volume up / down |
| Ctrl+Alt+M | Mute |
| Ctrl+Alt+G | Ghost mode (clicks pass through the window) |
| Ctrl+Alt+PgUp / PgDn | Opacity up / down |
| Ctrl+Alt+0 | Reset opacity to 100% |
| Ctrl+Alt+L | Lock the app (when the app lock is on) |

Global shortcuts use Ctrl+Alt with keys that don't type symbols through AltGr on Spanish and
Latin American layouts. Some older Intel graphics drivers use Ctrl+Alt+arrows to rotate the
screen; in that case Windows won't let the app register them. If another program already uses a
shortcut, the help screen (F1) shows it struck through.
Each site's own shortcuts keep working.

### Chrome extensions

Open ⋯ → Extensiones → **Abrir carpeta** (`%LOCALAPPDATA%\yt_mini_player\extensions`) and drop in:

- unpacked extension folders (with `manifest.json`, also when nested in a single wrapper folder),
- `.crx` files (versions 2 and 3) or `.zip` files. They're extracted automatically and the
  original is moved to `.archivos\`. A newer archive with the same name updates the extension.

Click **Recargar**, then press F5. Each extension can be enabled or disabled from the menu, and
its options page opened if it has one. Deleting an extension's folder uninstalls it on the next
reload.

**Approval:** any program running as you can write to that folder, and an extension sees the
pages (and the signed-in sessions) it runs on. So each extension gets a **SHA-256 fingerprint**
of its whole content and only loads after you approve it in a Windows dialog that lists its
permissions and warns if it can read every site. If a single byte changes, it asks again.
Rejected ones stay off until you press **Recargar**. Archives are capped at 10,000 files and
256 MB unpacked (zip-bomb protection).

WebView2 limitations: no Chrome Web Store, no extension toolbar and no popup windows, so you need
to obtain the `.crx`/`.zip` yourself (many extensions publish them on GitHub). Manifest V2
extensions will likely not load. A recent WebView2 runtime is required; the menu says so if
yours is too old.

## Security and privacy

### Passwords and accounts

- **The app never sees or stores your passwords.** You sign in on each service's own page.
  WebView2's password saving and form autofill are turned off, so the profile keeps no passwords,
  addresses or cards either. Use your usual password manager.
- **Sessions** (the cookies that keep you signed in) live in the WebView2 profile. Chromium
  encrypts them with **AES-256-GCM** and protects that key with **DPAPI**, Windows' per-user
  data protection, the same way Edge and Chrome do.
- **The app's own data is in an encrypted vault** (`vault.rs`): envelope encryption, the same
  scheme as Chrome's `os_crypt`, Electron's `safeStorage` and Signal Desktop. A random 256-bit
  data key encrypts each file (settings, window position, extension ids and approvals, the PIN
  hash, the music library and license) with **AES-256-GCM**; each file has a random nonce and is
  bound to its purpose, so a modified or swapped file is rejected. The data key is wiped from
  memory after use (`zeroize`) and is only stored wrapped twice:
  - by the computer's **security chip (TPM 2.0)**, through Windows' *Microsoft Platform Crypto
    Provider*: a non-exportable RSA-2048 key generated inside the TPM (OAEP padding), the way
    Windows Hello protects its keys. A copy of the disk can't be opened on another computer, even
    knowing the Windows password;
  - and by **DPAPI**, tied to your Windows account (with app-specific entropy).

  No prompts or dialogs. The TPM step runs in the background the first time (generating the key
  inside the chip can take a few seconds) and only replaces `vault.key` after checking that the
  TPM gives the same key back. Without a usable TPM, the key stays protected by DPAPI. Files from
  older versions (`*.json`, in plain text) are migrated and deleted on first launch.
- **No EFS.** Version 0.6.0 encrypted the data folder with EFS, which makes Windows ask the first
  time to *back up the file encryption certificate and key*: a scary dialog for something that
  wasn't needed. Since 0.6.1 the app doesn't use EFS and decrypts a folder 0.6.0 had encrypted (no
  prompt). For whole-disk encryption, including the browser cache, use BitLocker or Windows Device
  Encryption.
- **Sign out everywhere:** tray → Seguridad → *Cerrar sesión en todos los servicios…* clears
  cookies, site storage, cache and history for every service (after a Windows confirmation).
- **Protection status:** tray → Seguridad → *Estado de la protección…* summarizes all of the
  above for this machine.

### App lock

Tray → Seguridad:

- **Bloquear con Windows Hello**: fingerprint, face or Windows PIN. Windows does the check
  (`UserConsentVerifier`) with its own retry limits; the app never sees biometric data.
- **Bloquear con PIN propio…**: at least 6 characters, no repeats (111111) or runs (123456). Only
  an **Argon2id** hash is kept (64 MiB, 3 passes, random salt; OWASP's recommended password KDF),
  inside the encrypted vault. After 5 wrong attempts each further one doubles the wait (30 s,
  1 min, … up to 15 min), and the counter survives restarts. **Forgot it?** The lock screen can
  remove the PIN after verifying you with Windows Hello; without Hello, the only way out is to
  sign out of every service first (so whoever doesn't know the PIN gets nothing).
- The app locks on start, when Windows locks (Win+L, switch user), on sleep, after 10 minutes
  hidden, and with **Ctrl+Alt+L**. While locked, the player is hidden behind its own lock screen
  (a separate webview with a closed protocol), playback pauses, the music bar and the tray stop
  showing what's playing, and every other command is ignored. Changing or turning off the lock
  requires verifying first. New extensions wait for the unlock to be approved.

The lock protects what can be seen and controlled in the app from someone else at the keyboard.
Encryption at rest comes from the vault (TPM + DPAPI) and Chromium's own DPAPI encryption.

### Web content

- **Authenticated IPC:** the app's scripts run inside a single wrapper that captures WebView2's
  real message function and a random 128-bit session token before the page loads. A page can
  still call `window.ipc.postMessage`, but without the token the app drops the message (compared
  in constant time). Our buttons and shortcuts only react to real user input (`isTrusted`), so a
  page can't press them with synthetic events. The functions the app calls in the page (toasts,
  state) require a second key, so a site can't draw fake app messages. App settings are only
  published to the app's own start page.
- **Navigation filter:** https only (plain http is upgraded, except on the local network for
  Plex/Jellyfin-style servers), no `file:`, custom protocols or other schemes, and extension
  pages only for installed extensions.
- **New windows** from a catalog service or a sign-in provider open in the same window;
  anything else opens in your default browser (at most one every 2 s).
- **Downloads are blocked.** **Permissions are denied** (camera, microphone, location,
  notifications, clipboard reading, file system, screen capture…); autoplay is allowed.
- **SmartScreen is on** (wry turns it off by default; with "Otro sitio" any site can be opened).
  Tracking prevention is set to *balanced*.
- **The music bar shows the domain** of pages outside the catalog, so a site can't pass itself
  off as something else down there.
- The start page, music bar and lock screen are served with a strict Content-Security-Policy,
  `no-referrer` and `X-Content-Type-Options: nosniff`, and the bar and lock screen can't navigate
  away from their page.
- **My music:** its page only runs its own script (CSP with the script's SHA-256 hash, not
  `unsafe-inline`), renders tags and titles as text only, and can't be framed. Its commands
  (`music:…`) are a separate closed list accepted **only from that page's URL**, with sizes and
  ids validated; the app pushes data to it only after checking, inside the page, that it is that
  page. Songs and covers are served by id (never by a path from the URL) and require a random
  per-session key, so another site open in the player can't read your files; cross-site requests
  are refused. Library, fingerprints and license are stored in the encrypted vault.
- **Network (My music):** HTTPS through Windows' own TLS stack (SChannel, TLS 1.2+, the system's
  certificates; verification is never turned off), a descriptive User-Agent, few parallel
  downloads and back-off with `Retry-After`, as the Internet Archive asks. Lemon Squeezy keys
  are checked against **this store and these products** (the license API accepts keys from any
  store).
- **Maximum overlay** (UIAccess can send input to administrator windows): that process only
  opens the catalog services and their sign-in pages; no *Otro sitio*, no external links.

### Supply chain

- `.github/workflows/ci.yml`: tests, clippy, JS and browser tests on Windows, and
  `cargo-deny` (RustSec advisories, licenses, crates.io only) on every push.
- `.github/workflows/release.yml`: builds the release on GitHub, runs `cargo audit`, publishes
  `SHA256SUMS.txt` and a signed **build-provenance attestation** (Sigstore) for the zip and the
  executables: `gh attestation verify YT-Mini-Player-<version>-win64.zip --repo <owner>/<repo>`.
- Dependabot opens weekly update PRs for crates and actions.
- The local "maximum overlay" certificate is code-signing only and not a CA, and its private key
  is deleted right after signing.

## Selling the plans (Lemon Squeezy setup)

Payments are handled by [Lemon Squeezy](https://www.lemonsqueezy.com) as merchant of record
(checkout, taxes/VAT, receipts, refunds, the customer portal). The app only uses its public
[License API](https://docs.lemonsqueezy.com/api/license-api) (activate, validate,
deactivate). To connect your store:

1. In Lemon Squeezy, create two products with **license keys enabled**:
   - *Descargas*: subscription, 2 USD / month, activation limit e.g. 3.
   - *De por vida*: single payment, 10 USD, activation limit e.g. 3, keys that never expire.
   Subscription keys expire when the subscription ends; disabling a key (refund) blocks it.
2. Note the **store id**, each product id (or variant id) and each product's **checkout URL**
   (Share → checkout link), plus your customer portal URL if you want a *Gestionar suscripción*
   button.
3. Build with these environment variables (they're compiled in; none of them is secret):

   ```powershell
   $env:YTMP_LS_STORE_ID = '12345'
   $env:YTMP_LS_DOWNLOADS_PRODUCTS = '111111'          # comma-separated product or variant ids
   $env:YTMP_LS_LIFETIME_PRODUCTS = '222222'
   $env:YTMP_LS_DOWNLOADS_CHECKOUT = 'https://<store>.lemonsqueezy.com/buy/<uuid>'
   $env:YTMP_LS_LIFETIME_CHECKOUT = 'https://<store>.lemonsqueezy.com/buy/<uuid>'
   $env:YTMP_LS_PORTAL = 'https://<store>.lemonsqueezy.com/billing'   # optional
   powershell -ExecutionPolicy Bypass -File tools\package.ps1
   ```

   In GitHub, set the same names as repository **variables**; `release.yml` passes them to the
   build. Without them the paid features stay locked and the *Plan* tab says the store isn't
   configured. `YTMP_DEV_UNLOCK=1` unlocks everything for development (with a red banner);
   never ship that build.
4. Test with Lemon Squeezy's test mode first: buy, activate, cancel the subscription, refund.

What to check before selling (not legal advice):

- **Internet Archive:** its [terms of use](https://archive.org/about/terms.php) say access to
  the collections is provided free of charge for scholarship and research purposes. The app only searches and
  downloads on the user's request, never mirrors or bulk-crawls, and only offers items whose
  license allows copying, but you sell access to a feature that relies on the Archive: read the
  terms yourself, and consider contacting them. Per-item licenses are set by uploaders and can be
  wrong; the app shows each item's license and author and links to it.
- The paid plans unlock **features of the app** (downloading, playlists), not the music: the
  music itself is free under its license. Say so on your product page.
- Jamendo, Spotify and YouTube are deliberately **not** download sources: Jamendo's API terms
  forbid offline caching for apps and require a commercial agreement, Spotify is DRM-protected,
  and downloading from YouTube breaks its terms of service.
- License checks run in the client, so a determined user can patch them out of an open-source
  build. That's the usual trade-off for desktop apps; the server side (Lemon Squeezy) is what
  controls refunds and subscriptions.

## Building from source

Requirements: Rust (stable) and **Visual Studio Build Tools** with *Desktop development with
C++*. VS Code alone is not enough: the build needs `link.exe`.

```powershell
cargo run                                                   # debug build with console logs
powershell -ExecutionPolicy Bypass -File tools\package.ps1  # both variants + release zip in dist\
```

See [Selling the plans](#selling-the-plans-lemon-squeezy-setup) for the store variables.

`YTMP_UIACCESS=1` builds the maximum-overlay variant (manifest with `uiAccess="true"`). That
executable only starts once signed and installed under Program Files, so use the installer.

MSVC builds link the WebView2 loader statically into the `.exe`. The prebuilt release zip was
cross-compiled from Linux with MinGW (GNU target), so it ships the redistributable
`WebView2Loader.dll` next to the executables.

## Changelog

**0.7.0** — games and My music (includes 0.6.1)
- Games: clicking the player while a fullscreen game is in front no longer takes focus from the
  game ("game mode", `WS_EX_NOACTIVATE` + `MA_NOACTIVATE`), so Minecraft and other GLFW/SDL games
  no longer minimize or pause. Before, that minimize also made the player think the game was
  exclusive fullscreen and stop covering it; exclusive fullscreen is now detected by event order
  (minimized while still in front), so Alt+Tab no longer counts. Instant reaction to z-order
  changes (`EVENT_OBJECT_REORDER`) and faster polling while a game keeps re-asserting itself.
- New: **My music** (key 9): local library (folders, tags, covers), player with shuffle/repeat
  and Media Session, playlists, duplicate finder (identical file, same song, same audio via
  Chromaprint), and downloads of openly licensed music from the Internet Archive (single songs or
  whole albums as playlists, resumable, SHA-1 verified, skipping songs you already have).
- New: plans through Lemon Squeezy (*Descargas* 2 USD/month, *De por vida* 10 USD once), sold
  without nagging: 2 / 5 / unlimited playlists, inline limits instead of dialogs, nothing taken
  away when a plan ends, license checks every 7 or 30 days and offline grace (14 days monthly,
  forever lifetime).
- Security: closed `music:` protocol only from the music page, CSP by script hash, per-session
  key for songs and covers, everything stored in the vault, HTTPS through SChannel.

**0.6.1** — no more EFS prompt
- The data folder is no longer encrypted with EFS, so Windows no longer asks to back up an
  "encryption certificate and key". A folder encrypted by 0.6.0 is decrypted on first launch.
- The vault key is now bound to the computer's TPM (non-exportable key inside the chip) in addition
  to the Windows account, set up silently in the background; DPAPI only if there's no usable TPM.

**0.6.0** — security
- Accounts: password saving and autofill off; *Sign out everywhere*; data folder encrypted with
  EFS where available; app data in an AES-256-GCM vault with a DPAPI-wrapped key (old plain
  JSON files are migrated and deleted).
- App lock with Windows Hello or an Argon2id-hashed PIN, with growing waits after failures;
  locks on start, Win+L, sleep, after 10 min hidden and with Ctrl+Alt+L.
- Authenticated IPC (session token, `isTrusted`, keyed host functions); navigation filter,
  http → https, blocked downloads and permissions, popups to the default browser, SmartScreen
  on, restricted maximum-overlay mode, domain shown in the music bar.
- Extensions only load after approval (SHA-256 fingerprint + permissions dialog); zip-bomb limits.
- Installer: SHA-256 checks (on the Program Files copy before signing), non-CA certificate.
- CI with tests, clippy, cargo-deny and cargo audit; signed release provenance; Dependabot.

**0.5.1**
- Fixed: the music bar had a wide white frame around it. tao's "undecorated with shadow" mode
  keeps an invisible resize frame outside the client area, which showed up as a white border on
  this window. The bar now has no shadow and handles `WM_NCCALCSIZE` itself, so the webview
  fills the whole window edge to edge.

**0.5.0**
- Fixed: the quick-controls panel never appeared. Its webview was created with different
  WebView2 environment options than the player's (browser extensions on vs. off) over the same
  profile folder, which WebView2 rejects. It now shares the player's environment.
- New: the controls live **inside the taskbar** as a music bar, left of the notification area,
  sized to the taskbar's height and DPI. It shows up on its own while a music service is open
  (YouTube Music, plus Spotify, SoundCloud, Apple Music, Deezer, TIDAL and Amazon Music through
  *Otro sitio*); adding a music service to the catalog is a single `music: true` flag.
- The bar narrows to avoid covering app icons (UI Automation), can be dragged along the taskbar,
  hides with auto-hide or fullscreen apps, and stays above the taskbar when you click it.
- New setting in tray → *Ajustes*: *Barra de música en la barra de tareas*.

**0.4.1**
- Redesigned quick controls: a slim media bar (440×64) with the thumbnail, title and artist,
  transport buttons, an animated audio visualizer that doubles as a volume meter, and a
  draggable progress line. Lavender window border on Windows 11 (drawn by the page on
  Windows 10). The ±10 s buttons and volume slider are gone from the panel; the shortcuts and the
  mouse wheel cover them.
- The tray menu is now dark and tidier: window options moved into an *Ajustes* submenu.

**0.4.0**
- New: quick controls next to the tray icon for when the player is hidden: thumbnail, title,
  seek bar, previous / play-pause / next, ±10 s, mute and volume. It never takes focus.
- New: global shortcuts for ±10 s (Ctrl+Alt+←/→), volume (Ctrl+Alt+↑/↓) and mute (Ctrl+Alt+M);
  while hidden, each one shows the quick controls briefly as an on-screen indicator.
- New: the tray menu shows what's playing and has play/pause, previous and next; the tray icon's
  tooltip shows the title; double-click shows the player.
- Previous/next now work on every service through the site's own Media Session controls.

**0.3.0**
- New: service picker at startup with YouTube, YouTube Music, Netflix, Crunchyroll, Prime Video,
  Disney+, HBO Max and Twitch, plus any other site; a DRM check; and a main-menu button, Alt+Home
  shortcut and tray entry to come back.
- New: dragging the video moves the window on every service, not only YouTube.
- New: optional maximum-overlay install (UIAccess) to stay above fullscreen apps and games.
- Improved: in the normal install, the player reacts within ~¼ s when a fullscreen app covers it.
- Each service gets the user agent it needs: Chrome for Google's services (sign-in), Edge, which
  is what WebView2 really is, for the rest.

**0.2.1**
- Fixed: video-only mode let YouTube's title and recommendations paint over the video (most
  visible in fullscreen). It now hides everything except the player and turns itself off while
  YouTube is in fullscreen.
- Fixed: the player could end up behind borderless fullscreen games that mark themselves as
  topmost. A new guard raises it back above them without stealing focus.

**0.2.0**
- Drag the video to move the window, side toolbar and ⋯ menu, video-only mode, hide to tray,
  local and global shortcuts, Chrome extensions, installer and single instance.

## Project layout

| File | Contents |
| --- | --- |
| `src/main.rs` | Entry point, single instance, start page or last service, event loop |
| `src/app.rs` | App state and execution of every command |
| `src/services.rs` | Service catalog, URL matching, user-typed addresses |
| `src/home.rs` · `src/home.html` | Service picker served at `https://ytmp.localhost/` |
| `src/ipc.rs` | Page → host protocol (closed list) and internal commands |
| `src/window.rs` | Borderless window, icon, restored geometry |
| `src/webview.rs` | WebView2 with isolated profile, injected scripts, per-service user agent |
| `src/extensions.rs` | `.crx`/`.zip` handling, manifests, sync with the WebView2 profile |
| `src/tray.rs` | Tray icon and menu |
| `src/hotkeys.rs` | Global shortcuts |
| `src/platform.rs` | Win32: opacity, click-through, clipboard, corners, UIAccess, launch at startup |
| `src/topmost.rs` | Keeps the player above borderless fullscreen games |
| `src/links.rs` | Normalization of YouTube links from the clipboard |
| `src/config.rs` | Paths and encrypted state files (`settings.enc`, `window.enc`, `extensions.enc`) |
| `src/lifecycle.rs` | Lower memory and rendering while minimized or hidden |
| `src/inject.js` | Drag the video on any site, Alt+drag, edge resizing |
| `src/ui.js` | Toolbar, menu, help, toasts, local shortcuts, video-only mode |
| `src/maintenance.js` | Preventive reload during long YouTube sessions |
| `src/media.js` | Reads what's playing and runs play/seek/volume actions on any site |
| `src/media.rs` | Playback state (validated) and media actions |
| `src/mini.rs` · `src/mini.html` | Music bar in the taskbar (no focus, shares the player's WebView2 environment) |
| `src/dock.rs` · `src/taskbar.rs` | Where the bar goes in the taskbar (pure, tested) · Win32/UI Automation side |
| `src/vault.rs` | Encrypted vault: AES-256-GCM, key wrapped by the TPM and DPAPI |
| `src/lock.rs` · `src/lockview.rs` · `src/lock.html` | App lock: Windows Hello, Argon2id PIN, lock screen |
| `src/policy.rs` | Navigation, new-window and IPC-token rules (pure, tested) |
| `src/music.html` · `src/music/` | My music: page, service thread, library and duplicates, file scanning, acoustic fingerprint, Internet Archive, download queue, media/cover serving, Windows dialogs |
| `src/license.rs` | Lemon Squeezy licenses: plans, activation, validation, offline grace |
| `src/net.rs` | HTTPS client (SChannel), retries with `Retry-After`, resumable downloads |
| `src/security.rs` | Removal of 0.6.0's EFS encryption, Windows lock/sleep notifications, system dialogs |
| `build.rs` | Icon, version info and manifest (normal or UIAccess) |
| `dist/` | Installer (`install.ps1`, `Instalar.cmd`, `Instalar-superposicion.cmd`, `Desinstalar.cmd`, `LEEME.txt`) |
| `tools/` | `measure.ps1` (memory and CPU), `package.ps1` (packaging) |

User data lives in `%LOCALAPPDATA%\yt_mini_player`: `profile\`
(sessions for every service), `extensions\`, `vault.key`, the encrypted `*.enc` files and
`music\` (encrypted library, fingerprints and license, plus a cover cache). Downloaded songs go
to `Music\YT Mini Player`; your own folders are only read.

## Tests

```powershell
cargo test                               # protocol, services, start page, settings, topmost, …
node tests/inject.test.js                # drag and edges, simulated DOM
node tests/maintenance.test.js           # preventive reload, simulated clock
python tests/ui_e2e.py screenshots       # YouTube UI in Chromium (pip install playwright)
python tests/home_e2e.py screenshots     # service picker and a non-YouTube player
python tests/mini_e2e.py screenshots     # music bar + media.js on a real <audio>
python tests/security_e2e.py screenshots # hostile page vs. the app's scripts, lock screen
python tests/music_e2e.py screenshots    # My music page with the app's real wrapper and CSP
node tests/media.test.js                 # media.js with simulated Media Session and players
powershell -ExecutionPolicy Bypass -File tests\measure.test.ps1
```

`ui_e2e.py` loads a page that mimics YouTube's layout (fixed header, two columns, nested stacking
contexts) under its real Trusted Types policy and checks 49 behaviors. `home_e2e.py` serves the
picker with the app's real Content-Security-Policy and checks 36 behaviors: keys 1–9, the address
field, the startup switch, the DRM check (both outcomes), small windows, and dragging, clicking
and seeking on a generic streaming player. `mini_e2e.py` checks the music bar at taskbar sizes
(360×40 for Windows 11, 360×32 for Windows 10, 48 px floating, and the narrow 250–280 px
variants): every button, click and drag on the progress line, dragging the bar by its title,
wheel volume meter, hover, right-click close, empty/live/muted states, broken thumbnail and
layout; then it runs media.js against Chromium's real Media Session with a real audio element
(74 checks in total). `security_e2e.py` loads a hostile page that writes to `window.ipc`,
hooks WebView2's message function, fires synthetic keys and clicks, fakes toasts and searches for
the token, and checks that none of it gets through; then it drives the lock screen (Hello, PIN,
waits, new PIN, small windows). The browser tests inject exactly the wrapper `webview.rs` builds
(`tests/bundle.py` reads it from the source). `music_e2e.py` serves the My music page with its
hash-based CSP and drives it as the app would: library, filtering with accents, playback through
`/media` with the session key, Media Session and media.js reports (taskbar bar), next/pause from
the app, track menus, Recycle Bin confirmation, upsells, plan cards, license activation,
Internet Archive search, album view, previews, album download, the download queue, duplicates,
plan limits shown inline (never a dialog), playlists, a forged message without the key, and 480×270 / 256×144 windows (80 checks).

The Rust tests include fake HTTP servers for the Internet Archive and Lemon Squeezy: searching,
opening an album, downloading it as a playlist in parallel (with a song that turns out to be the
same audio as another and is discarded), resuming with `Range`, rejecting a bad SHA-1,
cancelling, activating/validating/deactivating keys from this store and from another one, expiry
and offline grace. Fingerprint tests use synthesized audio (same melody at two sample rates vs.
a different one). The topmost guard was also checked in Wine against fake games: one that
re-asserts topmost every 30 ms, a GLFW-like one that minimizes on focus loss (click on the
player, Alt+Tab and back) and an exclusive-fullscreen one; and the music service, built for
Windows, was run in Wine (DPAPI vault, Windows paths, Recycle Bin, media serving).

Manual checks still pending on real Windows:

- [ ] Netflix and Crunchyroll play (and the picker's DRM line matches what happens).
- [ ] YouTube Music: the music bar appears inside the taskbar, left of the clock, without covering app icons; buttons work and the app in front keeps focus.
- [ ] Clicking an empty spot of the taskbar: the bar stays visible (above the taskbar).
- [ ] The bar hides with a fullscreen video/game and with auto-hide, and comes back.
- [ ] Dragging the bar's title moves it along the taskbar and the position survives a restart.
- [ ] Hidden, on YouTube or Netflix: clicking the tray icon shows the bar for a few seconds.
- [ ] Hidden: Ctrl+Alt+N / ↑ / M show the panel briefly and act on YouTube and on Netflix.
- [ ] The music bar has a lavender border on Windows 11 and the tray menu is dark.
- [ ] Google sign-in still works on YouTube after visiting another service.
- [ ] Dragging the video moves the window on YouTube and on Netflix without pausing.
- [ ] Hide with the eye button and bring it back with Ctrl+Alt+Y or the tray; audio keeps playing while hidden.
- [ ] Fullscreen (F11 or the site's button) fills the monitor; Esc restores the previous size.
- [ ] With "always on top", the player stays above a borderless game after clicking into the game.
- [ ] Maximum overlay: the installer finishes, the ⋯ menu shows it active, and the player stays above a fullscreen app.
- [ ] Maximum overlay: launch at startup starts hidden in the tray; `Desinstalar.cmd` removes the certificate.
- [ ] 70% opacity looks translucent (not black); ghost mode lets clicks through.
- [ ] A `.crx` extension shows up in the menu and works after F5, after approving it in the dialog.
- [ ] Windows Hello lock: enabling asks for Hello; Win+L then coming back shows the lock screen; Hello unlocks.
- [ ] PIN lock: wrong PIN 5 times starts the wait; the right PIN unlocks.
- [ ] Tray → Seguridad → Estado de la protección says the key is protected by the TPM (on a PC with TPM 2.0, after the first launch).
- [ ] Upgrading from 0.6.0 on Windows Pro: no EFS dialog; `cipher /u /n` no longer lists files under `yt_mini_player`; sessions are kept.
- [ ] Sign out everywhere: after it, YouTube and Netflix ask to sign in again.
- [ ] Upgrading from 0.5.x keeps the settings (the old `.json` files disappear, `.enc` appear).
- [ ] Minecraft (F11 fullscreen): the player stays on top; clicking pause on it doesn't minimize the game.
- [ ] A borderless game that re-asserts topmost: the player doesn't flicker.
- [ ] My music: add a folder with a few thousand songs; scanning finishes and the list scrolls smoothly.
- [ ] My music: songs play, seeking works on FLAC and MP3, and the taskbar bar shows the cover.
- [ ] Duplicates: a song in MP3 and FLAC is found as "Mismo audio"; the Recycle Bin gets the other copy.
- [ ] Download (with a test store or `YTMP_DEV_UNLOCK=1`): an album downloads as a playlist; downloading it again downloads nothing.
- [ ] Lemon Squeezy test mode: buy, activate, restart offline, deactivate.
- [ ] Free plan: a third playlist can't be created (inline note, no dialog); after cancelling a test subscription with 5 playlists, all 5 remain and can be edited.

## Known issues

- DRM depends on the WebView2 runtime. Microsoft says Widevine works in WebView2, but there are
  reports of intermittent failures, especially on fresh profiles. The picker's DRM line shows
  what's available; if it says none, wait a few minutes after installing and check again.
- Exclusive fullscreen games can't be covered by any window, even with maximum overlay. Use
  borderless or windowed fullscreen. If a game minimizes itself while in front right after the
  player covers it, the player stops trying for that window.
- A game that re-asserts "topmost" on every frame can still win for a few milliseconds at a time
  (a faint flicker); the maximum-overlay install avoids it entirely.
- My music plays what WebView2 plays: MP3, AAC/M4A, FLAC, Ogg Vorbis, Opus and WAV. WMA, ALAC and
  AIFF files are not listed. The acoustic fingerprint covers the first 60 s: two different edits
  of a song (radio edit vs. album) are not duplicates; Opus files are compared by content and
  tags only.
- Internet Archive search depends on its service; items with only a ZIP, or without a usable
  license, can't be downloaded. Per-item licenses are declared by uploaders.
- The first scan of a large library reads tags and a 192 KB content hash of each file; on a slow
  disk, thousands of songs take a few minutes (later scans only check size and date).
- Maximum overlay: native WebView2 pop-ups (the right-click menu, `<select>` drop-downs) may open
  behind the player while it's always on top. Turn "always on top" off for a moment (Ctrl+T) to
  use them.
- Google may block sign-in inside embedded webviews despite the user-agent override.
- Sign-in pop-ups (e.g. "Sign in with Google" on a third-party site) open in the same window
  instead of a pop-up; flows that depend on talking back to the opener window may not finish.
  Signing in directly on the service's page works.
- The app lock is access control for the app's window, not encryption of the WebView2 profile
  (WebView2 needs it readable while running). Cookies are encrypted by Chromium; site storage
  and cache are not (as in Edge and Chrome): enable BitLocker / Device Encryption for the whole
  disk.
- The TPM and DPAPI protect against other accounts, offline disk access and copies of the disk.
  Malware already running as your own Windows user can do anything you can; no app-level
  encryption stops that.
- If Windows resets your account's protection keys (e.g. an administrator resets your password),
  the TPM is cleared, or the motherboard is replaced, the vault can't be read: the app starts
  with default settings (sign in again, re-activate the license, re-add music folders).
- Testers of 0.6.0 who chose "back up later" in the EFS dialog may see that reminder once more;
  choosing *Never* dismisses it. After upgrading, the app no longer has EFS-encrypted files.
- **Opacity and ghost mode are experimental.** They rely on a Win32 layered window, and how that
  combines with WebView2 rendering hasn't been verified. If the window turns black, Ctrl+Alt+0
  or tray → Ajustes → "Opacidad al 100 %" restores it.
- Video-only mode and drag detection depend on each site's page structure and may need updates
  in `ui.js` / `inject.js` if a site changes it.
- Next/previous and seeking depend on what each site declares; if a site declares nothing (or,
  on Netflix, no seek control), those buttons stay disabled in the music bar.
- The music bar never takes focus, so it has no keyboard shortcuts of its own; use the global
  ones.
- The music bar is a separate always-on-top window placed over the taskbar, not part of it
  (embedding a window into Explorer's taskbar ties Explorer to the app: if the app hung, so would
  the taskbar). Where the app icons end is read through UI Automation; if Windows doesn't answer,
  the bar uses its full width and may cover the last icons on small screens; drag it or close it
  with a right-click. With many app icons there may be no room at all: it then keeps its minimum
  width over the icons.
- With the maximum-overlay install the bar sits in a layer above the Start menu and the taskbar
  flyouts; it only occupies the taskbar strip, so it doesn't overlap them.
- Hardware media keys and the Windows media flyout may also control the player through WebView2
  (it can appear there as "Microsoft Edge WebView2"); the app doesn't depend on it.
- An extension's options page opens in the same window (Alt+← to go back).
- The right-edge resize strip overlaps part of the page scrollbar.
- The preventive reload drops YouTube's temporary "Add to queue" list (playlists are kept).
