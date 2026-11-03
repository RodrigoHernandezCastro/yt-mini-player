YT Mini Player 0.13.3
====================

A floating player for Windows 10/11: YouTube, YouTube Music, Netflix,
Crunchyroll, Prime Video, Disney+, HBO Max, Twitch and any other site.
Plus "My music": your library, playlists, duplicates and openly licensed
music downloads. And "Watch together": share a code and up to five friends
see and hear what you play.

The app speaks English, Spanish, Russian, Polish and Japanese (see
LANGUAGE below).

INSTALL
  1. Unzip this file into any folder.
  2. Pick one:
     - "Install.cmd" (normal). Copies the app to
       %LOCALAPPDATA%\Programs\YT Mini Player, creates shortcuts and opens
       it. It does not ask for administrator permission.
     - "Install-overlay.cmd" (maximum overlay). The same, but the player
       can stay above full-screen apps and games. It asks for administrator
       permission once (see further down).

  If Windows shows "Windows protected your PC" (SmartScreen), click "More
  info" → "Run anyway". It only happens the first time. Signed releases
  name their publisher there (see docs/SIGNING.md in the source).

  If your antivirus removes a file while unpacking (install.ps1 or a
  program), Install.cmd says so and explains what to do in your language;
  the same steps are in ANTIVIRUS.txt.

  The installer checks every file against SHA256SUMS.txt before
  installing: if the download is damaged or someone changed it, it stops.
  Its messages follow the Windows display language.

  Upgrading from 0.7 or older: just run Install.cmd (or
  Install-overlay.cmd) from this version. The installer files now have
  English names; the old ones (Instalar.cmd, Desinstalar.cmd...) are not
  needed any more. Your session, settings and library are kept.

LANGUAGE
  On first start the app uses the Windows display language if it is one
  of the five; otherwise English. To change it at any time:
  - Start screen: the globe button at the top right shows the current
    language in its own name (English, Español, Русский, Polski, 日本語).
  - Tray icon (right-click) → "Language" (always also written in
    English, so it can be found from any language).
  - Side bar → More (⋯) → App → Language.
  - Settings (the gear) → General → Language.
  The change applies at once, without restarting or losing what plays.

QUICK START
  The first time, a four-step welcome shows the basics (Skip closes it;
  Settings → General → "Welcome tour" shows it again).
  When it opens, pick the service: key 1–9 or click it on the keypad
  (9 = My music). To go back to the menu: the house button on the side
  bar, Alt+Home, or the tray → Main menu.

  Move ...................... drag the video (or Alt + drag)
  Resize .................... drag the edges or corners
  Buttons ................... bring the mouse to the right edge
  Settings .................. the gear (start screen or side bar), Ctrl+,
                              or tray → Settings → Open Settings…
  Shortcut help ............. F1
  Full screen in the window . Ctrl+M or the side bar's button: the video
                              fills the player window on any site
                              (Crunchyroll too); Esc goes back
  Show / hide ............... Ctrl+Alt+Y (works even when hidden)
  Play / pause .............. Ctrl+Alt+P
  Next / previous ........... Ctrl+Alt+N / Ctrl+Alt+B
  ±10 seconds ............... Ctrl+Alt+→ / Ctrl+Alt+←
  Volume / mute ............. Ctrl+Alt+↑ / Ctrl+Alt+↓ / Ctrl+Alt+M
  Exit ...................... Ctrl+Q, or right-click the tray icon → Exit

  Keyboard layouts: on layouts where Ctrl+Alt+<letter> types a character
  (AltGr), the app uses another key for that shortcut instead of stealing
  the character. On a Polish keyboard, for example, AltGr+N types "ń" and
  AltGr+L types "ł", so "next" becomes Ctrl+Alt+. (period) and "lock"
  Ctrl+Alt+K. F1, the tray menu and the music bar's tooltips always show
  the keys that really work on your PC. Window shortcuts (Ctrl+T, Ctrl+H...)
  also work on Cyrillic layouts, on the same keys.

