// Interfaz propia del reproductor, inyectada por el host (ver webview.rs).
//
// - Barra lateral derecha que aparece al acercar el ratón al borde.
// - Menú "Más" con todas las opciones, extensiones y ajustes.
// - Ayuda de atajos (F1), avisos breves y atajos de teclado locales.
// - Modo "solo video" y sincronización de pantalla completa con el host.
//
// Restricciones de YouTube que condicionan el código:
// - Trusted Types: nada de innerHTML; todo se construye con el DOM.
// - Todo vive en un shadow DOM para no heredar ni romper estilos.
// - Los eventos de la barra no deben llegar al reproductor (pausaría).
(() => {
  'use strict';

  if (window.top !== window || window.__ytmpUi) return;
  Object.defineProperty(window, '__ytmpUi', { value: true });

  const IS_YOUTUBE = /(^|\.)youtube\.com$/.test(location.hostname);
  // Pantalla de inicio de la app (selector de servicios).
  const IS_HOME = location.hostname === 'ytmp.localhost';
  const REVEAL_ZONE = 64;   // px desde el borde derecho que muestran la barra
  const HIDE_DELAY = 1200;  // ms antes de ocultarla al alejarse
  const TOAST_MS = 3500;
  const SVG_NS = 'http://www.w3.org/2000/svg';

  // Canal autenticado con la app (token de sesión, ver webview.rs).
  const send = (message) => __ytmpSend(message);

  let state = {
    version: '', pinned: true, videoOnly: false, opacity: 100, ghost: false,
    fullscreen: false, autostart: false, tray: true, overlay: false,
    extensions: [], extensionProblems: [], extensionsError: null, hotkeys: [],
  };

  // ---------------------------------------------------------------------------
  // Utilidades DOM (sin innerHTML)
  // ---------------------------------------------------------------------------

  function h(tag, props, ...children) {
    const el = document.createElement(tag);
    for (const [key, value] of Object.entries(props || {})) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') el.className = value;
      else if (key === 'text') el.textContent = value;
      // Solo eventos reales del usuario: la página no puede pulsar
      // nuestros botones con eventos sintéticos.
      else if (key.startsWith('on')) el.addEventListener(key.slice(2), (e) => { if (e.isTrusted) value(e); });
      else el.setAttribute(key, value === true ? '' : String(value));
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      el.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return el;
  }

  // Iconos de 24×24 con trazo; `paths` son atributos `d`, `dots` círculos.
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
  };

  // ---------------------------------------------------------------------------
  // Estilos
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
    .root {
      --bg: rgba(18, 18, 20, 0.88);
      --bg-solid: #18181b;
      --line: rgba(255, 255, 255, 0.10);
      --text: #f1f1f1;
      --muted: #a8a8ad;
      --accent: #8fa6ff;
      --accent-bg: rgba(108, 140, 255, 0.22);
      --danger: #ff7b7b;
      font: 13px/1.35 "Roboto", "Segoe UI", system-ui, sans-serif;
      color: var(--text);
      -webkit-font-smoothing: antialiased;
    }
    svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 2;
          stroke-linecap: round; stroke-linejoin: round; display: block; }
    svg .fill { fill: currentColor; stroke: none; }

    .bar {
      position: fixed; top: 50%; right: 10px; z-index: 2147483647;
      display: flex; flex-direction: column; gap: 2px; padding: 5px;
      background: var(--bg); border: 1px solid var(--line); border-radius: 12px;
      backdrop-filter: blur(14px); box-shadow: 0 8px 28px rgba(0,0,0,.45);
      transform: translate(calc(100% + 16px), -50%); opacity: 0;
      transition: transform .18s ease, opacity .18s ease;
      pointer-events: none;
    }
    .bar.show { transform: translate(0, -50%); opacity: 1; pointer-events: auto; }
    .bar .sep { height: 1px; margin: 3px 4px; background: var(--line); }
    @media (max-height: 300px) { .bar [data-secondary] { display: none; } }
    @media (max-height: 190px) { .bar .sep { display: none; } }

    button { font: inherit; color: inherit; background: none; border: 0; margin: 0; cursor: pointer; }
    button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
    .bar button {
      width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center;
      color: #e8e8ea;
    }
    .bar button[hidden] { display: none; }
    .bar button:hover { background: rgba(255,255,255,.10); }
    .bar button.on { background: var(--accent-bg); color: var(--accent); }
    .bar button.grip { cursor: grab; color: var(--muted); }
    .bar button.danger:hover { background: rgba(255, 90, 90, .18); color: var(--danger); }

    .panel {
      position: fixed; z-index: 2147483647; top: 8px; right: 58px;
      width: 300px; max-width: calc(100vw - 16px); max-height: calc(100vh - 16px); overflow: auto;
      background: var(--bg-solid); border: 1px solid var(--line); border-radius: 12px;
      box-shadow: 0 12px 36px rgba(0,0,0,.55); padding: 6px 0;
      scrollbar-width: thin; scrollbar-color: #444 transparent;
    }
    .panel[hidden] { display: none; }
    @media (max-width: 380px) { .panel { left: 8px; width: auto; } }
    @media (max-width: 360px) { .panel kbd { display: none; } }
    .section { padding: 6px 0; border-top: 1px solid var(--line); }
    .section:first-child { border-top: 0; }
    .section h3 {
      margin: 2px 14px 4px; font-size: 11px; font-weight: 600; letter-spacing: .06em;
      text-transform: uppercase; color: var(--muted);
    }
    .row {
      display: flex; align-items: center; gap: 10px; width: 100%; min-height: 32px;
      padding: 4px 14px; text-align: left;
    }
    button.row:hover { background: rgba(255,255,255,.07); }
    .row .label { flex: 1; min-width: 0; }
    .row .sub { display: block; color: var(--muted); font-size: 11.5px; overflow-wrap: anywhere; }
    .row .sub.error { color: var(--danger); }
    kbd {
      font: 11px/1 ui-monospace, "Cascadia Mono", Consolas, monospace; color: var(--muted);
      border: 1px solid var(--line); border-radius: 4px; padding: 3px 5px; white-space: nowrap;
    }
    .switch {
      flex: none; width: 30px; height: 18px; border-radius: 9px; background: #4a4a50; position: relative;
      transition: background .15s;
    }
    .switch::after {
      content: ""; position: absolute; top: 2px; left: 2px; width: 14px; height: 14px; border-radius: 50%;
      background: #fff; transition: transform .15s;
    }
    .switch.on { background: #6c8cff; }
    .switch.on::after { transform: translateX(12px); }
    .chips { display: flex; gap: 4px; }
    .chip {
      min-width: 30px; height: 26px; padding: 0 8px; border-radius: 6px; border: 1px solid var(--line);
      display: grid; place-items: center; font-size: 13px; line-height: 1;
    }
    .chip:hover { background: rgba(255,255,255,.08); }
    .value { min-width: 44px; text-align: center; font-variant-numeric: tabular-nums; }
    .hint { margin: 2px 14px 6px; color: var(--muted); font-size: 12px; }
    .actions { display: flex; gap: 6px; padding: 4px 14px 2px; flex-wrap: wrap; }
    .actions .chip { padding: 0 10px; }
    .link { color: var(--accent); font-size: 12px; padding: 2px 4px; border-radius: 4px; }
    .link:hover { background: var(--accent-bg); }
    .footer { padding: 6px 14px 2px; color: var(--muted); font-size: 11px; }

    .help {
      position: fixed; inset: 0; z-index: 2147483647; display: grid; place-items: center;
      background: rgba(0,0,0,.55); padding: 12px;
    }
    .help[hidden] { display: none; }
    .help .card {
      width: min(560px, 100%); max-height: 100%; overflow: auto; background: var(--bg-solid);
      border: 1px solid var(--line); border-radius: 14px; padding: 14px 18px 16px;
      box-shadow: 0 16px 48px rgba(0,0,0,.6);
    }
    .help h2 { margin: 0 0 4px; font-size: 16px; font-weight: 600; }
    .help h3 { margin: 14px 0 6px; font-size: 11px; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }
    .help table { width: 100%; border-collapse: collapse; }
    .help td { padding: 4px 0; vertical-align: top; }
    .help td:first-child { width: 42%; padding-right: 12px; }
    .help .off { color: var(--muted); text-decoration: line-through; }
    .help .note { color: var(--muted); font-size: 12px; margin-top: 10px; }

    .toasts {
      position: fixed; left: 50%; bottom: 18px; transform: translateX(-50%); z-index: 2147483647;
      display: flex; flex-direction: column; align-items: center; gap: 6px; pointer-events: none;
      width: max-content; max-width: calc(100vw - 120px);
    }
    .toast {
      background: rgba(18,18,20,.94); border: 1px solid var(--line); border-radius: 10px;
      padding: 8px 14px; box-shadow: 0 6px 20px rgba(0,0,0,.45);
      animation: in .18s ease; text-align: center;
    }
    .toast.out { opacity: 0; transition: opacity .3s; }
    @keyframes in { from { opacity: 0; transform: translateY(6px); } }
    @media (prefers-reduced-motion: reduce) {
      .bar, .toast, .switch, .switch::after { transition: none; animation: none; }
    }
  `;

  // Modo solo video: el reproductor ocupa toda la ventana en /watch.
  //
  // No se puede confiar en z-index: YouTube crea varios contextos de
  // apilamiento entre el reproductor y la raíz, y su contenido se pintaría
  // encima. En su lugar, todo lo que no es el reproductor se vuelve
  // invisible (visibility no pinta ni recibe clics) y el reproductor se fija
  // a la ventana. Los antepasados con transform/filter/contain, que
  // cambiarían a qué se refiere "fixed", se neutralizan desde JS.
  const VO_CLASS = 'ytmp-video-only';
  const VO_CHAIN = 'ytmp-vo-chain';
  const PLAYER = '#movie_player';
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
      z-index: 2147483000 !important; background: #000 !important;
    }
    html.${VO_CLASS} ${PLAYER} .html5-video-container { width: 100% !important; height: 100% !important; }
    html.${VO_CLASS} ${PLAYER} video.html5-main-video {
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

  // Marca los antepasados del reproductor que convertirían "fixed" en
  // relativo a ellos (transform, filter, contain...). Se recalcula cada vez
  // porque YouTube mueve el reproductor entre contenedores (cine, normal).
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
    const player = document.querySelector(PLAYER);
    // En pantalla completa manda YouTube: su propio diseño ya muestra solo
    // el video, y mezclar ambos rompe la página.
    const on = !!state.videoOnly && isWatch() && !!player && !document.fullscreenElement;
    if (on) markPlayerChain(player);
    if (root.classList.contains(VO_CLASS) === on) return;
    root.classList.toggle(VO_CLASS, on);
    if (!on) for (const el of document.querySelectorAll('.' + VO_CHAIN)) el.classList.remove(VO_CHAIN);
    // Que el reproductor recalcule el tamaño de sus controles.
    requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
  }

  // ---------------------------------------------------------------------------
  // Acciones de página
  // ---------------------------------------------------------------------------

  function toggleFullscreen() {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
      return;
    }
    // Con el botón de YouTube su interfaz queda coherente.
    const button = isWatch() && document.querySelector('#movie_player .ytp-fullscreen-button');
    if (button) button.click();
    else document.documentElement.requestFullscreen().catch(() => {});
  }

  // Menú principal (selector de servicios).
  function goHome() {
    send('home');
  }

  // ---------------------------------------------------------------------------
  // Atajos locales (con la ventana enfocada)
  // ---------------------------------------------------------------------------

  const onlyCtrl = (e) => e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
  const noMods = (e) => !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
  const letter = (e, l) => (e.key || '').toLowerCase() === l;

  // `run` ausente = atajo nativo, solo se muestra en la ayuda.
  const SHORTCUTS = [
    { keys: 'F11', label: 'Pantalla completa', match: (e) => e.key === 'F11' && noMods(e), run: toggleFullscreen },
    { keys: 'Ctrl+T', label: 'Siempre visible', match: (e) => onlyCtrl(e) && letter(e, 't'), run: () => send('pin') },
    { keys: 'Ctrl+M', label: 'Solo video (YouTube)', match: (e) => IS_YOUTUBE && onlyCtrl(e) && letter(e, 'm'), run: () => send('video-only') },
    { keys: 'Ctrl+H', label: 'Ocultar en la bandeja', match: (e) => onlyCtrl(e) && letter(e, 'h'), run: () => send('hide') },
    { keys: 'Ctrl+L', label: 'Abrir el enlace de YouTube copiado', match: (e) => onlyCtrl(e) && letter(e, 'l'), run: () => send('open-clipboard') },
    {
      keys: 'Ctrl+1 / 2 / 3', label: 'Tamaño pequeño / mediano / grande',
      match: (e) => onlyCtrl(e) && /^Digit[123]$/.test(e.code),
      run: (e) => send('size:' + { Digit1: 's', Digit2: 'm', Digit3: 'l' }[e.code]),
    },
    {
      keys: 'Ctrl+Shift+1 … 4', label: 'Llevar a una esquina (↖ ↗ ↙ ↘)',
      match: (e) => e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && /^Digit[1-4]$/.test(e.code),
      run: (e) => send('corner:' + { Digit1: 'nw', Digit2: 'ne', Digit3: 'sw', Digit4: 'se' }[e.code]),
    },
    { keys: 'Alt+Inicio', label: 'Menú principal (elegir servicio)', match: (e) => e.altKey && !e.ctrlKey && !e.shiftKey && e.key === 'Home', run: goHome },
    { keys: 'Alt+← / Alt+→', label: 'Atrás / adelante' },
    { keys: 'F5', label: 'Recargar la página' },
    { keys: 'F1', label: 'Esta ayuda', match: (e) => e.key === 'F1' && noMods(e), run: () => toggleHelp() },
    { keys: 'Ctrl+Q', label: 'Salir', match: (e) => onlyCtrl(e) && letter(e, 'q'), run: () => send('quit') },
    { keys: 'Esc', label: 'Cerrar el menú o la ayuda' },
  ];

  const GESTURES = [
    ['Arrastrar el video', 'Mover la ventana (un clic sin mover sigue pausando)'],
    ['Alt + arrastrar', 'Mover desde cualquier punto'],
    ['Bordes y esquinas', 'Cambiar el tamaño'],
    ['Borde derecho', 'Mostrar la barra de botones'],
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
    const shortcut = SHORTCUTS.find((s) => s.run && s.match(e));
    if (!shortcut) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    shortcut.run(e);
  }, { capture: true });

  // ---------------------------------------------------------------------------
  // Construcción de la interfaz
  // ---------------------------------------------------------------------------

  let host = null, root = null, bar = null, panel = null, help = null, toasts = null;
  let menuOpen = false, helpOpen = false, barHover = false, hideTimer = 0;
  const barButtons = {};

  function barButton(key, title, iconFn, onClick, opts = {}) {
    const button = h('button', {
      type: 'button', title, 'aria-label': title,
      class: opts.class || null, 'data-secondary': opts.secondary || null,
      onclick: onClick,
    }, iconFn());
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

    // Nada de lo que pase dentro debe llegar al reproductor ni a YouTube.
    for (const type of ['click', 'dblclick', 'mousedown', 'mouseup', 'pointerdown', 'pointerup',
      'keydown', 'keyup', 'wheel', 'contextmenu']) {
      shadow.addEventListener(type, (e) => e.stopPropagation());
    }

    root = h('div', { class: 'root' });

    const grip = barButton('grip', 'Mover (también puedes arrastrar el video)', ICONS.grip, null, { class: 'grip' });
    grip.addEventListener('pointerdown', (e) => {
      if (!e.isTrusted || e.button !== 0) return;
      e.preventDefault();
      if (typeof __ytmpShared.startDrag === 'function') __ytmpShared.startDrag();
      else send('drag');
    });

    bar = h('div', { class: 'bar', role: 'toolbar', 'aria-label': 'YT Mini Player' },
      grip,
      barButton('home', 'Menú principal (Alt+Inicio)', ICONS.home, goHome),
      barButton('back', 'Atrás (Alt+←)', ICONS.back, () => history.back(), { secondary: true }),
      h('div', { class: 'sep', 'data-secondary': true }),
      barButton('videoOnly', 'Solo video (Ctrl+M)', ICONS.videoOnly, () => send('video-only'), { secondary: true }),
      barButton('fullscreen', 'Pantalla completa (F11)', ICONS.fullscreen, toggleFullscreen),
      barButton('pinned', 'Siempre visible (Ctrl+T)', ICONS.pin, () => send('pin'), { secondary: true }),
      barButton('hide', 'Ocultar en la bandeja (Ctrl+H)', ICONS.hide, () => send('hide'), { secondary: true }),
      h('div', { class: 'sep' }),
      barButton('more', 'Más opciones', ICONS.more, () => (menuOpen ? closeMenu() : openMenu())),
      barButton('quit', 'Cerrar (Ctrl+Q)', ICONS.close, () => send('quit'), { class: 'danger' }),
    );
    bar.addEventListener('pointerenter', () => { barHover = true; showBar(); });
    bar.addEventListener('pointerleave', () => { barHover = false; scheduleHide(); });

    panel = h('div', { class: 'panel', hidden: true, role: 'menu' });
    help = h('div', { class: 'help', hidden: true, onclick: (e) => { if (e.target === help) closeHelp(); } });
    toasts = h('div', { class: 'toasts', role: 'status', 'aria-live': 'polite' });

    root.append(bar, panel, help, toasts);
    shadow.append(root);
    placeHost();
    render();
  }

  // En pantalla completa solo se pinta el elemento a pantalla completa: la
  // interfaz se mueve dentro de él para seguir disponible.
  function placeHost() {
    if (!host) return;
    const parent = document.fullscreenElement || document.documentElement;
    if (host.parentNode !== parent) parent.appendChild(host);
  }

  // ---------------------------------------------------------------------------
  // Barra: mostrar / ocultar
  // ---------------------------------------------------------------------------

  function showBar() {
    if (!bar || state.ghost) return;
    clearTimeout(hideTimer);
    bar.classList.add('show');
  }

  function scheduleHide(delay = HIDE_DELAY) {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!barHover && !menuOpen) bar && bar.classList.remove('show');
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
  // Menú "Más"
  // ---------------------------------------------------------------------------

  function openMenu() {
    menuOpen = true;
    closeHelp();
    renderMenu();
    panel.hidden = false;
    showBar();
    barButtons.more.classList.add('on');
  }

  function closeMenu() {
    if (!menuOpen) return;
    menuOpen = false;
    panel.hidden = true;
    barButtons.more.classList.remove('on');
    scheduleHide();
  }

  // Cerrar al hacer clic fuera (la página no recibe ese clic de más).
  window.addEventListener('pointerdown', (e) => {
    if (menuOpen && e.target !== host) closeMenu();
  }, { capture: true });

  function section(title, ...rows) {
    return h('div', { class: 'section' }, h('h3', { text: title }), ...rows);
  }

  function actionRow(label, keys, onClick, sub = null) {
    return h('button', { type: 'button', class: 'row', role: 'menuitem', onclick: onClick },
      h('span', { class: 'label' }, label, sub && h('span', { class: 'sub', text: sub })),
      keys && h('kbd', { text: keys }));
  }

  function toggleRow(label, keys, on, onClick, sub = null) {
    return h('button', { type: 'button', class: 'row', role: 'menuitemcheckbox', 'aria-checked': String(!!on), onclick: onClick },
      h('span', { class: 'label' }, label, sub && h('span', { class: 'sub', text: sub })),
      keys && h('kbd', { text: keys }),
      h('span', { class: 'switch' + (on ? ' on' : '') }));
  }

  function chipsRow(label, chips) {
    return h('div', { class: 'row' },
      h('span', { class: 'label', text: label }),
      h('span', { class: 'chips' }, chips.map(([text, title, onClick]) =>
        h('button', { type: 'button', class: 'chip', title, 'aria-label': title, onclick: onClick, text }))));
  }

  function extensionRows() {
    const rows = [];
    if (state.extensionsError) {
      rows.push(h('p', { class: 'hint', text: state.extensionsError }));
    }
    for (const ext of state.extensions) {
      let sub = ext.version ? 'v' + ext.version : '';
      let error = false;
      if (ext.error) { sub = ext.error; error = true; }
      else if (ext.pending) sub = 'Instalando…';
      const label = h('span', { class: 'label' }, ext.name,
        sub && h('span', { class: 'sub' + (error ? ' error' : ''), text: sub }));
      const options = ext.id && ext.hasPage && !ext.error
        ? h('button', { type: 'button', class: 'link', text: 'Opciones', onclick: (e) => { e.stopPropagation(); closeMenu(); send('ext-options:' + ext.id); } })
        : null;
      if (ext.id && !ext.error) {
        rows.push(h('div', { class: 'row' }, label, options,
          h('button', {
            type: 'button', class: 'switch' + (ext.enabled ? ' on' : ''), role: 'switch',
            'aria-checked': String(!!ext.enabled), title: ext.enabled ? 'Desactivar' : 'Activar',
            'aria-label': (ext.enabled ? 'Desactivar ' : 'Activar ') + ext.name,
            onclick: () => send('ext-toggle:' + ext.id),
          })));
      } else {
        rows.push(h('div', { class: 'row' }, label));
      }
    }
    for (const problem of state.extensionProblems) {
      rows.push(h('div', { class: 'row' }, h('span', { class: 'label' }, h('span', { class: 'sub error', text: problem }))));
    }
    if (!state.extensions.length && !state.extensionProblems.length && !state.extensionsError) {
      rows.push(h('p', { class: 'hint', text: 'Copia extensiones descomprimidas, .crx o .zip en la carpeta y pulsa Recargar.' }));
    }
    rows.push(h('div', { class: 'actions' },
      h('button', { type: 'button', class: 'chip', text: 'Abrir carpeta', onclick: () => send('ext-folder') }),
      h('button', { type: 'button', class: 'chip', text: 'Recargar', onclick: () => send('ext-reload') })));
    return rows;
  }

  function renderMenu() {
    if (!panel) return;
    const op = state.opacity;
    panel.replaceChildren(
      section('Ventana',
        toggleRow('Siempre visible', 'Ctrl+T', state.pinned, () => send('pin')),
        IS_YOUTUBE && toggleRow('Solo video', 'Ctrl+M', state.videoOnly, () => send('video-only'),
          isWatch() ? null : 'Se aplica al abrir un video'),
        actionRow(document.fullscreenElement ? 'Salir de pantalla completa' : 'Pantalla completa', 'F11',
          () => { closeMenu(); toggleFullscreen(); }),
        chipsRow('Tamaño', [
          ['S', 'Pequeño (Ctrl+1)', () => send('size:s')],
          ['M', 'Mediano (Ctrl+2)', () => send('size:m')],
          ['L', 'Grande (Ctrl+3)', () => send('size:l')],
        ]),
        chipsRow('Esquina', [
          ['↖', 'Arriba a la izquierda (Ctrl+Shift+1)', () => send('corner:nw')],
          ['↗', 'Arriba a la derecha (Ctrl+Shift+2)', () => send('corner:ne')],
          ['↙', 'Abajo a la izquierda (Ctrl+Shift+3)', () => send('corner:sw')],
          ['↘', 'Abajo a la derecha (Ctrl+Shift+4)', () => send('corner:se')],
        ]),
        h('div', { class: 'row' },
          h('span', { class: 'label', text: 'Opacidad' }),
          h('span', { class: 'chips' },
            h('button', { type: 'button', class: 'chip', text: '−', title: 'Menos opacidad', 'aria-label': 'Menos opacidad', disabled: op <= 20 || null, onclick: () => send('opacity:' + Math.max(20, op - 10)) }),
            h('span', { class: 'value', text: op + ' %' }),
            h('button', { type: 'button', class: 'chip', text: '+', title: 'Más opacidad', 'aria-label': 'Más opacidad', disabled: op >= 100 || null, onclick: () => send('opacity:' + Math.min(100, op + 10)) }))),
        toggleRow('Modo fantasma', 'Ctrl+Alt+G', state.ghost, () => send('ghost'),
          'Los clics atraviesan la ventana'),
      ),
      section('Navegar',
        !IS_HOME && actionRow('Menú principal', 'Alt+Inicio', () => { closeMenu(); goHome(); }),
        actionRow('Abrir enlace copiado', 'Ctrl+L', () => { closeMenu(); send('open-clipboard'); }),
        !IS_HOME && actionRow('Recargar la página', 'F5', () => location.reload()),
      ),
      section('Extensiones', ...extensionRows()),
      section('Aplicación',
        state.overlay && h('div', { class: 'row' }, h('span', { class: 'label' }, 'Superposición máxima activa',
          h('span', { class: 'sub', text: 'Con "Siempre visible", queda encima de apps a pantalla completa' }))),
        toggleRow('Iniciar con Windows', null, state.autostart, () => send('autostart'), 'Arranca oculto en la bandeja'),
        actionRow('Atajos de teclado', 'F1', () => { closeMenu(); openHelp(); }),
        state.tray && actionRow('Ocultar en la bandeja', 'Ctrl+H', () => { closeMenu(); send('hide'); }),
        actionRow('Salir', 'Ctrl+Q', () => send('quit')),
      ),
      h('div', { class: 'footer', text: 'YT Mini Player' + (state.version ? ' ' + state.version : '') }),
    );
  }

  // ---------------------------------------------------------------------------
  // Ayuda
  // ---------------------------------------------------------------------------

  function table(rows) {
    return h('table', {}, h('tbody', {}, rows.map(([keys, label, off]) =>
      h('tr', { class: off ? 'off' : null }, h('td', {}, h('kbd', { text: keys })), h('td', { text: label })))));
  }

  function openHelp() {
    if (!help) return;
    helpOpen = true;
    closeMenu();
    const global = state.hotkeys.map((k) => [k.keys, k.label + (k.available ? '' : ' (otro programa lo usa)'), !k.available]);
    help.replaceChildren(h('div', { class: 'card', role: 'dialog', 'aria-label': 'Atajos de teclado' },
      h('h2', { text: 'Atajos de teclado' }),
      h('h3', { text: 'Con la ventana activa' }),
      table(SHORTCUTS.map((s) => [s.keys, s.label])),
      global.length ? h('h3', { text: 'En todo Windows (aunque esté oculto)' }) : null,
      global.length ? table(global) : null,
      h('h3', { text: 'Con el ratón' }),
      table(GESTURES),
      h('p', { class: 'note', text: 'Los atajos propios de YouTube (espacio, K, J, L, F, M…) siguen funcionando.' }),
    ));
    help.hidden = false;
  }

  function closeHelp() {
    if (!helpOpen) return;
    helpOpen = false;
    help.hidden = true;
  }

  function toggleHelp() {
    if (helpOpen) closeHelp();
    else openHelp();
  }

  // ---------------------------------------------------------------------------
  // Avisos
  // ---------------------------------------------------------------------------

  function toast(message) {
    mount();
    if (!toasts) return;
    const el = h('div', { class: 'toast', text: String(message) });
    toasts.append(el);
    while (toasts.children.length > 3) toasts.firstChild.remove();
    setTimeout(() => {
      el.classList.add('out');
      setTimeout(() => el.remove(), 320);
    }, TOAST_MS);
  }

  // ---------------------------------------------------------------------------
  // Estado del host
  // ---------------------------------------------------------------------------

  function render() {
    if (!bar) return;
    barButtons.pinned.classList.toggle('on', !!state.pinned);
    barButtons.videoOnly.classList.toggle('on', !!state.videoOnly);
    barButtons.hide.hidden = !state.tray;
    // Cada botón solo donde tiene sentido.
    barButtons.home.hidden = IS_HOME;
    barButtons.back.hidden = IS_HOME;
    barButtons.fullscreen.hidden = IS_HOME;
    barButtons.videoOnly.hidden = !IS_YOUTUBE;
    const fs = !!document.fullscreenElement;
    barButtons.fullscreen.replaceChildren(fs ? ICONS.exitFullscreen() : ICONS.fullscreen());
    const fsTitle = fs ? 'Salir de pantalla completa (F11)' : 'Pantalla completa (F11)';
    barButtons.fullscreen.title = fsTitle;
    barButtons.fullscreen.setAttribute('aria-label', fsTitle);
    if (state.ghost) bar.classList.remove('show');
    if (menuOpen) renderMenu();
  }

  // Solo la app puede llamarlas: la primera comparación (estricta, sin
  // ejecutar código de la página) descarta cualquier otra llamada.
  function sync(key, next) {
    if (key !== __ytmpHostKey) return;
    state = Object.assign({}, state, next);
    applyVideoOnly();
    render();
    // La pantalla de inicio escucha esto para reflejar los ajustes. A otros
    // sitios no se les cuentan (extensiones instaladas, ajustes...).
    if (IS_HOME) window.dispatchEvent(new CustomEvent('ytmp:state', { detail: state }));
  }

  function hostToast(key, message) {
    if (key !== __ytmpHostKey) return;
    toast(message);
  }

  document.addEventListener('fullscreenchange', () => {
    applyVideoOnly();
    placeHost();
    render();
    send('fullscreen:' + (document.fullscreenElement ? 1 : 0));
  });

  // Navegaciones internas de YouTube (no recargan el documento).
  for (const type of ['yt-navigate-finish', 'popstate']) {
    window.addEventListener(type, () => { mount(); placeHost(); applyVideoOnly(); if (menuOpen) renderMenu(); }, true);
  }

  Object.defineProperty(window, '__ytmpSync', { value: sync });
  Object.defineProperty(window, '__ytmpToast', { value: hostToast });

  function start() {
    mount();
    applyVideoOnly();
    send('ready');
    // Pista de uso la primera vez.
    try {
      if (IS_YOUTUBE && !localStorage.getItem('ytmp-hint-v2')) {
        localStorage.setItem('ytmp-hint-v2', '1');
        showBar();
        scheduleHide(4000);
        toast('Arrastra el video para mover la ventana · F1: atajos');
      }
    } catch (_) { /* almacenamiento no disponible */ }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
