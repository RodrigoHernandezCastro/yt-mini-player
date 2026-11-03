// The player's own interface, injected by the host (see webview.rs).
//
// - Right side bar that shows up when the mouse nears the edge.
// - "More" menu with the everyday options; the rest in the Settings
//   window (the gear, Ctrl+,).
// - Shortcut help (F1), short notices and local keyboard shortcuts.
// - Full screen in the window (YouTube's "video only", any player elsewhere)
//   and the real full screen, kept in sync with the host.
//
// YouTube restrictions that shape the code:
// - Trusted Types: no innerHTML; everything is built with the DOM.
// - Everything lives in a shadow DOM, to neither inherit nor break styles.
// - The bar's events must not reach the player (it would pause).
(() => {
  'use strict';

  if (window.top !== window || window.__ytmpUi) return;
  Object.defineProperty(window, '__ytmpUi', { value: true });

  const IS_YOUTUBE = /(^|\.)youtube\.com$/.test(location.hostname);
  // The app's own pages: the start screen (service picker) and My music.
  const IS_APP_PAGE = location.origin === 'https://ytmp.localhost';
  const IS_HOME = IS_APP_PAGE && location.pathname === '/';
  const REVEAL_ZONE = 64;   // px from the right edge that show the bar
  const HIDE_DELAY = 1200;  // ms before hiding it after moving away
  const TOAST_MS = 3500;
  const SVG_NS = 'http://www.w3.org/2000/svg';

  // Authenticated channel with the app (session token, see webview.rs).
  const send = (message) => __ytmpSend(message);

  let state = {
    version: '', pinned: true, videoOnly: false, fullscreenInWindow: false, opacity: 100, ghost: false,
    fullscreen: false, autostart: false, tray: true, overlay: false, hardwareAcceleration: true,
    extensions: [], extensionProblems: [], extensionsError: null, hotkeys: [],
  };

  // While the menu or the help is open, guests using a shared window cannot
  // click on it (the app is told; see remote.rs).
  let overlayReported = false;
  function reportOverlay() {
    const open = !!(menuOpen || helpOpen);
    if (open === overlayReported) return;
    overlayReported = open;
    send('menu-open:' + (open ? 1 : 0));
  }

  // ---------------------------------------------------------------------------
  // Language
  // ---------------------------------------------------------------------------

  // The app sends the strings as soon as the page says it is ready, and
  // again whenever the language changes (__ytmpLocale).
  let I18N = { lang: 'en', languages: [], strings: {} };
  let localeReady = false;

  function t(key, args) {
    let s = I18N.strings[key];
    if (s === undefined) return localeReady ? key : '';
    if (typeof s === 'object') {
      const n = Number(args && args.count);
      s = s[Number.isFinite(n) ? new Intl.PluralRules(I18N.lang).select(n) : 'other'] ?? s.other ?? s.many ?? key;
    }
    return args ? s.replace(/\{(\w+)\}/g, (m, k) => (k in args ? String(args[k]) : m)) : s;
  }

  // "Label (keys)" with each language's punctuation.
  const withKeys = (label, keys) => t('ui.with_keys', { label, keys });
  const percent = (value) => {
    try { return new Intl.NumberFormat(I18N.lang, { style: 'percent' }).format(value / 100); } catch (_) { return value + '%'; }
  };
  // Key names that differ by language ("Home" is "Inicio" on a Spanish keyboard).
  const KEY = {
    home: () => t('key.home'),
  };

  // ---------------------------------------------------------------------------
  // DOM helpers (no innerHTML)
  // ---------------------------------------------------------------------------

  function h(tag, props, ...children) {
    const el = document.createElement(tag);
    for (const [key, value] of Object.entries(props || {})) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') el.className = value;
      else if (key === 'text') el.textContent = value;
      // Only real user events: the page cannot press our buttons with
      // synthetic events.
      else if (key.startsWith('on')) el.addEventListener(key.slice(2), (e) => { if (e.isTrusted) value(e); });
      else el.setAttribute(key, value === true ? '' : String(value));
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      el.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return el;
  }

  // 24×24 stroke icons; `paths` are `d` attributes, `dots` circles.
  function icon(paths, dots = [], filled = []) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', '0 0 24 24');
    svg.setAttribute('aria-hidden', 'true');
    for (const d of paths) {
      const p = document.createElementNS(SVG_NS, 'path');
      p.setAttribute('d', d);
      svg.append(p);
    }
    for (const d of filled) {
      const p = document.createElementNS(SVG_NS, 'path');
      p.setAttribute('d', d);
      p.setAttribute('class', 'fill');
      svg.append(p);
    }
    for (const [cx, cy] of dots) {
      const c = document.createElementNS(SVG_NS, 'circle');
      c.setAttribute('cx', cx);
      c.setAttribute('cy', cy);
      c.setAttribute('r', '1.6');
      c.setAttribute('class', 'fill');
      svg.append(c);
    }
    return svg;
  }

  const ICONS = {
    grip: () => icon([], [[9, 6], [15, 6], [9, 12], [15, 12], [9, 18], [15, 18]]),
    back: () => icon(['M15 18l-6-6 6-6']),
    home: () => icon(['M4 11.5L12 5l8 6.5', 'M6.5 10v9h11v-9', 'M10 19v-5h4v5']),
    videoOnly: () => icon(['M3 7a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z'], [], ['M10 9l5 3-5 3z']),
    fullscreen: () => icon(['M4 9V4h5', 'M20 9V4h-5', 'M4 15v5h5', 'M20 15v5h-5']),
    exitFullscreen: () => icon(['M9 4v5H4', 'M15 4v5h5', 'M9 20v-5H4', 'M15 20v-5h5']),
    pin: () => icon(['M12 16v5', 'M8 4h8', 'M9.5 4v5.5L6.5 13v3h11v-3l-3-3.5V4']),
    hide: () => icon(['M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z', 'M12 9.2a2.8 2.8 0 1 0 0 5.6 2.8 2.8 0 0 0 0-5.6z', 'M4 4l16 16']),
    more: () => icon([], [[6, 12], [12, 12], [18, 12]]),
    close: () => icon(['M6 6l12 12', 'M18 6L6 18']),
    globe: () => icon(['M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z', 'M3 12h18', 'M12 3c2.6 2.5 3.9 5.5 3.9 9s-1.3 6.5-3.9 9c-2.6-2.5-3.9-5.5-3.9-9S9.4 5.5 12 3z']),
    settings: () => icon(['M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
      'M19.2 13.4a7.4 7.4 0 0 0 0-2.8l1.9-1.5-1.9-3.3-2.3.9a7.3 7.3 0 0 0-2.4-1.4L14.1 3h-4.2l-.4 2.3a7.3 7.3 0 0 0-2.4 1.4l-2.3-.9-1.9 3.3 1.9 1.5a7.4 7.4 0 0 0 0 2.8l-1.9 1.5 1.9 3.3 2.3-.9a7.3 7.3 0 0 0 2.4 1.4l.4 2.3h4.2l.4-2.3a7.3 7.3 0 0 0 2.4-1.4l2.3.9 1.9-3.3z']),
  };

  // ---------------------------------------------------------------------------
  // Styles
  // ---------------------------------------------------------------------------

  function sheet(css) {
    try {
      const s = new CSSStyleSheet();
      s.replaceSync(css);
      return s;
    } catch (_) {
      return null;
    }
  }

  const UI_CSS = `
    :host { all: initial; }
    * { box-sizing: border-box; }
    /* The app's palette, deepest to lightest: deep twilight, French blue,
       bright teal blue, blue green, turquoise surf, sky aqua, frosted blue
       (two) and light cyan. */
    .root {
      --deep: #03045e;
      --french: #023e8a;
      --teal: #0077b6;
      --blue-green: #0096c7;
      --turquoise: #00b4d8;
      --sky: #48cae4;
      --frost: #90e0ef;
      --frost-2: #ade8f4;
      --cyan: #caf0f8;
      --dim: rgba(144, 224, 239, .62);
      --tint: rgba(2, 62, 138, .55);
      --alert: #ff9f8a;
      --tide: linear-gradient(90deg, var(--teal), var(--blue-green), var(--turquoise), var(--sky), var(--frost), var(--frost-2), var(--cyan));
      font: 13px/1.4 "Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo UI", system-ui, sans-serif;
      color: var(--cyan);
      -webkit-font-smoothing: antialiased;
    }
    svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 1.8;
          stroke-linecap: round; stroke-linejoin: round; display: block; }
    svg .fill { fill: currentColor; stroke: none; }

    .bar {
      position: fixed; top: 50%; right: 10px; z-index: 2147483647;
      display: flex; flex-direction: column; gap: 1px; padding: 5px 4px;
      background: linear-gradient(180deg, rgba(2, 62, 138, .96), rgba(3, 4, 94, .96));
      border: 1px solid var(--teal); border-radius: 18px;
      box-shadow: 0 10px 30px rgba(1, 2, 40, .55);
      transform: translate(calc(100% + 16px), -50%); opacity: 0;
      transition: transform .18s ease, opacity .18s ease;
      pointer-events: none;
    }
    .bar.show { transform: translate(0, -50%); opacity: 1; pointer-events: auto; }
    .bar .sep { height: 1px; margin: 4px 6px; background: var(--tide); opacity: .55; }
    @media (max-height: 300px) { .bar [data-secondary] { display: none; } }
    @media (max-height: 190px) { .bar .sep { display: none; } }

    button { font: inherit; color: inherit; background: none; border: 0; margin: 0; cursor: pointer; }
    button:focus-visible { outline: 2px solid var(--sky); outline-offset: 1px; }
    .bar button {
      position: relative; width: 32px; height: 32px; border-radius: 50%; display: grid; place-items: center;
      color: var(--frost);
      transition: background-color .12s, color .12s;
    }
    .bar button[hidden] { display: none; }
    .bar button:hover { background: var(--teal); color: var(--cyan); }
    .bar button.on { background: var(--turquoise); color: var(--deep); }
    .bar button.on:hover { background: var(--sky); }
    .bar button.grip { cursor: grab; color: var(--blue-green); }
    .bar button.grip:hover { background: none; color: var(--sky); }
    .bar button.danger:hover { background: var(--alert); color: var(--deep); }
    /* The name of the button under the pointer, at once and next to it
       (the system tooltip takes a second and covers the bar). */
    .tip {
      position: fixed; z-index: 2147483647; right: 60px; top: 0; max-width: min(260px, calc(100vw - 80px));
      padding: 5px 10px 6px; border-radius: 8px; background: var(--deep); border: 1px solid var(--teal);
      color: var(--cyan); font-size: 12.5px; line-height: 1.35; pointer-events: none;
      box-shadow: 0 8px 22px rgba(1, 2, 40, .55);
    }
    .tip[hidden] { display: none; }
    .tip kbd { margin-left: 6px; }

    .panel {
      position: fixed; z-index: 2147483647; top: 8px; right: 58px;
      width: 300px; max-width: calc(100vw - 16px); max-height: calc(100vh - 16px); overflow: auto;
      background: var(--deep); border: 1px solid var(--teal); border-radius: 10px; padding: 0 0 4px;
      box-shadow: 0 14px 36px rgba(1, 2, 40, .6);
      scrollbar-width: thin; scrollbar-color: var(--teal) transparent;
    }
    .panel::before { content: ""; display: block; height: 3px; background: var(--tide); }
    .panel[hidden] { display: none; }
    @media (max-width: 380px) { .panel { left: 8px; width: auto; } }
    @media (max-width: 360px) { .panel kbd { display: none; } }
    .section { padding: 6px 0; border-top: 1px solid var(--french); }
    .section:first-of-type { border-top: 0; }
    .section h3 { margin: 6px 14px 4px; font-size: 12px; font-weight: 600; color: var(--sky); }
    .row {
      display: flex; align-items: center; gap: 10px; width: 100%; min-height: 32px;
      padding: 4px 14px; text-align: left;
    }
    button.row:hover { background: linear-gradient(90deg, var(--tint), rgba(2, 62, 138, 0)); }
    .row .label { flex: 1; min-width: 0; }
    .row .sub { display: block; color: var(--dim); font-size: 12px; overflow-wrap: anywhere; }
    .row .sub.error { color: var(--alert); }
    kbd {
      font-family: inherit; font-size: 11px; line-height: 1; color: var(--frost);
      font-variant-numeric: tabular-nums; white-space: nowrap;
      padding: 3px 6px; border-radius: 4px; background: var(--french);
    }
    .switch {
      flex: none; width: 30px; height: 16px; border-radius: 8px; background: var(--french); position: relative;
      box-shadow: inset 0 0 0 1px var(--teal); transition: background .15s;
    }
    .switch::after {
      content: ""; position: absolute; top: 3px; left: 3px; width: 10px; height: 10px; border-radius: 50%;
      background: var(--frost); transition: transform .15s, background .15s;
    }
    .switch.on { background: linear-gradient(90deg, var(--blue-green), var(--sky)); box-shadow: none; }
    .switch.on::after { transform: translateX(14px); background: var(--deep); }
    .chips { display: flex; gap: 4px; }
    .chip {
      min-width: 30px; height: 26px; padding: 0 8px; border-radius: 13px; border: 1px solid var(--teal);
      display: grid; place-items: center; font-size: 13px; line-height: 1; color: var(--frost-2);
    }
    .chip:hover { background: var(--teal); color: var(--cyan); }
    .value { min-width: 44px; text-align: center; font-variant-numeric: tabular-nums; color: var(--turquoise); font-weight: 600; }
    .hint { margin: 2px 14px 6px; color: var(--dim); font-size: 12px; }
    .actions { display: flex; gap: 6px; padding: 4px 14px 2px; flex-wrap: wrap; }
    .actions .chip { padding: 0 12px; }
    .link { color: var(--sky); font-size: 12px; padding: 2px 4px; border-radius: 4px; }
    .link:hover { text-decoration: underline; }
    .footer { padding: 6px 14px 2px; color: var(--blue-green); font-size: 11.5px; }
    .row .globe { flex: none; width: 16px; height: 16px; stroke-width: 1.5; color: var(--sky); }
    .row .current { color: var(--frost); white-space: nowrap; }
    .row.language { padding-left: 40px; }
    .row.language .check { width: 14px; color: var(--turquoise); visibility: hidden; }
    .row.language[aria-checked="true"] .check { visibility: visible; }

    .help {
      position: fixed; inset: 0; z-index: 2147483647; display: grid; place-items: center;
      background: rgba(3, 4, 94, .72); padding: 12px;
    }
    .help[hidden] { display: none; }
    /* Full screen in the window: thin strips on the window's edges, above a
       player in a frame (its events never reach this page), so the window
       can still be resized and the side bar found. */
    .edge { position: fixed; z-index: 2147483646; background: transparent; }
    .edge[hidden] { display: none; }
    .edge.n { top: 0; left: 0; right: 0; height: 6px; }
    .edge.s { bottom: 0; left: 0; right: 0; height: 6px; }
    .edge.w { top: 0; bottom: 0; left: 0; width: 6px; }
    .edge.e { top: 0; bottom: 0; right: 0; width: 6px; }
    .help .card {
      width: min(560px, 100%); max-height: 100%; overflow: auto; background: var(--deep);
      border: 1px solid var(--teal); border-radius: 12px; padding: 0 20px 18px;
      box-shadow: 0 18px 50px rgba(1, 2, 40, .7);
    }
    .help .card::before { content: ""; display: block; height: 3px; margin: 0 -20px 14px; background: var(--tide); }
    .help h2 { margin: 0 0 4px; font: 600 17px/1.3 "Segoe UI Variable Display", "Segoe UI", "Yu Gothic UI", system-ui, sans-serif; }
    .help h3 { margin: 16px 0 6px; font-size: 12px; font-weight: 600; color: var(--sky); }
    .help table { width: 100%; border-collapse: collapse; }
    .help td { padding: 5px 0; vertical-align: top; border-top: 1px solid var(--french); }
    .help td:first-child { width: 42%; padding-right: 12px; color: var(--turquoise); font-variant-numeric: tabular-nums; }
    .help .off { color: var(--dim); text-decoration: line-through; }
    .help .note { color: var(--dim); font-size: 12px; margin-top: 10px; }

    .toasts {
      position: fixed; left: 50%; bottom: 18px; transform: translateX(-50%); z-index: 2147483647;
      display: flex; flex-direction: column; align-items: center; gap: 6px; pointer-events: none;
      width: max-content; max-width: calc(100vw - 120px);
    }
    .toast {
      position: relative; overflow: hidden;
      background: linear-gradient(90deg, var(--french), var(--deep)); border: 1px solid var(--teal);
      border-radius: 18px; padding: 8px 16px 9px; animation: in .18s ease; text-align: center;
      box-shadow: 0 8px 24px rgba(1, 2, 40, .55);
    }
    .toast::after { content: ""; position: absolute; left: 14px; right: 14px; bottom: 0; height: 2px; background: var(--tide); opacity: .8; }
    .toast.out { opacity: 0; transition: opacity .3s; }
    @keyframes in { from { opacity: 0; transform: translateY(4px); } }
    @media (prefers-reduced-motion: reduce) {
      .bar, .toast, .switch, .switch::after, .bar button { transition: none; animation: none; }
    }
  `;

  // Video only mode: the player fills the whole window on /watch.
  //
  // z-index cannot be trusted: YouTube creates several stacking contexts
  // between the player and the root, and its content would be painted on
  // top. Instead, everything that is not the player becomes invisible
  // (visibility neither paints nor gets clicks) and the player is fixed to
  // the window. Ancestors with transform/filter/contain, which would change
  // what "fixed" refers to, are neutralised from JS.
  const VO_CLASS = 'ytmp-video-only';
  const VO_CHAIN = 'ytmp-vo-chain';
  // The player found by findPlayer() carries this attribute, so the CSS
  // does not depend on YouTube's names.
  const PLAYER_ATTR = 'data-ytmp-player';
  const PLAYER = `[${PLAYER_ATTR}]`;
  const VIDEO_ONLY_CSS = `
    html.${VO_CLASS}, html.${VO_CLASS} body { overflow: hidden !important; background: #000 !important; }
    html.${VO_CLASS} body *:not(${PLAYER}):not(${PLAYER} *) { visibility: hidden !important; }
    html.${VO_CLASS} .${VO_CHAIN} {
      transform: none !important; filter: none !important; backdrop-filter: none !important;
      perspective: none !important; contain: none !important; will-change: auto !important;
    }
    html.${VO_CLASS} ${PLAYER} {
      visibility: visible !important;
      position: fixed !important; inset: 0 !important; margin: 0 !important;
      width: 100vw !important; height: 100vh !important; max-width: none !important; max-height: none !important;
      z-index: 2147483000 !important; background: #000 !important; border: 0 !important;
    }
    html.${VO_CLASS} video${PLAYER} { object-fit: contain !important; }
    html.${VO_CLASS} ${PLAYER} .html5-video-container { width: 100% !important; height: 100% !important; }
    html.${VO_CLASS} ${PLAYER} video {
      width: 100% !important; height: 100% !important; left: 0 !important; top: 0 !important;
      object-fit: contain !important;
    }
  `;
  const videoOnlySheet = sheet(VIDEO_ONLY_CSS);

  function ensureDocumentSheet() {
    if (!videoOnlySheet || !document.adoptedStyleSheets) return;
    if (!document.adoptedStyleSheets.includes(videoOnlySheet)) {
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, videoOnlySheet];
    }
  }

  const isWatch = () => IS_YOUTUBE && location.pathname === '/watch';

  // YouTube's player, by its names and, if YouTube renames it, by what it
  // is: the box around the biggest video. `how` says which way found it
  // (the self-check reports anything but "main").
  const PLAYER_SELECTORS = [['#movie_player', 'main'], ['.html5-video-player', 'fallback']];
  function findPlayer() {
    for (const [selector, how] of PLAYER_SELECTORS) {
      const el = document.querySelector(selector);
      if (el) return { el, how };
    }
    let biggest = null;
    let area = 0;
    for (const video of document.querySelectorAll('video')) {
      const r = video.getBoundingClientRect();
      if (r.width * r.height > area) { biggest = video; area = r.width * r.height; }
    }
    if (biggest && biggest.parentElement && biggest.parentElement !== document.body) {
      return { el: biggest.parentElement, how: 'video' };
    }
    return { el: null, how: 'none' };
  }

  // Marks the player (and only it) for the CSS.
  function markPlayer(player) {
    for (const el of document.querySelectorAll(PLAYER)) if (el !== player) el.removeAttribute(PLAYER_ATTR);
    if (player && !player.hasAttribute(PLAYER_ATTR)) player.setAttribute(PLAYER_ATTR, '');
  }

  // --- Full screen in the window -----------------------------------------------
  //
  // The video fills the player window (not the screen), on any site, with
  // the same CSS as YouTube's "video only": everything else is invisible and
  // the player is fixed to the window. On YouTube that is the "video only"
  // setting (every watch page). Elsewhere it is asked for each time (the
  // side bar's button, Ctrl+M, or the video's own full-screen button when
  // Settings say so) and lasts until asked again or the player goes away.
  // No real full screen is involved: no click is needed to start it, the
  // window keeps its size and the side bar stays.
  const MIN_FILL_AREA = 0.08;  // of the window, for a video or a frame to count
  let fill = null;             // the element filling the window, on request
  let fillTimer = 0;
  let screenUntil = 0;         // a real full screen asked by the app's own button

  // The visible element of `selector` with the largest area, if any.
  function largest(selector) {
    let best = null;
    for (const el of document.querySelectorAll(selector)) {
      const r = el.getBoundingClientRect();
      const area = r.width * r.height;
      if (r.width >= 160 && r.height >= 90 && (!best || area > best.area)
        && getComputedStyle(el).visibility !== 'hidden') best = { el, area };
    }
    return best;
  }

  // The box around a video that holds its controls too: its ancestors while
  // they stay about the video's size (as wide, at most half as tall again
  // for a control bar under it), never a column beside it (a chat).
  function playerBox(video) {
    const r = video.getBoundingClientRect();
    let box = video;
    for (let el = video.parentElement; el && el !== document.body && el !== document.documentElement; el = el.parentElement) {
      const b = el.getBoundingClientRect();
      if (b.width > r.width * 1.1 + 2 || b.height > r.height * 1.5 + 2) break;
      box = el;
    }
    return box;
  }

  // What should fill the window here: the largest video (with its
  // controls) or the largest frame, which is how Crunchyroll and many other
  // sites embed their player.
  function findFillTarget() {
    if (isWatch()) return findPlayer().el;
    const min = innerWidth * innerHeight * MIN_FILL_AREA;
    const best = [largest('video'), largest('iframe')].filter((c) => c && c.area >= min).sort((a, b) => b.area - a.area)[0];
    if (!best) return null;
    return best.el.tagName === 'VIDEO' ? playerBox(best.el) : best.el;
  }

  // Is the window filled (or, on YouTube, set to be on watch pages)?
  const filling = () => (IS_YOUTUBE ? !!state.videoOnly : !!fill);

  function setFill(el) {
    fill = el || null;
    clearInterval(fillTimer);
    // The player can go away (another page, an episode list): checked every
    // second while it fills the window, never otherwise.
    if (fill) {
      fillTimer = setInterval(() => {
        if (!fill || !fill.isConnected || getComputedStyle(fill).display === 'none') setFill(null);
      }, 1000);
    }
    applyVideoOnly();
    render();
  }

  // The side bar's button and Ctrl+M.
  function toggleFill() {
    if (IS_YOUTUBE) {
      send('video-only');
      return;
    }
    if (fill) {
      setFill(null);
      return;
    }
    const target = findFillTarget();
    if (target) setFill(target);
    else toast(t('ui.fill_nothing'));
  }

  // The video's own full-screen button (or a double click) while Settings
  // say it fills the window: the real full screen is left at once and the
  // same element fills the window instead; pressed again, it stops.
  function fullscreenToWindow(el) {
    document.exitFullscreen().catch(() => {});
    if (IS_YOUTUBE) {
      if (isWatch()) send('video-only');
      return;
    }
    if (fill) {
      setFill(null);
      return;
    }
    const whole = el === document.documentElement || el === document.body;
    setFill(whole ? findFillTarget() : el);
  }

  // Marks the player's ancestors that would make "fixed" relative to them
  // (transform, filter, contain...). Worked out every time, because YouTube
  // moves the player between containers (theatre, default).
  function markPlayerChain(player) {
    for (const el of document.querySelectorAll('.' + VO_CHAIN)) el.classList.remove(VO_CHAIN);
    for (let el = player && player.parentElement; el && el !== document.documentElement; el = el.parentElement) {
      const cs = getComputedStyle(el);
      if (cs.transform !== 'none' || cs.filter !== 'none' || cs.perspective !== 'none'
        || (cs.backdropFilter && cs.backdropFilter !== 'none')
        || (cs.contain && cs.contain !== 'none')
        || /transform|filter|perspective/.test(cs.willChange || '')) {
        el.classList.add(VO_CHAIN);
      }
    }
  }

  function applyVideoOnly() {
    const root = document.documentElement;
    if (!root) return;
    ensureDocumentSheet();
    if (fill && !fill.isConnected) fill = null;
    const player = fill || (isWatch() ? findPlayer().el : null);
    markPlayer(player);
    // In a real full screen the site is in charge (its own layout already
    // shows only the video, and mixing both breaks the page), except when
    // the whole page went full screen from the side bar while a player was
    // filling the window: then it keeps filling, now the screen.
    const fs = document.fullscreenElement;
    const wanted = fill ? true : !!state.videoOnly && isWatch();
    const on = wanted && !!player && (!fs || (!!fill && fs === document.documentElement));
    if (on) markPlayerChain(player);
    if (edges) for (const edge of edges) edge.hidden = !(on && !!fill);
    if (root.classList.contains(VO_CLASS) === on) return;
    root.classList.toggle(VO_CLASS, on);
    if (!on) for (const el of document.querySelectorAll('.' + VO_CHAIN)) el.classList.remove(VO_CHAIN);
    // So the player works out the size of its controls again.
    requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
  }

  // ---------------------------------------------------------------------------
  // Page actions
  // ---------------------------------------------------------------------------

  function toggleFullscreen() {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
      return;
    }
    // This one fills the screen even when a video's own button fills the
    // window (fullscreenchange tells them apart by this).
    screenUntil = performance.now() + 2000;
    // With YouTube's own button its interface stays consistent.
    const player = isWatch() && findPlayer().el;
    const button = player && player.querySelector('.ytp-fullscreen-button');
    if (button) button.click();
    else document.documentElement.requestFullscreen().catch(() => {});
  }

  // Main menu (service picker).
  function goHome() {
    send('home');
  }

  // The Settings window (at a section, if given).
  function openSettings(section) {
    closeMenu();
    send(typeof section === 'string' ? 'settings:' + section : 'settings');
  }

  // ---------------------------------------------------------------------------
  // Local shortcuts (while the window has focus)
  // ---------------------------------------------------------------------------

  const onlyCtrl = (e) => e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
  const noMods = (e) => !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
  // By the letter printed on the key when the layout types Latin letters
  // (so Dvorak or AZERTY get "their" T), and by position when it does not:
  // on a Russian keyboard Ctrl+T is typed on the "Е" key.
  const letter = (e, l) => {
    const key = (e.key || '').toLowerCase();
    return /^[a-z]$/.test(key) ? key === l : e.code === 'Key' + l.toUpperCase();
  };
  // Keys whose name depends on the language ("{home}" → Home, Inicio…).
  const keyText = (keys) => keys.replace('{home}', KEY.home());

  // No `run` = a built-in shortcut, only listed in the help.
  const SHORTCUTS = [
    { keys: 'F11', label: 'ui.fullscreen', match: (e) => e.key === 'F11' && noMods(e), run: toggleFullscreen },
    { keys: 'Ctrl+T', label: 'ui.always_on_top', match: (e) => onlyCtrl(e) && letter(e, 't'), run: () => send('pin') },
    { keys: 'Ctrl+M', label: 'ui.fill_window', match: (e) => !IS_APP_PAGE && onlyCtrl(e) && letter(e, 'm'), run: toggleFill },
    { keys: 'Ctrl+H', label: 'ui.hide_to_tray', match: (e) => onlyCtrl(e) && letter(e, 'h'), run: () => send('hide') },
    { keys: 'Ctrl+,', label: 'ui.settings', match: (e) => onlyCtrl(e) && (e.code === 'Comma' || e.key === ','), run: openSettings },
    { keys: 'Ctrl+L', label: 'ui.open_copied_youtube_link', match: (e) => onlyCtrl(e) && letter(e, 'l'), run: () => send('open-clipboard') },
    {
      keys: 'Ctrl+1 / 2 / 3', label: 'ui.size_small_medium_large',
      match: (e) => onlyCtrl(e) && /^Digit[123]$/.test(e.code),
      run: (e) => send('size:' + { Digit1: 's', Digit2: 'm', Digit3: 'l' }[e.code]),
    },
    {
      keys: 'Ctrl+Shift+1 … 4', label: 'ui.move_to_corner',
      match: (e) => e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && /^Digit[1-4]$/.test(e.code),
      run: (e) => send('corner:' + { Digit1: 'nw', Digit2: 'ne', Digit3: 'sw', Digit4: 'se' }[e.code]),
    },
    { keys: 'Alt+{home}', label: 'ui.main_menu_pick', match: (e) => e.altKey && !e.ctrlKey && !e.shiftKey && e.key === 'Home', run: goHome },
    { keys: 'Alt+← / Alt+→', label: 'ui.back_forward' },
    { keys: 'F5', label: 'ui.reload_page' },
    { keys: 'F1', label: 'ui.this_help', match: (e) => e.key === 'F1' && noMods(e), run: () => toggleHelp() },
    { keys: 'Ctrl+Q', label: 'ui.quit', match: (e) => onlyCtrl(e) && letter(e, 'q'), run: () => send('quit') },
    { keys: 'Esc', label: 'ui.close_menu_or_help' },
  ];

  const GESTURES = [
    ['ui.gesture.drag_video', 'ui.gesture.drag_video_does'],
    ['ui.gesture.alt_drag', 'ui.gesture.alt_drag_does'],
    ['ui.gesture.edges', 'ui.gesture.edges_do'],
    ['ui.gesture.right_edge', 'ui.gesture.right_edge_does'],
  ];

  window.addEventListener('keydown', (e) => {
    if (!e.isTrusted || e.isComposing || e.repeat) return;
    if (e.key === 'Escape' && (menuOpen || helpOpen)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      closeMenu();
      closeHelp();
      return;
    }
    // Escape leaves full screen in the window, as it leaves a real one.
    if (e.key === 'Escape' && fill && !document.fullscreenElement && noMods(e)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      setFill(null);
      return;
    }
    const shortcut = SHORTCUTS.find((s) => s.run && s.match(e));
    if (!shortcut) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    shortcut.run(e);
  }, { capture: true });

  // ---------------------------------------------------------------------------
  // Building the interface
  // ---------------------------------------------------------------------------

  let host = null, root = null, bar = null, panel = null, help = null, toasts = null, tip = null, tipTimer = 0;
  let edges = null;
  let menuOpen = false, helpOpen = false, barHover = false, barFocus = false, hideTimer = 0, languagesOpen = false;
  const barButtons = {};

  // Tooltip of each side bar button: [text key, shortcut].
  const BAR_TITLES = {
    grip: ['ui.move_window', null],
    home: ['ui.main_menu', 'Alt+{home}'],
    back: ['ui.back', 'Alt+←'],
    videoOnly: ['ui.fill_window', 'Ctrl+M'],
    pinned: ['ui.always_on_top', 'Ctrl+T'],
    hide: ['ui.hide_to_tray', 'Ctrl+H'],
    settings: ['ui.settings', 'Ctrl+,'],
    more: ['ui.more_options', null],
    quit: ['ui.close', 'Ctrl+Q'],
  };

  function setTitle(button, label, keys) {
    button.dataset.label = label;
    button.dataset.keys = keys || '';
    button.setAttribute('aria-label', keys ? withKeys(label, keys) : label);
    button.removeAttribute('title');
  }

  // The button's name next to it: after a moment with the mouse, at once
  // with the keyboard.
  function showTip(button, now) {
    clearTimeout(tipTimer);
    const go = () => {
      if (!tip || !button.isConnected || button.hidden || menuOpen || !bar.classList.contains('show')) return;
      if (!button.dataset.label) return;
      tip.replaceChildren(button.dataset.label, button.dataset.keys ? h('kbd', { text: button.dataset.keys }) : '');
      tip.hidden = false;
      const r = button.getBoundingClientRect();
      const height = tip.getBoundingClientRect().height;
      tip.style.right = Math.round(window.innerWidth - r.left + 8) + 'px';
      tip.style.top = Math.round(Math.max(4, Math.min(window.innerHeight - height - 4, r.top + r.height / 2 - height / 2))) + 'px';
    };
    if (now) go();
    else tipTimer = setTimeout(go, 220);
  }

  function hideTip() {
    clearTimeout(tipTimer);
    if (tip) tip.hidden = true;
  }

  function barButton(key, iconFn, onClick, opts = {}) {
    const button = h('button', {
      type: 'button', class: opts.class || null, 'data-secondary': opts.secondary || null, onclick: onClick,
    }, iconFn());
    button.addEventListener('pointerenter', () => showTip(button, false));
    button.addEventListener('pointerleave', hideTip);
    button.addEventListener('focus', () => { if (button.matches(':focus-visible')) showTip(button, true); });
    button.addEventListener('blur', hideTip);
    button.addEventListener('click', hideTip);
    barButtons[key] = button;
    return button;
  }

  function mount() {
    if (host && host.isConnected) return;
    if (!document.documentElement) return;

    host = document.createElement('ytmp-ui');
    const shadow = host.attachShadow({ mode: 'closed' });
    const uiSheet = sheet(UI_CSS);
    if (uiSheet) shadow.adoptedStyleSheets = [uiSheet];
    else shadow.append(h('style', { text: UI_CSS }));

    // Nothing that happens inside must reach the player or YouTube.
    for (const type of ['click', 'dblclick', 'mousedown', 'mouseup', 'pointerdown', 'pointerup',
      'keydown', 'keyup', 'wheel', 'contextmenu']) {
      shadow.addEventListener(type, (e) => e.stopPropagation());
    }

    root = h('div', { class: 'root' });

    const grip = barButton('grip', ICONS.grip, null, { class: 'grip' });
    grip.addEventListener('pointerdown', (e) => {
      if (!e.isTrusted || e.button !== 0) return;
      e.preventDefault();
      if (typeof __ytmpShared.startDrag === 'function') __ytmpShared.startDrag();
      else send('drag');
    });

    bar = h('div', { class: 'bar', role: 'toolbar', 'aria-label': 'YT Mini Player' },
      grip,
      barButton('home', ICONS.home, goHome),
      barButton('back', ICONS.back, () => history.back(), { secondary: true }),
      h('div', { class: 'sep', 'data-secondary': true }),
      barButton('videoOnly', ICONS.videoOnly, toggleFill, { secondary: true }),
      barButton('fullscreen', ICONS.fullscreen, toggleFullscreen),
      barButton('pinned', ICONS.pin, () => send('pin'), { secondary: true }),
      barButton('hide', ICONS.hide, () => send('hide'), { secondary: true }),
      h('div', { class: 'sep' }),
      barButton('settings', ICONS.settings, () => openSettings()),
      barButton('more', ICONS.more, () => (menuOpen ? closeMenu() : openMenu())),
      barButton('quit', ICONS.close, () => send('quit'), { class: 'danger' }),
    );
    bar.addEventListener('pointerenter', () => { barHover = true; showBar(); });
    bar.addEventListener('pointerleave', () => { barHover = false; scheduleHide(); });
    // Reached with Tab: the bar shows itself (a hidden button with the
    // focus helps nobody).
    bar.addEventListener('focusin', () => { barFocus = true; showBar(); });
    bar.addEventListener('focusout', () => { barFocus = false; scheduleHide(); });

    panel = h('div', { class: 'panel', hidden: true, role: 'menu' });
    help = h('div', { class: 'help', hidden: true, onclick: (e) => { if (e.target === help) closeHelp(); } });
    toasts = h('div', { class: 'toasts', role: 'status', 'aria-live': 'polite' });
    tip = h('div', { class: 'tip', hidden: true, 'aria-hidden': 'true' });

    edges = ['n', 's', 'w', 'e'].map((side) => h('div', { class: 'edge ' + side, hidden: true, 'aria-hidden': 'true' }));
    root.append(...edges, bar, tip, panel, help, toasts);
    shadow.append(root);
    placeHost();
    render();
  }

  // In fullscreen only the fullscreen element is painted: the interface
  // moves inside it to stay available.
  function placeHost() {
    if (!host) return;
    const parent = document.fullscreenElement || document.documentElement;
    if (host.parentNode !== parent) parent.appendChild(host);
  }

  // ---------------------------------------------------------------------------
  // Bar: show / hide
  // ---------------------------------------------------------------------------

  function showBar() {
    if (!bar || state.ghost) return;
    clearTimeout(hideTimer);
    bar.classList.add('show');
  }

  function scheduleHide(delay = HIDE_DELAY) {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!barHover && !barFocus && !menuOpen && bar) {
        bar.classList.remove('show');
        hideTip();
      }
    }, delay);
  }

  window.addEventListener('pointermove', (e) => {
    if (!bar || e.pointerType !== 'mouse') return;
    const near = e.clientX >= window.innerWidth - REVEAL_ZONE;
    if (near) showBar();
    else if (bar.classList.contains('show') && !barHover && !menuOpen) scheduleHide();
  }, { capture: true, passive: true });

  document.addEventListener('mouseleave', () => scheduleHide(400), { passive: true });

  // ---------------------------------------------------------------------------
  // "More" menu
  // ---------------------------------------------------------------------------

  function openMenu() {
    hideTip();
    menuOpen = true;
    languagesOpen = false;
    closeHelp();
    renderMenu();
    panel.hidden = false;
    // A menu opens at its top, wherever it was left.
    panel.scrollTop = 0;
    showBar();
    barButtons.more.classList.add('on');
    reportOverlay();
  }

  function closeMenu() {
    if (!menuOpen) return;
    menuOpen = false;
    panel.hidden = true;
    barButtons.more.classList.remove('on');
    scheduleHide();
    reportOverlay();
  }

  // Close on a click outside (the page does not get that extra click).
  window.addEventListener('pointerdown', (e) => {
    if (menuOpen && e.target !== host) closeMenu();
  }, { capture: true });

  function section(title, ...rows) {
    return h('div', { class: 'section' }, h('h3', { text: title }), ...rows);
  }

  function actionRow(label, keys, onClick, sub = null) {
    return h('button', { type: 'button', class: 'row', role: 'menuitem', onclick: onClick },
      h('span', { class: 'label' }, label, sub && h('span', { class: 'sub', text: sub })),
      keys && h('kbd', { text: keyText(keys) }));
  }

  function toggleRow(label, keys, on, onClick, sub = null) {
    return h('button', { type: 'button', class: 'row', role: 'menuitemcheckbox', 'aria-checked': String(!!on), onclick: onClick },
      h('span', { class: 'label' }, label, sub && h('span', { class: 'sub', text: sub })),
      keys && h('kbd', { text: keyText(keys) }),
      h('span', { class: 'switch' + (on ? ' on' : '') }));
  }

  function chipsRow(label, chips) {
    return h('div', { class: 'row' },
      h('span', { class: 'label', text: label }),
      h('span', { class: 'chips' }, chips.map(([text, title, onClick]) =>
        h('button', { type: 'button', class: 'chip', title, 'aria-label': title, onclick: onClick, text }))));
  }

  // Language: the globe and each language's own name, so it can be found
  // even by someone who cannot read the language the app is in.
  function languageRows() {
    const current = I18N.languages.find((l) => l.code === I18N.lang);
    const globe = ICONS.globe();
    globe.setAttribute('class', 'globe');
    const rows = [h('button', {
      type: 'button', class: 'row', role: 'menuitem', 'aria-haspopup': 'true', 'aria-expanded': String(languagesOpen),
      onclick: () => { languagesOpen = !languagesOpen; renderMenu(); },
    }, globe, h('span', { class: 'label', text: t('lang.label') }),
    current && h('span', { class: 'current', lang: current.code, text: current.name }))];
    if (!languagesOpen) return rows;
    let names = null;
    try { names = new Intl.DisplayNames([I18N.lang], { type: 'language' }); } catch (_) { /* older engine */ }
    for (const language of I18N.languages) {
      const local = names && language.code !== I18N.lang ? names.of(language.code) : '';
      rows.push(h('button', {
        type: 'button', class: 'row language', role: 'menuitemradio', 'aria-checked': String(language.code === I18N.lang),
        onclick: () => {
          languagesOpen = false;
          if (language.code !== I18N.lang) send('lang:' + language.code);
          else renderMenu();
        },
      }, h('span', { class: 'check', 'aria-hidden': 'true', text: '✓' }),
      h('span', { class: 'label' }, h('span', { lang: language.code, text: language.name }),
        local && local.toLowerCase() !== language.name.toLowerCase() && h('span', { class: 'sub', text: local }))));
    }
    return rows;
  }

  // "2 on, 1 off", "None yet" or the problem, under the Extensions row
  // (the list and the switches are in Settings).
  function extensionSummary() {
    if (state.extensionsError) return state.extensionsError;
    const problems = state.extensionProblems.length + state.extensions.filter((e) => e.error).length;
    if (problems) return t('ui.ext.problems', { count: problems });
    const on = state.extensions.filter((e) => e.enabled).length;
    if (!state.extensions.length) return t('ui.ext.none');
    return t('ui.ext.summary', { on, count: state.extensions.length });
  }

  function renderMenu() {
    if (!panel) return;
    const op = state.opacity;
    const fs = !!document.fullscreenElement;
    panel.replaceChildren(
      section(t('ui.section.window'),
        toggleRow(t('ui.always_on_top'), 'Ctrl+T', state.pinned, () => send('pin')),
        !IS_APP_PAGE && toggleRow(t('ui.fill_window'), 'Ctrl+M', filling(), toggleFill,
          IS_YOUTUBE && !isWatch() ? t('ui.video_only_later') : null),
        actionRow(t(fs ? 'ui.exit_fullscreen' : 'ui.fullscreen'), 'F11', () => { closeMenu(); toggleFullscreen(); }),
        chipsRow(t('ui.size'), [
          [t('ui.size.s'), withKeys(t('ui.size.small'), 'Ctrl+1'), () => send('size:s')],
          [t('ui.size.m'), withKeys(t('ui.size.medium'), 'Ctrl+2'), () => send('size:m')],
          [t('ui.size.l'), withKeys(t('ui.size.large'), 'Ctrl+3'), () => send('size:l')],
        ]),
        chipsRow(t('ui.corner'), [
          ['↖', withKeys(t('ui.corner.top_left'), 'Ctrl+Shift+1'), () => send('corner:nw')],
          ['↗', withKeys(t('ui.corner.top_right'), 'Ctrl+Shift+2'), () => send('corner:ne')],
          ['↙', withKeys(t('ui.corner.bottom_left'), 'Ctrl+Shift+3'), () => send('corner:sw')],
          ['↘', withKeys(t('ui.corner.bottom_right'), 'Ctrl+Shift+4'), () => send('corner:se')],
        ]),
        h('div', { class: 'row' },
          h('span', { class: 'label', text: t('ui.opacity') }),
          h('span', { class: 'chips' },
            h('button', { type: 'button', class: 'chip', text: '−', title: t('ui.opacity_down'), 'aria-label': t('ui.opacity_down'), disabled: op <= 20 || null, onclick: () => send('opacity:' + Math.max(20, op - 10)) }),
            h('span', { class: 'value', text: percent(op) }),
            h('button', { type: 'button', class: 'chip', text: '+', title: t('ui.opacity_up'), 'aria-label': t('ui.opacity_up'), disabled: op >= 100 || null, onclick: () => send('opacity:' + Math.min(100, op + 10)) }))),
        toggleRow(t('ui.ghost'), 'Ctrl+Alt+G', state.ghost, () => send('ghost'), t('ui.ghost_sub')),
      ),
      section(t('ui.section.navigate'),
        !IS_HOME && actionRow(t('ui.main_menu'), 'Alt+{home}', () => { closeMenu(); goHome(); }),
        actionRow(t('ui.open_copied_link'), 'Ctrl+L', () => { closeMenu(); send('open-clipboard'); }),
        actionRow(t('ui.together'), null, () => { closeMenu(); send('together'); },
          state.together === 'host' ? t('ui.together_hosting') : state.together === 'guest' ? t('ui.together_watching') : null),
        !IS_HOME && actionRow(t('ui.reload_page'), 'F5', () => location.reload()),
      ),
      section(t('ui.section.app'),
        actionRow(t('ui.settings_all'), 'Ctrl+,', () => openSettings(), t('ui.settings_sub')),
        actionRow(t('ui.section.extensions'), null, () => openSettings('extensions'), extensionSummary()),
        ...languageRows(),
        state.overlay && h('div', { class: 'row' }, h('span', { class: 'label' }, t('ui.overlay_on'),
          h('span', { class: 'sub', text: t('ui.overlay_on_sub') }))),
        actionRow(t('ui.shortcuts'), 'F1', () => { closeMenu(); openHelp(); }),
        state.tray && actionRow(t('ui.hide_to_tray'), 'Ctrl+H', () => { closeMenu(); send('hide'); }),
        actionRow(t('ui.quit'), 'Ctrl+Q', () => send('quit')),
      ),
      h('div', { class: 'footer', text: 'YT Mini Player' + (state.version ? ' ' + state.version : '') }),
    );
  }

  // ---------------------------------------------------------------------------
  // Help
  // ---------------------------------------------------------------------------

  function table(rows) {
    return h('table', {}, h('tbody', {}, rows.map(([keys, label, off]) =>
      h('tr', { class: off ? 'off' : null }, h('td', {}, h('kbd', { text: keys })), h('td', { text: label })))));
  }

  function openHelp() {
    if (!help) return;
    helpOpen = true;
    closeMenu();
    const global = state.hotkeys.map((k) => [
      k.keys, k.available ? k.label : t('ui.help.unavailable', { label: k.label, reason: k.note || '' }), !k.available,
    ]);
    help.replaceChildren(h('div', { class: 'card', role: 'dialog', 'aria-label': t('ui.shortcuts') },
      h('h2', { text: t('ui.shortcuts') }),
      h('h3', { text: t('ui.help.window') }),
      table(SHORTCUTS.map((s) => [keyText(s.keys), t(s.label)])),
      global.length ? h('h3', { text: t('ui.help.global') }) : null,
      global.length ? table(global) : null,
      h('h3', { text: t('ui.help.mouse') }),
      table(GESTURES.map(([what, does]) => [t(what), t(does)])),
      h('p', { class: 'note', text: t('ui.help.youtube_keys') }),
    ));
    help.hidden = false;
    reportOverlay();
  }

  function closeHelp() {
    if (!helpOpen) return;
    helpOpen = false;
    help.hidden = true;
    reportOverlay();
  }

  function toggleHelp() {
    if (helpOpen) closeHelp();
    else openHelp();
  }

  // ---------------------------------------------------------------------------
  // Notices
  // ---------------------------------------------------------------------------

  function toast(message) {
    mount();
    if (!toasts) return;
    const el = h('div', { class: 'toast', text: String(message) });
    toasts.append(el);
    while (toasts.children.length > 3) toasts.firstChild.remove();
    // Long notices stay longer (about 60 ms a character, up to 15 s).
    setTimeout(() => {
      el.classList.add('out');
      setTimeout(() => el.remove(), 320);
    }, Math.min(15000, Math.max(TOAST_MS, String(message).length * 60)));
  }

  // ---------------------------------------------------------------------------
  // Host state
  // ---------------------------------------------------------------------------

  function render() {
    if (!bar) return;
    for (const [key, [label, keys]] of Object.entries(BAR_TITLES)) {
      setTitle(barButtons[key], t(label), keys ? keyText(keys) : null);
    }
    barButtons.pinned.classList.toggle('on', !!state.pinned);
    barButtons.videoOnly.classList.toggle('on', filling());
    barButtons.hide.hidden = !state.tray;
    // Each button only where it makes sense.
    barButtons.home.hidden = IS_HOME;
    barButtons.back.hidden = IS_HOME;
    barButtons.fullscreen.hidden = IS_HOME;
    barButtons.videoOnly.hidden = IS_APP_PAGE;
    const fs = !!document.fullscreenElement;
    barButtons.fullscreen.replaceChildren(fs ? ICONS.exitFullscreen() : ICONS.fullscreen());
    setTitle(barButtons.fullscreen, t(fs ? 'ui.exit_fullscreen' : 'ui.fullscreen'), 'F11');
    if (state.ghost) bar.classList.remove('show');
    if (menuOpen) renderMenu();
  }

  // Only the app can call these: the first comparison (strict, without
  // running any of the page's code) drops every other call.
  function sync(key, next) {
    if (key !== __ytmpHostKey) return;
    state = Object.assign({}, state, next);
    applyVideoOnly();
    render();
    // The start screen listens to this to show the settings. Other sites
    // are told nothing (installed extensions, settings...).
    if (IS_HOME) window.dispatchEvent(new CustomEvent('ytmp:state', { detail: state }));
  }

  function hostToast(key, message) {
    if (key !== __ytmpHostKey) return;
    toast(message);
  }

  // "Keyboard shortcuts" in Settings opens this help in the player.
  function hostHelp(key) {
    if (key !== __ytmpHostKey) return;
    mount();
    openHelp();
  }

  let hintPending = false;

  // Strings in the app's language: this script's (`ui`) and, on the app's
  // own pages, the page's (`page`), which it gets as "ytmp:locale".
  function locale(key, next) {
    if (key !== __ytmpHostKey || !next) return;
    I18N = { lang: next.lang, languages: next.languages || [], strings: next.ui || {} };
    localeReady = true;
    render();
    if (helpOpen) openHelp();
    if (hintPending) {
      hintPending = false;
      toast(t('ui.first_hint'));
    }
    if (IS_APP_PAGE && next.page) {
      window.dispatchEvent(new CustomEvent('ytmp:locale', {
        detail: { lang: next.lang, languages: next.languages || [], strings: next.page },
      }));
    }
  }

  document.addEventListener('fullscreenchange', () => {
    const el = document.fullscreenElement;
    if (el && state.fullscreenInWindow && !IS_APP_PAGE && performance.now() > screenUntil
      && (!IS_YOUTUBE || isWatch())) {
      // Nothing goes to the app: the window keeps its size.
      fullscreenToWindow(el);
      return;
    }
    if (el) screenUntil = 0;
    applyVideoOnly();
    placeHost();
    render();
    send('fullscreen:' + (document.fullscreenElement ? 1 : 0));
  });

  // YouTube's internal navigations (they do not reload the document).
  for (const type of ['yt-navigate-finish', 'popstate']) {
    window.addEventListener(type, () => { mount(); placeHost(); applyVideoOnly(); if (menuOpen) renderMenu(); }, true);
  }

  // --- Self-check: does the page still have what the app relies on? ----------
  //
  // Services change their pages without notice. On YouTube's watch page the
  // app needs the player (video only, full screen, dragging), its API (the
  // music bar, watch together) and its full-screen button; on Google's
  // sign-in, a refusal is worth telling apart from a wrong password. The
  // result goes to the app ("page-check:<json>"), which notes it in the
  // session diary and warns once if something is missing. Nothing about
  // what is watched is sent: only which pieces were found.
  const CHECK_AFTER_MS = 10000;
  let checkTimer = 0;
  let checkSent = '';

  function pageCheck() {
    if (IS_APP_PAGE) return null;
    if (/(^|\.)accounts\.google\.com$/.test(location.hostname)) {
      return /\/signin\/rejected/.test(location.pathname) ? { site: 'google', page: 'sign-in-refused' } : null;
    }
    if (!isWatch()) return null;
    const { el, how } = findPlayer();
    return {
      site: 'youtube',
      page: 'watch',
      player: how,
      api: !!el && typeof el.getPlayerState === 'function',
      video: !!el && !!el.querySelector('video'),
      fullscreenButton: !!el && !!el.querySelector('.ytp-fullscreen-button'),
    };
  }

  function runPageCheck() {
    clearTimeout(checkTimer);
    const result = pageCheck();
    if (!result) return;
    const json = JSON.stringify(result);
    if (json === checkSent) return;
    checkSent = json;
    send('page-check:' + json);
  }

  // Once the page has settled: when a video starts playing, or after a while.
  function schedulePageCheck() {
    clearTimeout(checkTimer);
    checkTimer = setTimeout(runPageCheck, CHECK_AFTER_MS);
  }
  document.addEventListener('playing', () => { if (isWatch()) setTimeout(runPageCheck, 500); }, true);
  window.addEventListener('yt-navigate-finish', schedulePageCheck, true);

  Object.defineProperty(window, '__ytmpSync', { value: sync });
  Object.defineProperty(window, '__ytmpToast', { value: hostToast });
  Object.defineProperty(window, '__ytmpHelp', { value: hostHelp });
  Object.defineProperty(window, '__ytmpLocale', { value: locale });

  // The page's size in CSS px: a guest using a shared window clicks on a
  // picture of it, and the app places that click here (remote.rs).
  let viewportTimer = 0;
  let viewportSent = '';
  function reportViewport() {
    clearTimeout(viewportTimer);
    viewportTimer = setTimeout(() => {
      const size = Math.round(window.innerWidth) + 'x' + Math.round(window.innerHeight);
      if (size !== viewportSent && window.innerWidth > 0 && window.innerHeight > 0) {
        viewportSent = size;
        send('viewport:' + size);
      }
    }, 150);
  }
  window.addEventListener('resize', reportViewport, { passive: true });

  function start() {
    mount();
    applyVideoOnly();
    send('ready');
    reportViewport();
    schedulePageCheck();
    // A usage hint the first time (once the strings have arrived).
    try {
      if (IS_YOUTUBE && !localStorage.getItem('ytmp-hint-v2')) {
        localStorage.setItem('ytmp-hint-v2', '1');
        showBar();
        scheduleHide(4000);
        if (localeReady) toast(t('ui.first_hint'));
        else hintPending = true;
      }
    } catch (_) { /* storage not available */ }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