SETTINGS
  Every option in one window, by section, with a line under each one
  saying what it does: General (language, start with Windows, open the
  last service on start, welcome tour), Window (always on top, opacity,
  ghost mode, size, corner, full screen in the window on YouTube, whether
  a video's own full-screen button fills the window, music bar), Watch
  together (your name, share or sync), Privacy and security (app lock,
  lock now, sign out of every service), Extensions and Advanced (hardware
  acceleration, global shortcuts, the folder with the session diary,
  version).
  It opens above the player, so it also turns ghost mode off. Esc closes
  it. While the app is locked it stays closed.

MUSIC BAR (IN THE TASKBAR)
  With YouTube Music open it shows up by itself inside the taskbar, left
  of the clock: cover, title, previous/pause/next and an audio wave. Also
  with Spotify, SoundCloud, Apple Music, Deezer, TIDAL or Amazon Music
  opened with "Another site".
  With other services: click the tray icon while the player is hidden (or
  tray → "Show the controls"); it hides by itself.

  The bottom line is the progress: click or drag to move around.
  Mouse wheel: volume. Click the wave: mute.
  Click the cover: shows the player. Right-click: closes the bar. Drag
  the title to move it along the taskbar.
  It does not cover app icons (it gets narrower), does not take focus
  from the game or app you are using, and hides with full-screen apps. To
  keep it from showing up by itself: tray → Settings → "Music bar in the
  taskbar".

WITH THE PLAYER HIDDEN
  Double-click the tray icon: shows the player.
  The shortcuts above also work while hidden and show the controls for a
  couple of seconds. The icon's menu (right-click) says what is playing.

MY MUSIC (KEY 9)
  Your own music, with the same taskbar bar, tray and global shortcuts.
  - Library: "Folder" adds the folder where you keep your music (MP3,
    FLAC, M4A, Ogg, Opus, WAV). Files are neither copied nor moved.
    Search, sort, shuffle, repeat; Space = play/pause.
    Each song's ⋯ menu: play next, add to a playlist, show in folder,
    remove, or move to the Recycle Bin.
  - Duplicates: finds repeated songs (same file, same song by
    artist/title/length, or same audio by acoustic fingerprint even if one
    is MP3 and the other FLAC) and suggests keeping the best quality one.
    Nothing is deleted without confirming, and it goes to the Recycle Bin.
  - Download: searches openly licensed music on the Internet Archive
    (Creative Commons and public domain; no "NonCommercial" licenses).
    You can listen first. It is saved in Music\YT Mini Player.
    What you already have is NOT downloaded again (and if a download turns
    out to be a song you already had, it is dropped so it takes no space).
    Credit the artists if you share the music.
  - Plans:
      Free .......... library, player, duplicates, listening to free
                      music online and 2 playlists
      Downloads ..... $2/month: download songs and albums, and
                      5 playlists
      Lifetime ...... $10 once: everything, with unlimited playlists
    No ads and no pop-ups: limits show where they apply ("2 of 2
    playlists"). If you change plans, nothing is deleted: extra playlists
    stay, you just cannot create new ones.
    "Plan" tab → Buy opens the payment in your browser (Lemon Squeezy),
    with no account. Paste the key from the email into "Activate".
    It is checked every 7 days (subscription) or every 30 (lifetime);
    offline, a subscription keeps working for 14 days and lifetime always.

WATCH TOGETHER
  Like screen sharing in Discord, inside the player. Open it from the
  start screen (next to the address), the side bar's ⋯ menu, Settings
  or the tray.
  - Host: "Start" gives a code like K7Q4-9XM2-PF3T. Send it to up to five
    friends. It changes by itself every 30 minutes ("New code now" changes
    it at once). You can remove someone (the code changes).
  - Two ways to watch (switch at any time):
      * Share my window: everyone sees and hears your window. Nobody else
        needs an account and nobody sees ads if you have Premium. Protected
        video (Netflix and the like) looks black to them.
      * Sync players: everyone plays it in their own app with their own
        account, and the players follow yours (for Netflix and the like).
  - The pointer button next to someone lets them use your window with
    their mouse and keyboard (or pause and seek, when syncing). Off by
    default; they can ask. Shortcuts, F keys, the app's side bar and
    sign-in or payment pages stay out of their reach.
  - Guest: type the code under "Join a friend". Sharing: the host's window
    appears in your player. Syncing: your player opens what the host is
    watching and follows it.
  Names, the code and what is playing are end-to-end encrypted; a shared
  window goes straight from the host's PC to each guest, encrypted too.
  Sharing only the app's sound needs Windows 11 (or Windows 10 build
  20348+). Where it can't, or nothing is heard, the window offers to share
  all of the PC's sound for that session (calls and notifications too).
  It needs a small relay server, free on Cloudflare (server/together-relay
  in the source; redeploy it for 0.10). If this build does not have one,
  the window asks for its address (wss://...) the first time.

HARDWARE ACCELERATION
  On by default (the graphics card decodes and draws the video). If
  videos stutter or the screen flickers, turn it off: side bar ⋯ menu or
  tray → Settings → "Hardware acceleration". It applies after restarting
  the app (it offers to do it right away).

NETFLIX, CRUNCHYROLL AND SIMILAR (DRM)
  These services need DRM (Widevine or PlayReady). The start screen checks
  whether the Windows engine (WebView2) has it and says so at the bottom.
  Right after installing it can take a few minutes to download: press
  "Check again". The best quality is the same as in Edge or Chrome on this
  PC.

ABOUT FULL-SCREEN GAMES AND APPS
  With "Always on top" on:
  - Normal install: the player goes back on top of games in "borderless
    full screen" or "windowed full screen" that cover it.
  - Game mode: with a full-screen game in front, clicking the player does
    NOT take focus from the game (it neither minimizes nor pauses:
    Minecraft and similar). The keyboard stays in the game; use the
    Ctrl+Alt shortcuts (P, N, B...) to control the video.
  - Minecraft (Java): from version 26.1, Video Settings has "Exclusive
    Fullscreen". Leave it off (the default): F11 is then borderless and
    the player stays on top. On, nothing can show over the game. Before
    26.1, play in a maximized window or use a borderless full-screen mod.
    "Fullscreen Resolution" other than "Current" is exclusive too.
  - Maximum overlay: uses the same Windows layer as the on-screen
    keyboard, above full-screen apps, the Start menu and Task Manager.
    For that the installer:
      * installs into C:\Program Files\YT Mini Player,
      * creates a local certificate, "YT Mini Player (local signing)",
        signs the program with it and deletes the private key (it cannot
        sign anything else),
      * removes the normal install so there are not two copies.
    "Uninstall.cmd" undoes all of it, certificate included (also the one
    older versions created). "Install.cmd" goes back to the normal version.
  In EXCLUSIVE full screen no window can show on top, not even with
  maximum overlay: switch the game to "borderless" or "windowed full
  screen". The player notices it and, once you leave the game, says what
  to change (session.log has "guard:" lines with the details).

CHROME EXTENSIONS
  More menu (⋯) → Extensions → Open folder. Copy unpacked extensions, .crx
  or .zip files there and press "Reload". Then reload the page with F5.
  Each new (or changed) extension asks for approval in a Windows dialog
  that shows its permissions; if it can read every page, it warns you.
  Unapproved extensions are not loaded.
  There is no Chrome Web Store: you need the extension's file (many publish
  it on GitHub). Extension button pop-ups do not exist in WebView2; if the
  extension has an options page, it opens from the menu.

SECURITY AND ACCOUNTS
  Passwords: the app never sees or stores them. You sign in on each
  service's own page; the engine's password saving and autofill are off.
  Sessions: encrypted by the engine (AES-256-GCM, key protected by
  Windows/DPAPI), as in Edge and Chrome.
  App settings (PIN included): an AES-256-GCM vault. Its key is tied to the
  PC's security chip (TPM) and your Windows account: a copy of the disk
  cannot be opened on another PC. No Windows prompts or dialogs.
  Whole disk (cache included): turn on BitLocker or Windows device
  encryption.
  Version 0.6.0 encrypted the folder with EFS and Windows asked to "back
  up your file encryption certificate": EFS is no longer used and the app
  decrypts that folder by itself. If the prompt shows up again, you can
  pick "Never".

  Tray (right-click) → Security:
    - Lock with Windows Hello (fingerprint, face or Windows PIN), or
    - Lock with a PIN of your own (only an Argon2id hash is kept). If you
      forget it, "Forgot your PIN?" removes it after Windows Hello verifies
      you; without Windows Hello, the sessions have to be cleared.
      It locks on opening, when Windows locks (Win+L), on sleep, after
      10 min hidden and with Ctrl+Alt+L.
    - Sign out of all services (clears cookies and data).
    - How you're protected (a summary of all of the above).

  Browsing: SmartScreen on; downloads blocked; camera, microphone,
  location and notifications denied; links to other sites open in your
  browser. Pages cannot give the app orders (an authenticated channel
  with a secret token per session).

UNINSTALL
  Double-click "Uninstall.cmd" (removes both versions). Sessions and
  settings are kept in %LOCALAPPDATA%\yt_mini_player; delete that folder by
  hand if you want them gone.
