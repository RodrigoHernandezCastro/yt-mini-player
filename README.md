# YT Mini Player

A lightweight, always-on-top streaming player for Windows 10/11. It opens YouTube, YouTube Music,
Netflix, Crunchyroll and other services in a borderless floating window on top of the system's
WebView2 runtime, so you keep each site's full interface and your accounts without running a
full browser.

It also has **My music**: a local music library and player with playlists,
duplicate detection and downloads of openly licensed music from the Internet Archive, with
optional paid plans sold through Lemon Squeezy. And **Watch together**: share a code and up to
five friends see and hear your window (and can use it, if you let them), or their own players
follow yours.

Built with Rust, [tao](https://crates.io/crates/tao) and [wry](https://crates.io/crates/wry).
Version 0.13.3. The interface is in **English, Spanish, Russian, Polish and Japanese** (see
[Languages](#languages)).

## Install

1. Download `YT-Mini-Player-0.13.3-win64.zip` from the Releases page and extract it anywhere.
2. Pick one:
   - **`Install.cmd`** (normal): copies the app to `%LOCALAPPDATA%\Programs\YT Mini Player`,
     adds Start menu and desktop shortcuts, and launches it. No admin rights needed.
   - **`Install-overlay.cmd`** (maximum overlay): the player can stay above fullscreen
     apps and games. Needs admin rights once; see [Maximum overlay](#maximum-overlay).
3. To uninstall either one, run **`Uninstall.cmd`**. Your sessions and settings are kept.

The installer's messages follow the Windows display language (any of the five, English
otherwise). Until 0.7 the installer files had Spanish names (`Instalar.cmd`, `Desinstalar.cmd`,
`LEEME.txt`…); upgrading only needs the new `Install.cmd`, and `Uninstall.cmd` also removes the
certificate an older overlay install created.

The installer checks every file against `SHA256SUMS.txt` before installing (and, for maximum
overlay, checks the copy already inside Program Files before signing it). Compare the zip's own
SHA-256 with the one published on the release page.

The executables are not code-signed by a public authority, so Windows SmartScreen may warn on
first launch: click **More info → Run anyway**.

**Requirements:** Windows 10 or 11 with the Microsoft Edge WebView2 Runtime (preinstalled on
Windows 11). If it's missing, the app offers a download link.

## Languages

English, Spanish (Español), Russian (Русский), Polish (Polski) and Japanese (日本語). On first
start the app follows the Windows display language, falling back to English. Change it any time,
without restarting:

- **Start screen:** the globe button at the top right, which shows the current language in its
  own name and lists each one as natives write it (with its name in the current language
  underneath).
- **Tray menu → Language**, always labeled in English too ("Язык / Language"), so it can be found
  from a language you can't read.
- **Side bar → ⋯ → App → Language.**

The choice is saved with the settings. Pages switch in place (what's playing, the open tab and
what you typed stay); the music bar and lock screen reload in the new language.

The texts were written for each language rather than translated word for word:

- **Plurals** follow each language's rules (Russian and Polish have three forms: *1 песня, 2
  песни, 5 песен*; *1 utwór, 2 utwory, 5 utworów*), and numbers, sizes, percentages, prices and
  dates are formatted by the language (`1 234`, `14,4 МБ`, `70 %` in Russian; `$2` in English,
  `2 US$` in Spanish). Lists of systems use each language's "and" (*Widevine i PlayReady*).
- **Windows' own words:** Recycle Bin is *Корзина*, *Kosz*, *ごみ箱*; "Always on top" is *Поверх всех
  окон*, *Zawsze na wierzchu*, *常に手前に表示*; Settings paths match each Windows language.
- **Quotes and punctuation:** «…» in Russian and Spanish, „…” in Polish, 「…」 in Japanese;
  Japanese sentences shown one after another are joined without a space.
- **Keys as printed on the keyboard:** a Spanish keyboard says *Inicio* and *RePág*, so the help
  says *Alt+Inicio*; other languages keep *Home* and *PgUp*.
- **Keyboard layouts:** global shortcuts are Ctrl+Alt+letter, which on some layouts is AltGr
  typing a character (Polish AltGr+N is *ń*, AltGr+L is *ł*). The app checks every installed
  layout and uses an alternative key instead of stealing the character (*next* becomes
  Ctrl+Alt+. and *lock* Ctrl+Alt+K); F1, the tray menu and the music bar show the keys that really
  work. Window shortcuts (Ctrl+T, Ctrl+H…) are matched by the key's letter on Latin layouts and
  by its position on others, so they work on a Russian keyboard too.
- **System dialogs** (extension approval, sign-out, forgotten PIN) use buttons with the action's
  name ("Sign out", "Don't load") instead of Yes/No, which Windows would label in its own
  language rather than the app's.
- **Plan names** were chosen per language (*Lifetime* is *Навсегда*, *Dożywotni*, *買い切り*),
  and sorting follows the language's alphabet.
- Pages load Japanese fonts (Yu Gothic UI, Meiryo UI) and set `<html lang>` so each script gets
  the right glyphs; long Russian or Japanese tab names collapse to icons when they don't fit.

Texts live in `src/locales/<code>.json`, one flat file per language with `{name}` placeholders
and plural forms by CLDR category. `cargo test` checks that every language has every key with the
same placeholders and the plural forms its language needs; `tests/i18n_keys.py` checks that every
key the code uses exists and that none is left unused. Adding a language means a new JSON file and
an entry in `src/i18n.rs`.

## Usage

The app opens on a **service picker** laid out like a keypad (1 2 3 / 4 5 6 / 7 8 9): press a
number key or click a key. To come back, use the house button in the toolbar, **Alt+Home**, or
the tray menu → *Main menu*. The first time, a short **welcome** (four steps, skippable) shows
how to pick a service, move and resize the window, where the controls are and what Watch
together does; *Settings → General → Welcome tour* shows it again.

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
| 9 | **My music** (your library, see [My music](#my-music)) | No |

The picker also opens **any other site** (e.g. Plex or Jellyfin) from the address field, and has
buttons for **Watch together**, the **language** and **Settings**. A small tab on the right edge
marks where the player's controls appear. It can skip itself at startup (*Settings → General →
Open the last service on start*) to go straight to the last service used.

**DRM:** Netflix, Crunchyroll and the other paid services need DRM (Widevine or PlayReady). The
picker checks what this computer's WebView2 runtime offers and says so at the bottom. On a fresh
install the runtime may take a few minutes to download its DRM module; use "Check again".
Expect the same quality limits as Edge or Chrome on the same PC (often 720p–1080p on the web).

| Action | How |
| --- | --- |
| Move | Drag the video (any service) · Alt + drag anywhere · the ⠿ grip in the toolbar |
| Resize | Drag an edge (6 px) or a corner (12 px) |
| Toolbar | Move the mouse to the right edge |
| Main menu | House button, Alt+Home, or tray → Main menu |
| More options | The ⋯ button in the toolbar |
| Settings | The gear (start screen or toolbar), **Ctrl+,**, or tray → Settings → Open Settings… |
| Hide | The eye button, Ctrl+H, or Ctrl+Alt+Y from anywhere |
| Close | The × button, Ctrl+Q, Alt+F4, or tray → Exit |

A plain click on the video still plays and pauses as usual.

**Side toolbar:** move, main menu, back, full screen in the window, fullscreen, always on top,
hide, settings, more options and close. Pointing at a button names it next to the bar, with its
shortcut; reaching the bar with Tab shows it. In short windows (under 300 px) only the
essentials remain; everything else is still in the ⋯ menu.

**⋯ menu:** the everyday things: always on top, full screen in the window, size presets (S/M/L), snap to a
screen corner, opacity, ghost mode, main menu, open a copied link, Watch together, *Settings…*,
a summary of the extensions (which opens them in Settings), language, keyboard shortcuts, hide
and quit.

**Settings** (its own window, above the player, usable even when the player is small or in
ghost mode): **General** (language, start with Windows, open the last service on start, the
welcome tour), **Window** (always on top, opacity, ghost mode, size, corner, full screen in the
window on YouTube, whether a video's own full-screen button fills the window, the
music bar), **Watch together** (your name, share or sync when you host), **Privacy and
security** (app lock off / Windows Hello / PIN, lock now, sign out of every service, how you're
protected), **Extensions** (each one with its switch and options, the folder, reload) and
**Advanced** (hardware acceleration with *Restart now*, the global shortcuts and which ones
another app took, the folder with the session diary, version and WebView2 version). Changes
apply at once; Esc closes it.

**Full screen in the window:** the video fills the player window, not the screen, and the rest
of the page is hidden, so a small window shows only the video. The side bar's button or Ctrl+M
switch it on any site: the player is found on the page (the video with its controls) or in a
frame (Crunchyroll and many others embed theirs that way). Esc, the button or Ctrl+M again end
it, and it ends by itself when the player goes away. The window keeps its size and the side bar
stays; the window's edges still resize it. On YouTube it is the *video only* setting: every
watch page opens that way until it is switched off. With **Settings → Window → "A video's
full-screen button fills the window"**, the player's own full-screen button (or a double click)
does the same instead of filling the screen; F11 and the side bar's full-screen button still fill
the screen. While YouTube is in a real full screen, video only steps aside.

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

**Minecraft (Java Edition):** Minecraft 26.1 added an *Exclusive Fullscreen* video option
(off by default). Off, F11 is borderless and the player stays above it; on, the game takes the
monitor for itself and nothing can be shown over it, not even with maximum overlay: turn it off.
Before 26.1, F11 asks GLFW for a real full screen, which can be exclusive on some graphics
drivers: play in a maximized window or with a borderless full-screen mod (for example *Borderless
Mining Revived* or *Fullscreen Borderless* for Fabric). A *Fullscreen Resolution* other than
*Current* always switches the display mode, which is exclusive too.

**System tray:** left-click hides the player, or opens the music bar when it's already hidden;
double-click shows it. Right-click opens a dark menu with what's playing, play/pause, previous,
next, *Show the controls*, show/hide, main menu, a *Settings* submenu (*Open Settings…*, always
on top, ghost mode, reset opacity, music bar in the taskbar, launch at startup, hardware
acceleration, extensions folder), a
*Language* submenu, *Security* and *Exit*. The icon's tooltip shows the current title. While
hidden, audio keeps playing and rendering and memory use are reduced.

### Music bar in the taskbar

A slim media bar that sits **inside the Windows taskbar**, just left of the notification area:
thumbnail, title and artist, previous / play-pause / next, an animated audio visualizer and a
thin turquoise progress line along the bottom edge.

- **Music services** (YouTube Music today; Spotify, SoundCloud, Apple Music, Deezer, TIDAL and
  Amazon Music when opened through *Another site*): the bar appears on its own while the service is
  open, whether the player window is visible or hidden.
- **Any other service:** click the tray icon while the player is hidden (or tray menu →
  *Show the controls*), or use a media shortcut while hidden. The bar shows for a few
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
- Tray menu → *Settings* → *Music bar in the taskbar* turns off the automatic bar
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
installed under **Program Files**. `Install-overlay.cmd` sets that up, asking for admin
rights once:

1. Installs `yt-mini-player-uia.exe` (same app, manifest with `uiAccess="true"`) to
   `C:\Program Files\YT Mini Player`.
2. Creates a local code-signing certificate, *YT Mini Player (local signing)*, trusts it on this
   computer, signs the executable, and **deletes the private key** so the certificate can't sign
   anything else.
3. Removes the normal per-user install so there's only one copy.

With "always on top" on, the player then stays above fullscreen apps, borderless games and most
"fullscreen" games (those using Windows' fullscreen optimizations), and also above the Start
menu and Task Manager, like the On-Screen Keyboard. `Uninstall.cmd` removes everything,
certificate included (also the *YT Mini Player (firma local)* one versions up to 0.7 created);
running `Install.cmd` switches back to the normal install. The ⋯ menu shows "Maximum overlay is
on" when it's in effect.

Limits: true **exclusive** fullscreen (some DirectX 9, OpenGL or Vulkan games, games with
their own resolution, Minecraft's *Exclusive Fullscreen*, or games with fullscreen optimizations
turned off) can't be covered by any window. Only overlays injected into the game's renderer
(Steam, Discord) can do that, and this app deliberately doesn't inject code: anti-cheat systems
treat it as cheating. Use borderless or windowed fullscreen in those games. The player notices
it (Windows reports Direct3D exclusive mode, the game changed the display mode, or it minimized
when covered), stops trying, and when the game leaves the front it says once what to change
(for Minecraft, which setting). The session diary's `guard:` lines show each step: the band the
player's window is in (2 = the UIAccess layer of this install), the game in front, the display
mode, what Windows reports and how many times the player was raised.

### My music

Key **9** on the picker (or the *My music* row) opens a music player for **your own files**,
inside the same window and with the same music bar in the taskbar, tray controls and global
shortcuts as any music service.

- **Library:** *Folder* adds a folder (MP3, FLAC, M4A/AAC, Ogg, Opus, WAV). Files are read in
  place, never copied: tags, duration, embedded cover art. Folders are re-checked when the page
  opens (new and deleted files) and with ⟳. Search ignores accents and case; sort by recent,
  title, artist or album; play all or shuffle. Each song's ⋯ menu: play next, add to a playlist,
  show in folder, remove from the library, or send to the Recycle Bin.
- **Player:** previous/next, seek, shuffle, repeat (all / one), volume, Space to play/pause,
  Media Session (so the taskbar bar, tray and media keys show and control it).
- **Duplicates:** finds the same song three ways and suggests keeping the best
  copy (lossless, then bitrate, then size): **same file** (identical content), **same song**
  (artist and title match once normalized: accents, "feat.", "(Remastered)", track numbers… and
  the length is within 3 s) and **same audio** (an acoustic fingerprint, Chromaprint, the
  algorithm behind AcoustID, matches even when the format or the tags differ). Nothing is deleted
  without confirming, and only to the Recycle Bin. "They're not the same" hides a group for good.
- **Download:** search the [Internet Archive](https://archive.org) for music under
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
  | Free | free | Library, player, folders, duplicates, searching and previewing free music | 2 |
  | Downloads | 2 USD / month | + downloading songs and whole albums | 5 |
  | Lifetime | 10 USD once | everything, for good | unlimited |

  *Plan* opens the checkout in your browser (Lemon Squeezy); paste the license key from the
  receipt e-mail into *Activate*. *Deactivate on this PC* frees the activation for another PC.

  **Non-invasive by design:**
  - No ads, no pop-ups, no "upgrade" dialogs. Limits are counts (2 / 5 / unlimited playlists),
    shown where they apply: "2 of 2 playlists" next to *New playlist*, and a one-line note
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

### Watch together

Like Discord's screen sharing, inside the player: the **host** shares a code, up to **five
guests** type it, and there are two ways to watch, which the host can switch at any time:

- **Share my window** (the default): everyone sees and hears exactly the host's window. Nobody
  else needs an account, and if the host has YouTube Premium nobody sees ads. The picture comes
  from the host's page as it is drawn (up to 1280×720, 30 frames a second) and the sound from
  the app itself (Windows' process loopback: only YT Mini Player's sound, never Discord, a game
  or a notification). If that sound can't be captured, or nothing is heard for 15 s while the
  page plays, the host's window says so and offers **Share all of my PC's sound** for that
  session, saying that calls and notifications go too; it goes back to the app's sound when
  sharing stops. It goes **straight from the host's PC to each guest** over WebRTC
  (H.264 when the graphics card can encode it, VP8 otherwise; Opus stereo), encrypted. Each
  guest's player shows it in place of its own page and goes back to that page afterwards.
  Protected video (Netflix, Disney+…) looks black to the guests: the window says so, and that is
  what the other mode is for.
- **Sync players**: each person plays the video in their own app, with their own account, and
  the players follow the host's (the same video, the same moment, play, pause, seek and speed).
  Works with Netflix, Crunchyroll and the other DRM services (each guest needs their own
  subscription there).

Open it from the start screen (*Watch together*, next to *My music*), the side bar's ⋯ menu or
the tray.

- **Host:** *Start* gives a code such as `K7Q4-9XM2-PF3T`. It changes **by itself every 30
  minutes** (the previous one still lets people in for 5 more minutes) and *New code now*
  changes it at once. The window lists who is in and how each one's connection is going
  (*watching*, *connecting…*, *can't connect*), and can remove someone (the code changes and the
  others get a new key, so they can't come back with what they knew).
- **Who may use the window.** Next to each guest, a pointer button: on, that person can use the
  host's window with their mouse and keyboard (sharing) or pause and seek for everyone
  (syncing). Off by default. A guest can *Ask to use it*; the host sees the request and answers
  *Allow* or *Not now*. While sharing, only one guest uses the window at a time (the others are
  told who is), and the host's app filters everything that arrives:
  - only the left button, the wheel and keys; no right or middle click, no Ctrl, Alt or Windows
    key combinations (AltGr characters such as @ are fine), no F keys, no browser or media keys
    — those stay on the guest's own PC;
  - never on the app's side bar, the window's edges or the app's menu while it is open, and
    a guest's click can never move the window, resize it or open the host's browser;
  - never on sign-in, account, settings or payment pages, nor on My music (the host is told);
  - whatever a guest is holding down is let go when its control is taken back or it leaves.
- **Guests:** type the code (any case, with or without dashes; O is read as 0 and I or L as 1;
  a typo is caught before connecting). Sharing: the host's window appears with its name and a
  *Live* mark, the sound on (one click if Windows asks for it), volume, full screen, *Ask to use
  it*, the people and *Leave*. If the connection fails it tries again by itself. Syncing: the
  player opens what the host is watching and keeps up with it: it seeks when it is more than
  1.5 s away (0.75 s paused), follows play, pause and the speed, waits while the host has a
  YouTube ad and catches up after its own; a guest's own pause or seek is sent to the host if it
  may, or undone if not. Live streams only follow play and pause. *Resync* tries again.
- **What a guest's player may open (syncing):** only https addresses of the catalog services
  (YouTube, YouTube Music, Netflix, Crunchyroll, Prime Video, Disney+, HBO Max, Twitch), without
  query strings (except YouTube's video and playlist) and never account, settings or sign-out
  pages. A host on another site shows up as "*Open example.com*" for the guest to click; a host
  on the start screen or in My music shares nothing (My music is their own files).
- **Upload.** The host sends one copy of the window to each guest: up to 4 Mbit/s with one
  guest, 3 each with two, less each as more join (1.2 Mbit/s each with five; never more than
  about 6 Mbit/s in all). Each connection starts at a good quality (about 1.5 Mbit/s, not
  WebRTC's usual few hundred kbit/s) and settles at what the path allows. Pictures go out at
  30 a second on average, keeping the ones that arrive a little early.
- **When something is off,** both apps write what they see every 10 seconds in the session
  diary (*Settings → Advanced → Session diary*): pictures per second captured and sent, the
  sound captured (and whether it was silent), what each guest is sent and receives (frames per
  second, size, codec, what limits it, round trip, sound level, whether the sound waits for a
  click). No names, codes or addresses.
- **Connections.** The apps find each other with public STUN servers; behind strict networks
  (some mobile and campus networks) a TURN relay is needed, which the relay server hands out if
  it has a Cloudflare TURN key (free up to 1,000 GB a month, see
  [`server/together-relay/README.md`](server/together-relay/README.md)). A direct connection
  shows each side the other's public IP address, as in any video call.
- **Sound**: the app's own sound needs Windows 11 (or Windows 10 build 20348 or later). On
  older versions the host can share all of the PC's sound instead (the window offers it).

**The code, mathematically.** The host's app keeps a random 256-bit secret *S* for the session.
For the 30-minute window *w* = ⌊Unix time ÷ 1800⌋:

```
T    = HMAC-SHA256(S, "ytmp-together/code/v1" ‖ w as 8 big-endian bytes)
code = first 55 bits of T in Crockford Base32 (11 characters) + a Luhn mod 32 check character
```

55 bits are 36 quadrillion possible codes, and without *S* (which never leaves the host's PC)
the next code can't be worked out from the current one. The check character catches any single
mistyped character and almost every swap of two neighbours. Crockford's alphabet leaves out I,
L, O and U, so nothing can be misread.

**What the server sees.** A small relay (Cloudflare Workers + Durable Objects, in
`server/together-relay/`) passes messages between the host and the guests. It never sees the
code: guests look the room up by a value derived from it with **Argon2id** (19 MiB, 2 passes)
and HKDF-SHA256, which expires with the code. Argon2id makes every guess cost memory and time,
so whoever runs the relay cannot try all the codes while one lasts. Everything else is
**end-to-end encrypted**: a guest proves it knows the code with a first message sealed with a
key derived the same way; the host answers with an
X25519 key exchange and the room key (AES-256-GCM), sealed with a key that mixes the
Diffie–Hellman secret and the code, so recording the traffic and learning the code later
doesn't open it. Room messages are bound to their sender and numbered (replays are dropped);
removing someone changes the room key. The relay keeps nothing once a room ends and logs
nothing.

**The server address.** Watch together needs the relay deployed once (free on Cloudflare; see
[`server/together-relay/README.md`](server/together-relay/README.md)). Release builds made with
`YTMP_TOGETHER_RELAY` set have it built in; otherwise the window asks for it the first time
(`wss://…`). On the free plan a room costs about one hour of Durable Object time per hour of
watching: roughly 28 hours of sessions a day.

### Keyboard shortcuts

In the window (press F1 to see them on screen):

| Shortcut | Action |
| --- | --- |
| 1–9 | Open a service or My music (on the main menu) |
| Space | Play / pause (in My music) |
| Alt+Home | Main menu |
| F11 | Fullscreen |
| Ctrl+T | Always on top |
| Ctrl+M | Full screen in the window (video only on YouTube) |
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

Ctrl+Alt+letter is AltGr+letter on many layouts. The app checks every installed keyboard layout
and, where a key types a character, uses an alternative instead (Ctrl+Alt+. for *next* and
Ctrl+Alt+K for *lock* on Polish keyboards); the help (F1), the tray menu and the music bar show
the keys actually in use. Some older Intel graphics drivers use Ctrl+Alt+arrows to rotate the
screen; in that case Windows won't let the app register them. If another program already uses a
shortcut, the help screen (F1) shows it struck through, with the reason.
Each site's own shortcuts keep working.

### Chrome extensions

Open ⋯ → Extensions → **Open folder** (`%LOCALAPPDATA%\yt_mini_player\extensions`) and drop in:

- unpacked extension folders (with `manifest.json`, also when nested in a single wrapper folder),
- `.crx` files (versions 2 and 3) or `.zip` files. They're extracted automatically and the
  original is moved to `.archives\` (0.7's `.archivos\` folder is renamed). A newer archive with
  the same name updates the extension. Extension names given in several languages (`_locales`)
  are shown in the app's language.

Click **Reload**, then press F5. Each extension can be enabled or disabled from the menu, and
its options page opened if it has one. Deleting an extension's folder uninstalls it on the next
reload.

**Approval:** any program running as you can write to that folder, and an extension sees the
pages (and the signed-in sessions) it runs on. So each extension gets a **SHA-256 fingerprint**
of its whole content and only loads after you approve it in a Windows dialog that lists its
permissions and warns if it can read every site. If a single byte changes, it asks again, and
an extension changed while the app was closed waits aside while WebView2 starts (WebView2
would otherwise load it before the app could ask).
Rejected ones stay off until you press **Reload**. Archives are capped at 10,000 files and
256 MB unpacked (zip-bomb protection), and so are extension folders: sizes are checked before
anything is read.

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
  bound to its purpose, so a modified or swapped file is rejected. Each file reaches the disk
  before it replaces the old one, so a power cut can't leave it half written or empty. The data
  key is wiped from memory after use (`zeroize`) and is only stored wrapped twice:
  - by the computer's **security chip (TPM 2.0)**, through Windows' *Microsoft Platform Crypto
    Provider*: a non-exportable RSA-2048 key generated inside the TPM (OAEP padding), the way
    Windows Hello protects its keys. A copy of the disk can't be opened on another computer, even
    knowing the Windows password;
  - and by **DPAPI**, tied to your Windows account (with app-specific entropy).

  No prompts or dialogs. The TPM step runs in the background the first time (generating the key
  inside the chip can take a few seconds) and only replaces `vault.key` after checking that the
  TPM gives the same key back. Without a usable TPM, the key stays protected by DPAPI. Plain
  `*.json` files are never read (anyone who can write to the folder could otherwise turn the
  lock off, approve an extension or unlock a plan).
- **The sign-ins belong to the settings.** A random marker sits both in the encrypted settings
  and in the browser profile. If the settings are deleted, damaged or from another key, or the
  profile is not theirs (swapped, or older settings put back), every saved sign-in is wiped
  before any page opens: deleting a file does not get anyone past the lock. The marker changes
  each time the lock is turned on.
- **No EFS.** Version 0.6.0 encrypted the data folder with EFS, which makes Windows ask the first
  time to *back up the file encryption certificate and key*: a scary dialog for something that
  wasn't needed. Since 0.6.1 the app doesn't use EFS and decrypts a folder 0.6.0 had encrypted (no
  prompt). For whole-disk encryption, including the browser cache, use BitLocker or Windows Device
  Encryption.
- **Sign out everywhere:** tray → Security → *Sign out of all services…* clears
  cookies, site storage, cache and history for every service (after a Windows confirmation).
- **Protection status:** tray → Security → *How you're protected…* summarizes all of the
  above for this machine.

### App lock

Tray → Security:

- **Lock with Windows Hello**: fingerprint, face or Windows PIN. Windows does the check
  (`UserConsentVerifier`) with its own retry limits; the app never sees biometric data.
- **Lock with a PIN…**: at least 6 characters, no repeats (111111) or runs (123456). Only
  an **Argon2id** hash is kept (64 MiB, 3 passes, random salt; OWASP's recommended password KDF),
  inside the encrypted vault. After 5 wrong attempts each further one doubles the wait (30 s,
  1 min, … up to 15 min), and the counter survives restarts. Changing the Windows clock neither
  shortens the wait nor stretches it past 15 minutes. **Forgot it?** The lock screen can
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
  anything else opens in your default browser, only when you clicked for it (WebView2 has no
  pop-up blocker, so a site could otherwise open tabs there by itself) and at most one every
  2 s. Only plain `http(s)`
  addresses are ever handed to the browser, and the app's folder buttons only open folders:
  Windows is never asked to run a file or a custom protocol.
- **Downloads are blocked.** **Permissions are denied** (camera, microphone, location,
  notifications, clipboard reading, file system, screen capture…); autoplay is allowed.
- **SmartScreen is on** (wry turns it off by default; with "Another site" any site can be opened).
  Tracking prevention is set to *balanced*.
- **The music bar shows the domain** of pages outside the catalog, so a site can't pass itself
  off as something else down there. Titles lose control and text-direction characters, so they
  can't be made to read backwards either.
- Every page of the app (start screen, music bar, lock screen, Settings, Watch together, the
  shared window) is served with a strict Content-Security-Policy that runs only the page's own
  script, by its SHA-256 hash (never `unsafe-inline`: no other inline script can run, not even
  one an extension slips in), plus `no-referrer` and `X-Content-Type-Options: nosniff`; the side
  windows can't navigate away from their page.
- **My music:** renders tags and titles as text only, and can't be framed. Its commands
  (`music:…`) are a separate closed list accepted **only from that page's URL**, with sizes and
  ids validated; the app pushes data to it only after checking, inside the page, that it is that
  page. Songs and covers are served by id (never by a path from the URL) and require a random
  per-session key, so another site open in the player can't read your files; cross-site requests
  are refused, and a request without the key gets no thread. Songs and covers are read off the
  window's thread. Library, fingerprints and license are stored in the encrypted vault.
  Downloaded files get names made safe for Windows (no reserved names, separators or drive
  letters, also in the Archive's own file names), so they always land inside
  `Music\YT Mini Player`.
- **Network (My music):** HTTPS only, never plain http (not even after a redirect), through
  Windows' own TLS stack (SChannel, TLS 1.2+, the system's certificates; verification is never
  turned off), a descriptive User-Agent, few parallel
  downloads and back-off with `Retry-After`, as the Internet Archive asks. Lemon Squeezy keys
  are checked against **this store and these products** (the license API accepts keys from any
  store).
- **Maximum overlay** (UIAccess can send input to administrator windows): that process only
  opens the catalog services and their sign-in pages; no *Another site*, no external links.

### Watch together

- The relay only passes messages; what they carry (the video's address, the position, names,
  the room key, the WebRTC offers and answers) is end-to-end encrypted (X25519, HKDF-SHA256,
  AES-256-GCM; see [Watch together](#watch-together)). It never sees the code, only a hash that
  expires with it.
- A shared window travels directly between the apps, encrypted by WebRTC (DTLS-SRTP). The
  certificates' fingerprints arrive inside the sealed offer and answer, so the relay can't step
  in between. Through TURN, Cloudflare passes encrypted packets it can't read.
- The page that sends the window is hidden, pinned to its address, with its own closed protocol
  (`cast:…`) and no permissions. The page that shows it to a guest only takes `stream:…`
  messages from that one address (like My music's). Offers and answers are checked (printable
  SDP, bounded size), and ICE servers too (only `stun:`/`turn:`/`turns:`, bounded).
- Remote control is filtered in the host's app (see above): the guest's page can send anything,
  and only what makes sense on a web page, where the host allowed it, gets through. Keys are
  checked by their code and by their name, so an F key can't come in disguised as another one.
- The app connects with Windows' TLS (SChannel, TLS 1.2+); `ws://` is only accepted for a relay
  on this computer.
- The window is another pinned page with its own closed protocol (`together:…`) and the app's
  strict CSP. Everything that arrives from the others is checked: sizes, numbers, names (no
  control or direction characters), member ids, and the addresses a guest may follow.
- A guest's player is only sent to https catalog addresses without query strings and never to
  account or sign-out pages; it can be sent to another site only by clicking *Open*.
- Locking the app hides the window; a locked guest's player is not controlled and its view of
  the host's window closes; a locked host stops sending (the guests see "paused") and ignores
  guests' requests and input (nothing plays behind the lock screen).

### Supply chain

- `.github/workflows/ci.yml`: tests, clippy, JS and browser tests on Windows, and
  `cargo-deny` (RustSec advisories, licenses, crates.io only) on every push.
- `.github/workflows/release.yml`: builds the release on GitHub, runs `cargo audit`, publishes
  `SHA256SUMS.txt` and a signed **build-provenance attestation** (Sigstore) for the zip and the
  executables: `gh attestation verify YT-Mini-Player-<version>-win64.zip --repo <owner>/<repo>`.
  It refuses a tag that doesn't match `Cargo.toml`'s version (`v0.13.3` for `0.13.3`) before
  building anything, and takes the release notes from `docs/releases/<version>.md` when there is
  one (GitHub's generated list otherwise).
- Dependabot opens weekly update PRs for crates and actions.
- The local "maximum overlay" certificate is code-signing only and not a CA, and its private key
  is deleted right after signing.

## Selling the plans (Lemon Squeezy setup)

Payments are handled by [Lemon Squeezy](https://www.lemonsqueezy.com) as merchant of record
(checkout, taxes/VAT, receipts, refunds, the customer portal). The app only uses its public
[License API](https://docs.lemonsqueezy.com/api/license-api) (activate, validate,
deactivate). To connect your store:

1. In Lemon Squeezy, create two products with **license keys enabled**:
   - *Downloads*: subscription, 2 USD / month, activation limit e.g. 3.
   - *Lifetime*: single payment, 10 USD, activation limit e.g. 3, keys that never expire.
   Subscription keys expire when the subscription ends; disabling a key (refund) blocks it.
2. Note the **store id**, each product id (or variant id) and each product's **checkout URL**
   (Share → checkout link), plus your customer portal URL if you want a *Manage subscription*
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

**0.13.3** — security hardening, no more lost vault on a power cut
(release notes: [`docs/releases/0.13.3.md`](docs/releases/0.13.3.md), with everything since 0.7.0)
- **Downloads stay inside `Music\YT Mini Player`:** a title that cleaned down to nothing (or to
  a reserved name such as `CON`) fell back to the Internet Archive's raw file name, and a name
  like `C:\…` could place the file elsewhere. Fallbacks are cleaned too, Archive file names with
  `\` or `:` are ignored, and item ids can't start with a dot.
- **Watch together:** keys a guest sends are checked by name as well as by code, so F11, F1,
  media and browser keys can't come in disguised (the page, and the app's own shortcuts, read
  the name).
- **App lock:** moving the clock forward no longer skips the wait after wrong PINs, and moving it
  back no longer locks the PIN for longer than 15 minutes (a monotonic clock while the app runs).
- Only plain `http(s)` addresses reach the default browser, and folder buttons only open folders;
  My music never uses plain http, not even after a redirect; titles lose text-direction
  characters; extension folders are checked by size before anything is read; media requests
  without the session key get no thread; the relay hashes before it touches a room.
- **Fixed:** encrypted files (and `vault.key`) are flushed to disk before they replace the old
  ones, so a power cut right after saving can't empty them; covers load off the window's thread;
  a language switch no longer re-hashes every extension; big Archive items parse in linear time.
- `npm test` for the relay works on Windows and always stops `wrangler` and `workerd`; clippy is
  clean on Rust 1.98.

**0.13.2** — about 5,800 new tests
- **2,032 generated Rust tests** (`src/tests/`, written by `tests/gen/`): every message the pages
  can send, accepted or refused, and random garbage at every parser; plural forms, grouping and
  every text rendered in five languages; services, links, navigation rules, the Watch together
  code (every single mistyped character), sound conversion at eleven sample rates, the frame
  pacer, the topmost guard's decisions, the diary's statistics, settings files; plus two
  timing tests.
- **3,355 new tests in the experience suite** (`tests/cases/`, now 4,126 in all): each of the
  856 texts checked in five languages; the start screen, welcome, Settings, side bar (shortcuts
  on Russian, Greek, Hebrew, Arabic, Dvorak and AZERTY layouts) and Watch together window in
  every language; twelve hostile payloads in every field that shows outside data; aria
  references, ids, names and roles; drawing times, memory after hundreds of updates, element
  counts and size budgets; more window sizes.
- **About 370 PowerShell tests** (`tests/scripts.tests.ps1`): the installer's messages in five
  languages, its fingerprint check, the release check and `sign.ps1`'s refusals, the process
  tree, the launchers. Nothing is installed or signed.
- Fixed, found by them: Settings opened a broken page when asked for a section called
  `constructor` or `__proto__`; the Japanese installer took 「いいえ」 (no) for a yes.

**0.13.1** — games in exclusive full screen
- The always-on-top guard tells exclusive full screen apart (Windows' Direct3D exclusive state, a
  display mode of the game's own, or a game that minimizes when covered), stops trying and, once
  the game leaves the front, says what to change in it: for Minecraft, *Exclusive Fullscreen*
  off (26.1 and later) or a maximized window or borderless mod before that; for other games,
  borderless or windowed full screen.
- Diary: `guard:` lines with the player window's band (2 = maximum overlay), each full-screen
  window in front, its display mode, what Windows reports, raises and why it gave up.
- Long notices stay on screen longer (about 60 ms a character, up to 15 s).

**0.13.0** — full screen in the window
- **The video fills the player window on any site**, not only YouTube: the side bar's button
  (it was YouTube's *video only*), Ctrl+M or the ⋯ menu. The app finds the player on the page
  or in a frame (Crunchyroll's is in one), hides the rest and keeps the window's size, side bar
  and edges. Esc or the button again end it; it ends by itself when the player goes away.
- **Settings → Window → "A video's full-screen button fills the window"** (off by default):
  the player's own full-screen button or a double click fills the window instead of the screen.
  F11 and the side bar's full-screen button still fill the screen, and with the window filled
  they keep only the player on screen, side bar included.

**0.12.0** — the evaluation plan's risks R1, R2, R4 and R6, mitigated
- **Sound when sharing, with a way out (R1).** If Windows won't give the app its own sound
  (Windows 10 before build 20348), or the capture hears nothing for 15 s while WebView2 says the
  page is playing, the host's window explains it and offers *Share all of my PC's sound* for
  that session (the default output's loopback), saying it also carries calls and notifications;
  *Share only the app's sound* goes back. Outputs that won't convert are converted by the app
  (float or integer, any rate, surround folded to stereo). The diary notes which source is used
  and when sound stops being heard.
- **Signing and antivirus (R2).** `tools/sign.ps1` signs both programs with Authenticode when
  a method is set up (Azure Artifact Signing in `release.yml`, or a certificate in the store for
  local builds), verifies each signature and its timestamp, and runs before the fingerprints are
  written. A signed overlay program installs without the local certificate. If an antivirus
  removes `install.ps1`, each launcher explains what happened and what to do in the Windows
  language (`ANTIVIRUS.txt`). Options, costs and steps: [`docs/SIGNING.md`](docs/SIGNING.md).
- **When a service changes its page (R4).** On YouTube's watch page the app checks that the
  player, its API, the video and the full-screen button are there; it writes the result to the
  diary and, if something is missing, says so once per session. A refused Google sign-in is told
  apart the same way. The player is found by its other names or as the box around the video,
  so video only, full screen and dragging keep working after a rename. A weekly workflow
  (`live-smoke.yml`) runs those checks against the real services and opens an issue when one
  fails.
- **Native regressions caught in CI (R6).** A drivable test build (`--features e2e`: DevTools
  port, own data folder and instance) lets `tests/windows_e2e.py` drive the real app on
  Windows: start screen, Settings changing the window's real always-on-top state, and two copies
  joining through a local relay. The release refuses that build (`check-release-exe.ps1`), and
  CI now also runs the experience suite.

**0.11.1** — window sharing: sound for the guests and a smoother picture
- **Sound for the guests:** the guest's player most likely kept the host's window muted until
  someone clicked it (the engine's autoplay rule: the page opens by itself, not by a click), and
  the "Turn on sound" button was easy to miss. The app now lets its pages play sound at once.
  If the capture itself hears nothing, the diary now says so.
- **Smoother picture:** the hidden page that sends the window ran at background priority, so
  Windows gave it CPU last while the host's video and games ran; it now keeps normal priority.
  Pictures that arrived a few milliseconds "early" were dropped (a 30 fps video on a 60 Hz
  screen lost about one in four); they now go out at 30 a second on average. Each connection
  starts sharp (about 1.5 Mbit/s) instead of climbing from a blurry few hundred kbit/s, and
  one or two guests get more upload (4 and 3 Mbit/s at most; 6 in all, as before).
- **Diary:** every 10 s, both apps write what sharing looks like (see Watch together →
  *When something is off*), so the next problem can be found from `session.log`.

**0.11.0** — friendlier for new users: a Settings window, a new start screen and a welcome
- **Settings window** (gear on the start screen and on the side toolbar, Ctrl+, or the tray):
  six sections with a line under each option saying what it does. It is a window of its own,
  pinned to its page with its own closed message list, so it works while the player is tiny
  or click-through; dialogs it opens (sign out, how you're protected) appear above it. While the
  app is locked it hides and takes no changes.
- **Start screen**: the services as a 3×3 keypad matching the number keys, a line that says how
  to start, Settings and language buttons in the header, Watch together next to the address,
  and a tab on the right edge where the controls appear. Fits 480×270 in every language.
- **Welcome** the first time (and on demand): four short steps with a picture each, keyboard
  friendly, skippable.
- **Side toolbar**: a Settings button; each button's name appears next to it at once (with
  its shortcut) instead of the slow system tooltip; Tab shows the bar. The ⋯ menu is shorter
  (startup, graphics and the extension list moved to Settings) and always opens at its top.
- **Tests**: `tests/suite.py`, 719 automated checks in one file (see [Tests](#tests)). What
  they caught and is now fixed: with reduced motion on, the waves kept moving on the start
  screen, lock screen, Watch together and the shared window (a CSS rule lost to a more specific
  one); the ⋯ menu reopened scrolled to where it was left; the side bar stayed invisible when
  reached with Tab; after the welcome the focus fell on the page instead of the services; the
  section asked for (e.g. "Extensions") was lost when Settings opened for the first time; and
  a few targets were under 24 px (switches, corners, "Check again").

**0.10.2** — fix: the host's app closed when a guest connected to a shared window
- The request for the app's sound (`ActivateAudioInterfaceAsync`) was made from a background
  thread; Windows never answered it there, and after 5 s the app gave up and freed what the
  pending request was still using, which closed the app. The request is now made from the main
  thread, as Microsoft requires, and nothing it uses is ever freed: if Windows still doesn't
  answer, the window is shared without sound (after 10 s) and the app keeps running.
- The sound guard only counts a crash of the same version.

**0.10.1** — finding a crash when sharing
- A diary of what the app does (`session.log` in `%LOCALAPPDATA%\yt_mini_player`, the last
  one kept as `previous-session.log`): sharing and watching steps, the sending page, the
  screencast, the sound capture, connection states — never names, codes, addresses or what is
  watched. A crash adds where it happened (a panic with its file and line, a Windows exception
  with its module and offset, a WebView2 process that died with its reason). After a crash
  while sharing, the app says where the diary is.
- If the app closed while starting the sound capture, the next session shares without sound
  instead of trying again.

**0.10.0** — share your window
- **Watch together shares the host's window**, like Discord: picture and sound go straight to
  each guest (WebRTC), so nobody else needs an account or sees ads. Rooms grow to the host and
  **five** guests. *Sync players* (each one plays it with their own account) stays for Netflix
  and other protected video; the host switches at any time.
- **Guests can use the host's window** with their mouse and keyboard, one at a time, only if
  the host allows it (per person, with *Ask to use it* / *Allow*). Shortcuts, F keys, the app's
  side bar, sign-in and payment pages are out of reach. The same switch lets someone pause and
  seek when syncing (it replaces *Guests can pause and seek*).
- The app's own sound is captured on its own (Windows process loopback), so other programs are
  never shared.
- **Hardware acceleration** setting (side bar's ⋯ menu and the tray's Settings), on by default,
  like the one in Chrome and Edge; the app offers to restart to apply it.
- The relay: rooms of six, ICE servers (STUN, plus Cloudflare TURN with a key), 32 KiB
  messages. **Redeploy it** (`npx wrangler deploy`): 0.10 apps need it, and 0.9 apps can't
  join 0.10 hosts (the host is told who must update).

**0.9.1** — a new look
- Every screen redrawn in one blue palette, from deep twilight (`#03045e`) to light cyan
  (`#caf0f8`): the deep blues are the background, the light ones the text, turquoise what can be
  pressed. A "tide line" with the whole palette under the headers, slow waves along the bottom
  of the start screen, lock screen and Watch together window, service numbers that deepen row by
  row, a visualizer and progress lines that fade from teal to cyan.
- Segoe UI Variable throughout (no more Bahnschrift), rounder controls, fewer boxes.
- A new icon in the same colors.

**0.9.0** — watch together
- **Watch together**: the host shares a code and up to two guests watch in sync, each in their
  own app (so DRM services work with everyone's own account). Play, pause, seek and speed
  follow the host; guests may control playback if the host allows it; YouTube ads hold everyone
  or are caught up after; live streams follow play and pause.
- The code is an HMAC-SHA256 of a session secret and the 30-minute window, in Crockford Base32
  with a Luhn mod 32 check character: it changes by itself every 30 minutes, typos are caught,
  and the host can change it at any time or remove a guest (new code and new room key).
- End-to-end encrypted through a small relay (`server/together-relay/`, Cloudflare Workers +
  Durable Objects, free plan): X25519 handshake bound to the code, AES-256-GCM room key, replay
  protection. The relay never sees the code, reconnects dropped guests (60 s) and hosts (120 s).
- Its own window, in the five languages, from the start screen, the side bar's ⋯ menu and the
  tray.

**0.8.0** — five languages
- The whole app is available in **English, Spanish, Russian, Polish and Japanese**, written for
  each language (plurals, number/size/price/date formats, quotes, Windows terminology, plan
  names, keys as printed on each keyboard). It follows the Windows language on first start.
- Language picker that is easy to find: a globe button on the start screen, *Language* in the tray
  menu (always also in English) and in the side bar's ⋯ menu. Switching is instant: pages update
  in place, the music bar and lock screen reload.
- Texts the app sends to its pages (My music's notices, scan results, download reasons, license
  messages) are now keys with arguments, shown in the page's current language, so they follow a
  language change.
- Global shortcuts check every installed keyboard layout and pick another key when AltGr would
  type a character with it (Polish ń/ł); the help, tray menu and music bar show the real keys.
  Window shortcuts also work on Cyrillic layouts.
- System dialogs use buttons named after the action (TaskDialog) instead of Yes/No.
- Code, comments, file names and docs are in English: `Install.cmd`, `Install-overlay.cmd`,
  `Uninstall.cmd`, `README.txt`; the installer speaks the Windows language and removes the
  overlay certificate of older versions too. The extensions' `.archivos` folder becomes
  `.archives`.

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
- New: plans through Lemon Squeezy (*Downloads* 2 USD/month, *Lifetime* 10 USD once), sold
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
  *Another site*); adding a music service to the catalog is a single `music: true` flag.
- The bar narrows to avoid covering app icons (UI Automation), can be dragged along the taskbar,
  hides with auto-hide or fullscreen apps, and stays above the taskbar when you click it.
- New setting in tray → *Settings*: *Music bar in the taskbar*.

**0.4.1**
- Redesigned quick controls: a slim media bar (440×64) with the thumbnail, title and artist,
  transport buttons, an animated audio visualizer that doubles as a volume meter, and a
  draggable progress line. Lavender window border on Windows 11 (drawn by the page on
  Windows 10). The ±10 s buttons and volume slider are gone from the panel; the shortcuts and the
  mouse wheel cover them.
- The tray menu is now dark and tidier: window options moved into a *Settings* submenu.

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
| `src/i18n.rs` · `src/locales/` | Languages: picking one, plural rules, formatting, page strings, messages for pages |
| `src/app.rs` | App state and execution of every command |
| `src/services.rs` | Service catalog, URL matching, user-typed addresses |
| `src/home.rs` · `src/home.html` | Service picker (keypad) and first-run welcome, served at `https://ytmp.localhost/` |
| `src/ipc.rs` | Page → host protocol (closed list) and internal commands |
| `src/window.rs` | Borderless window, icon, restored geometry |
| `src/webview.rs` | WebView2 with isolated profile, injected scripts, per-service user agent |
| `src/extensions.rs` | `.crx`/`.zip` handling, manifests, sync with the WebView2 profile |
| `src/tray.rs` | Tray icon and menu |
| `src/hotkeys.rs` | Global shortcuts, checked against every installed keyboard layout |
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
| `src/together/` | Watch together: the code (`code.rs`), the encryption (`crypto.rs`), what travels (`protocol.rs`), keeping a guest in sync (`sync.rs`), host and guest logic (`session.rs`), the WebSocket to the relay (`link.rs`), the glue with the app (`mod.rs`) |
| `src/together_panel.rs` · `src/together.html` | The *Watch together* window |
| `src/settings_panel.rs` · `src/settings.html` | The *Settings* window (its own page and closed message list) |
| `src/cast.rs` · `src/cast.html` | Sharing the host's window: the hidden page that turns its pictures and sound into WebRTC, one connection per guest |
| `src/devtools.rs` | The main page's DevTools protocol through WebView2 (screencast, input, page messages) |
| `src/loopback.rs` | The window's sound: the app's own (WASAPI process loopback) or, if the host asks, the PC's; silence check and format conversion |
| `src/e2e.rs` | The drivable test build for the Windows end-to-end tests (`--features e2e`, never released) |
| `src/remote.rs` | A guest using the host's window: what gets through, and where |
| `src/stream.html` | The host's window in a guest's player |
| `server/together-relay/` | The relay: a Cloudflare Worker with Durable Objects, and its tests |
| `build.rs` | Icon, version info and manifest (normal or UIAccess) |
| `dist/` | Installer (`install.ps1`, `Install.cmd`, `Install-overlay.cmd`, `Uninstall.cmd`, `README.txt`, `ANTIVIRUS.txt`) |
| `tools/` | `measure.ps1` (memory and CPU), `package.ps1` (packaging), `sign.ps1` (Authenticode), `check-release-exe.ps1` (refuses the test build) |
| `docs/SIGNING.md` | Signing the releases: certificates that fit, CI and local steps, antivirus false positives |
| `docs/releases/` | Release notes, one file per version: `release.yml` publishes `<version>.md` as the release's notes |
| `tests/` | Browser suites (`suite.py` and its `cases/`), script tests, the generators of `src/tests/` (`gen/`) |
| `src/tests/` | Generated table-driven Rust tests, one file per module (`#[path]` from that module) |

User data lives in `%LOCALAPPDATA%\yt_mini_player`: `profile\`
(sessions for every service), `extensions\`, `vault.key`, the encrypted `*.enc` files and
`music\` (encrypted library, fingerprints and license, plus a cover cache). Downloaded songs go
to `Music\YT Mini Player`; your own folders are only read.

## Tests

```powershell
cargo test                               # protocol, services, start page, settings, topmost, … (2,000+ generated cases)
python tests/gen/rust_cases.py           # writes src/tests/*.rs again after a change to a generator
powershell -ExecutionPolicy Bypass -File tests\scripts.tests.ps1   # installer, release check, signing, launchers
node tests/inject.test.js                # drag and edges, simulated DOM
node tests/maintenance.test.js           # preventive reload, simulated clock
python tests/ui_e2e.py screenshots       # YouTube UI in Chromium (pip install playwright)
python tests/home_e2e.py screenshots     # service picker and a non-YouTube player
python tests/mini_e2e.py screenshots     # music bar + media.js on a real <audio>
python tests/security_e2e.py screenshots # hostile page vs. the app's scripts, lock screen
python tests/music_e2e.py screenshots    # My music page with the app's real wrapper and CSP
node tests/media.test.js                 # media.js with simulated Media Session and players
powershell -ExecutionPolicy Bypass -File tests\measure.test.ps1
python tests/i18n_keys.py                # every text key used exists; none left unused
python tests/together_e2e.py screenshots # the Watch together window, in five languages
python tests/share_e2e.py screenshots    # window sharing: real WebRTC between cast.html and stream.html
python tests/suite.py --report tests/report   # the experience suite: 4,100+ tests, report.md + report.json
python tests/live_smoke.py --offline     # the weekly check against the real services, on stand-ins
cargo build --release --features e2e; python tests/windows_e2e.py --relay ws://127.0.0.1:8799   # the real app on Windows
cd server/together-relay; npm install; npm test   # the relay, on a local wrangler dev (Windows too)
```

`suite.py` is the experience suite: 4,126 tests in eight groups, run with one command and
summed up in `report.md` (`--category`, `-k` and `--list` pick some). The first 771 live in
`suite.py`; the rest in `tests/cases/`, one module per kind, which registers its tests in the
same list: `catalog` (each of the 856 texts in five languages: same shape, placeholders and key
caps as English, the plural forms each language needs, balanced quotes and brackets in its own
style), `home_more`, `settings_more`, `sidebar_more` and `together_more` (every key, control,
shortcut, menu row and screen in every language and from every state), `security_more` (twelve
payloads that try to become markup or run code, in every field that shows outside data, and
forged calls with near-miss keys), `a11y_more`, `perf_more` (drawing time, cost of hundreds of
updates, JS heap after them, element counts, string and file budgets: generous limits, so a
failure means several times slower), `layout_more` and `contract_more` (every key the code asks
for exists). The original groups: **unit**: plural forms per language, placeholders,
key caps in the welcome, the Settings state against `App::push_settings`. **contract**: every
message the Settings page and the start screen can send is one `ipc.rs` accepts, each page only
uses strings it is served, Settings hides while locked and reloads with the language. **e2e**:
every key and number on the start screen, the welcome step by step (mouse and keyboard),
every switch, slider, radio group and button in Settings (with the state the app sends back),
the side toolbar's gear, labels, Ctrl+, (also on a Russian layout) and menu. **security**: no
HTML injection or `eval` in any page or script, nothing loaded from the Internet, zero CSP
violations in five languages, hostile names shown as text, forged calls ignored, a hostile
site cannot open Settings. **a11y**: every control named, roles and tab stops, visible focus,
keyboard-only welcome, 24 px targets, WCAG contrast of the palette, reduced motion.
**performance**: size budgets, render time, DOM size, no request leaves the app, no idle
timers, throttled slider and debounced name. **layout**: the start screen, the welcome and
every Settings section in five languages at the window's real sizes, with nothing cut or
scrolling sideways. **i18n**: every new text in five languages with the same placeholders.

The generated Rust tests (`src/tests/*.rs`, 2,032) are tables written by `tests/gen/rust_cases.py`
and `rust_cases2.py`: one `#[test]` per case, so a failure names the case. They cover what goes
in from the pages (`ipc`: every message accepted, every malformed one refused, and seeded random
garbage for each parser, which must never panic), `i18n` (plural rules against CLDR, number
grouping, every placeholder rendered in every language), `services`, `links`, `policy`, the
Watch together `code` and `protocol`, `media`, `loopback` (the converter at eleven sample rates
and four channel layouts, chunk sizes, the silence watch), `pacer`, `topmost` (which windows
count as covering, exclusive full-screen signs, the advice), `diag`, `e2e` and `config`.
`tests/scripts.tests.ps1` (Windows PowerShell 5.1 or 7) reads `install.ps1` without running it:
the message table (same keys and placeholders in five languages, every message formats, the yes
answers each language takes), the fingerprint check against `SHA256SUMS.txt` in eleven cases,
the autostart command and the certificate test; then it runs `check-release-exe.ps1` and
`sign.ps1` against files in a temporary folder (the test build caught wherever its marker is;
every refusal of `sign.ps1` before anything is signed), `measure.ps1`'s process tree, the
launchers and `ANTIVIRUS.txt` read the way they read it.

Watch together is tested at three levels. The Rust tests check the code (a fixed vector worked
out separately in Python, every single mistyped character caught, look-alike letters), the
encryption, the sync decisions (clock offset, drift, adverts, live, sites that ignore seeking,
the guest's own clicks) and whole sessions through an in-memory relay (a host and two guests,
a forged hello, replays, the code's grace period, removal and rekeying, a guest coming back,
sharing with three guests: connections made and closed as people come and go, upload per
guest, asking for and taking back control, locking, an older app turned away, descriptions that
are not SDP). `remote.rs` has its own (clicks placed on the picture, the side bar and edges out
of reach, shortcuts and F keys kept home, AltGr characters, one guest at a time, the rate
limit, letting go, pages that are off limits).
`server/together-relay/test/run.mjs` runs the relay under `wrangler dev` (seats for five,
tickets, graces, lookups that expire, ICE servers, size and rate limits, eight guests joining at
the same moment). And with that relay
up, `YTMP_TEST_RELAY=ws://127.0.0.1:8799 cargo test against_a_real_relay` runs a host and six
guests' apps against it: wrong code, full room, following the video, a guest's pause reaching
the host, sharing the window (offer and answer sealed through the relay), removal with a new
key, the old code refused, and the end. `share_e2e.py` connects the sending page and a guest's
page with real WebRTC in Chromium: the picture arrives with its colours and full size, a 440 Hz
tone arrives as 440 Hz, clicks land on the right spot of a letterboxed picture, keys travel
but Ctrl shortcuts stay home, control taken back stops the input, a dropped connection is
retried, and every screen fits in five languages at 480×270.

`ui_e2e.py` loads a page that mimics YouTube's layout (fixed header, two columns, nested stacking
contexts) under its real Trusted Types policy and checks 69 behaviors, including tooltips read
from the accessibility tree, the language row, the help in Russian and shortcuts typed on Russian
and Dvorak layouts. `home_e2e.py` serves the picker with the app's real Content-Security-Policy
and checks 77 behaviors: keys 1–9 on the keypad, the address field, the Settings and Watch
together buttons, the DRM check (both
outcomes, with each language's "and"), the language picker (mouse and keyboard, switching in
place), Spanish/Russian/Polish layouts without overflow, small windows, and dragging, clicking
and seeking on a generic streaming player. `mini_e2e.py` checks the music bar at taskbar sizes
(360×40 for Windows 11, 360×32 for Windows 10, 48 px floating, and the narrow 250–280 px
variants): every button, click and drag on the progress line, dragging the bar by its title,
wheel volume meter, hover, right-click close, empty/live/muted states, broken thumbnail and
layout, the real shortcuts in tooltips and the bar in Russian, Japanese and Polish; then it runs
media.js against Chromium's real Media Session with a real audio element (88 checks in total). `security_e2e.py` loads a hostile page that writes to `window.ipc`,
hooks WebView2's message function, fires synthetic keys and clicks, fakes toasts and searches for
the token, and checks that none of it gets through; then it drives the lock screen (Hello, PIN,
waits, new PIN, small windows, in four languages; 55 checks). The browser tests inject exactly the wrapper `webview.rs` builds
(`tests/bundle.py` reads it from the source). `music_e2e.py` serves the My music page with its
hash-based CSP and drives it as the app would: library, filtering with accents, playback through
`/media` with the session key, Media Session and media.js reports (taskbar bar), next/pause from
the app, track menus, Recycle Bin confirmation, upsells, plan cards, license activation,
Internet Archive search, album view, previews, album download, the download queue, duplicates,
plan limits shown inline (never a dialog), playlists, a forged message without the key, switching
to Russian, Polish and Japanese (plurals, grouped numbers, units, prices, notices rewritten in
place), and 480×270 / 256×144 windows (107 checks).

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
- [ ] The music bar has a blue border on Windows 11 and the tray menu is dark.
- [ ] Google sign-in still works on YouTube after visiting another service.
- [ ] Dragging the video moves the window on YouTube and on Netflix without pausing.
- [ ] Hide with the eye button and bring it back with Ctrl+Alt+Y or the tray; audio keeps playing while hidden.
- [ ] Fullscreen (F11 or the site's button) fills the monitor; Esc restores the previous size.
- [ ] Crunchyroll episode: Ctrl+M fills the window with the player (header and episode list
      hidden), the edges still resize, Esc goes back; with the Settings switch on, the player's
      own full-screen button does the same.
- [ ] With "always on top", the player stays above a borderless game after clicking into the game.
- [ ] Maximum overlay: the installer finishes, the ⋯ menu shows it active, and the player stays above a fullscreen app.
- [ ] Maximum overlay: launch at startup starts hidden in the tray; `Uninstall.cmd` removes the certificate.
- [ ] 70% opacity looks translucent (not black); ghost mode lets clicks through.
- [ ] A `.crx` extension shows up in the menu and works after F5, after approving it in the dialog.
- [ ] Windows Hello lock: enabling asks for Hello; Win+L then coming back shows the lock screen; Hello unlocks.
- [ ] PIN lock: wrong PIN 5 times starts the wait; the right PIN unlocks.
- [ ] Tray → Security → How you're protected says the key is protected by the TPM (on a PC with TPM 2.0, after the first launch).
- [ ] Upgrading from 0.6.0 on Windows Pro: no EFS dialog; `cipher /u /n` no longer lists files under `yt_mini_player`; sessions are kept.
- [ ] Sign out everywhere: after it, YouTube and Netflix ask to sign in again.
- [ ] Upgrading from 0.5.x keeps the settings (the old `.json` files disappear, `.enc` appear).
- [ ] Minecraft (F11 fullscreen): the player stays on top; clicking pause on it doesn't minimize the game.
- [ ] A borderless game that re-asserts topmost: the player doesn't flicker.
- [ ] My music: add a folder with a few thousand songs; scanning finishes and the list scrolls smoothly.
- [ ] My music: songs play, seeking works on FLAC and MP3, and the taskbar bar shows the cover.
- [ ] Duplicates: a song in MP3 and FLAC is found as "Same audio"; the Recycle Bin gets the other copy.
- [ ] Download (with a test store or `YTMP_DEV_UNLOCK=1`): an album downloads as a playlist; downloading it again downloads nothing.
- [ ] Lemon Squeezy test mode: buy, activate, restart offline, deactivate.
- [ ] Free plan: a third playlist can't be created (inline note, no dialog); after cancelling a test subscription with 5 playlists, all 5 remain and can be edited.
- [ ] Languages: on a Japanese, Russian or Polish Windows the first start is in that language; the globe button, the tray's *Language* and the ⋯ menu switch it; fonts look right (Yu Gothic UI, Bahnschrift with Cyrillic).
- [ ] Polish keyboard layout: AltGr+N and AltGr+L type ń and ł in other apps while the player runs; F1 shows Ctrl+Alt+. and Ctrl+Alt+K.
- [ ] Russian keyboard layout: Ctrl+T and Ctrl+H work in the player window.
- [ ] Installer on a Russian/Japanese Windows: messages in that language, readable in the console.
- [ ] Watch together on three PCs with the relay deployed: the code connects, the guests' players open and follow a YouTube video (play, pause, seek), and a Netflix episode with three accounts.
- [ ] Watch together: removing a guest ends its session and changes the code; the host quitting ends it for everyone; unplugging the network for 20 s reconnects.

- [ ] Settings: the gear, Ctrl+, and the tray open it above the player; with ghost mode on it turns ghost mode off; Esc closes it.
- [ ] Settings → Privacy: picking Windows Hello or PIN shows the lock screen in the player; Lock now hides Settings.
- [ ] Settings → Advanced → hardware acceleration off → *Restart now* restarts with it off.
- [ ] First start of a fresh install: the welcome shows once; *Show it again* brings it back.
- [ ] My music with a few thousand songs: scrolling while the covers load doesn't freeze the window.
- [ ] PIN lock: after 5 wrong PINs, moving the Windows clock forward doesn't skip the wait.

## Known issues

- Watch together keeps players within about 1.5 s of each other, not frame-exact: each player
  buffers on its own. Sites that ignore seeking from outside (some DRM players) only follow play
  and pause; the window says so. Netflix seeks through its own player when the page exposes it.
- A shared window is as big as the host's window: a small player shares a small picture (up to
  1280×720). Each guest gets its own copy from the host (no server in between), so many
  guests need a good upload on the host's side. Video protected by DRM is black for the
  guests (use *Sync players*).
- While sharing, the host's page keeps drawing even with the player minimized or hidden, so
  the guests keep seeing it (it uses some more CPU and GPU than an idle hidden player).

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
  or tray → Settings → "Opacity 100%" restores it.
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
