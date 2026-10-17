// Lectura y control de la reproducción, inyectado en cada sitio (ver
// webview.rs). Permite manejar el video con la app oculta: el mini
// controlador, la bandeja y los atajos globales llegan aquí.
//
// Para cada acción se usa, en este orden:
// 1. Los controles que el propio sitio declara al navegador con la Media
//    Session API (YouTube, Netflix, Crunchyroll... registran "siguiente",
//    "anterior", "avanzar", etc.). Este script se ejecuta antes que la
//    página y anota esos controles, así puede pulsarlos igual que lo haría
//    el sistema.
// 2. La API del reproductor de YouTube (#movie_player).
// 3. El elemento <video>/<audio> directamente.
//
// La página informa a la app con "media-state:<json>" cuando algo cambia;
// la posición solo se envía cada segundo mientras la barra de música está
// a la vista (__ytmpMediaWatch).
(() => {
  'use strict';

  if (window.top !== window || window.__ytmpMediaInstalled) return;
  Object.defineProperty(window, '__ytmpMediaInstalled', { value: true });
  // En las páginas propias, solo "Mi música" reproduce algo.
  if (location.hostname === 'ytmp.localhost' && location.pathname !== '/music') return;

  // Netflix interrumpe la reproducción (error M7375) si se cambia
  // currentTime desde fuera; ahí solo se busca con sus propios controles.
  const NO_DIRECT_SEEK = /(^|\.)netflix\.com$/.test(location.hostname);
  const POLL_MS = 2000;
  const WATCH_MS = 1000;

  // Canal autenticado con la app (token de sesión, ver webview.rs).
  const send = (message) => __ytmpSend(message);

  // --- 1. Controles declarados por el sitio --------------------------------

  const handlers = new Map();
  try {
    const proto = window.MediaSession && window.MediaSession.prototype;
    const original = proto && proto.setActionHandler;
    if (typeof original === 'function') {
      Object.defineProperty(proto, 'setActionHandler', {
        configurable: true,
        writable: true,
        value: function setActionHandler(action, handler) {
          if (typeof handler === 'function') handlers.set(action, handler);
          else handlers.delete(action);
          return original.call(this, action, handler);
        },
      });
    }
  } catch (_) { /* sin Media Session: se usan los otros caminos */ }

  function runHandler(action, details) {
    const handler = handlers.get(action);
    if (!handler) return false;
    try {
      handler(Object.assign({ action }, details || {}));
      return true;
    } catch (_) {
      return false;
    }
  }

  // --- 2 y 3. Reproductor de YouTube y elemento de medios --------------------

  function ytPlayer() {
    const player = document.getElementById('movie_player');
    return player && typeof player.getPlayerState === 'function' ? player : null;
  }

  // El medio "principal": el que suena o, si ninguno, el más grande.
  function mainMedia() {
    let best = null;
    let bestScore = -1;
    for (const media of document.querySelectorAll('video, audio')) {
      if (!media.currentSrc && !media.srcObject) continue;
      const rect = media.getBoundingClientRect();
      const score = (media.paused ? 0 : 1e9) + rect.width * rect.height + (media.duration > 0 ? 1 : 0);
      if (score > bestScore) {
        best = media;
        bestScore = score;
      }
    }
    return best;
  }

  const finite = (n, fallback = 0) => (Number.isFinite(n) ? n : fallback);
  const clamp01 = (n) => Math.min(1, Math.max(0, n));

  // Miniatura: la más grande de las que declara el sitio, sin pasar de 640px.
  function pickArtwork(artwork) {
    let best = '';
    let bestWidth = -1;
    for (const item of artwork || []) {
      if (!item || typeof item.src !== 'string') continue;
      const width = Math.max(0, ...String(item.sizes || '').split(/\s+/).map((s) => parseInt(s, 10) || 0));
      const score = width <= 640 ? width : 640 - (width - 640) / 10;
      if (score > bestWidth) {
        best = item.src;
        bestWidth = score;
      }
    }
    try {
      return best ? new URL(best, location.href).href : '';
    } catch (_) {
      return '';
    }
  }

  function youtubeThumbnail() {
    try {
      const id = new URL(location.href).searchParams.get('v');
      return id && /^[\w-]{11}$/.test(id) ? `https://i.ytimg.com/vi/${id}/mqdefault.jpg` : '';
    } catch (_) {
      return '';
    }
  }

  function readState() {
    const media = mainMedia();
    const yt = ytPlayer();
    const meta = navigator.mediaSession && navigator.mediaSession.metadata;
    const duration = media ? finite(media.duration) : 0;
    const live = media ? media.duration === Infinity : false;

    let title = (meta && meta.title) || '';
    if (!title && media) title = document.title.replace(/\s+[-|–]\s+(YouTube Music|YouTube|Netflix|Crunchyroll|Twitch)$/i, '');
    const artist = (meta && (meta.artist || meta.album)) || '';
    const artwork = pickArtwork(meta && meta.artwork) || (yt ? youtubeThumbnail() : '');

    let volume = media ? media.volume : 1;
    let muted = media ? media.muted : false;
    if (yt && typeof yt.getVolume === 'function') {
      volume = finite(yt.getVolume(), 100) / 100;
      muted = !!yt.isMuted();
    }

    return {
      hasMedia: !!media,
      title,
      artist,
      artwork,
      playing: !!media && !media.paused && !media.ended,
      position: media ? finite(media.currentTime) : 0,
      duration,
      live,
      volume: clamp01(volume),
      muted,
      canNext: handlers.has('nexttrack') || !!(yt && typeof yt.nextVideo === 'function'),
      canPrevious: handlers.has('previoustrack') || !!yt || (!!media && duration > 0 && !NO_DIRECT_SEEK),
      canSeek: !!media && duration > 0 && !live
        && (handlers.has('seekto') || handlers.has('seekforward') || !!yt || !NO_DIRECT_SEEK),
      site: location.hostname,
      watching,
    };
  }

  // --- Informe a la app -------------------------------------------------------

  let lastSent = '';
  let watching = false;

  // La posición solo cuenta como cambio mientras alguien mira el progreso.
  const keyFor = (state) => JSON.stringify(Object.assign({}, state, { position: watching ? Math.floor(state.position) : 0 }));

  function report(force) {
    const state = readState();
    const key = keyFor(state);
    if (!force && key === lastSent) return;
    lastSent = key;
    send('media-state:' + JSON.stringify(state));
  }

  let soon = 0;
  function reportSoon() {
    clearTimeout(soon);
    soon = setTimeout(() => report(false), 120);
  }

  // Los eventos de medios no burbujean, pero pasan por la fase de captura.
  for (const type of ['play', 'pause', 'ended', 'emptied', 'loadedmetadata', 'durationchange', 'volumechange', 'seeked']) {
    document.addEventListener(type, reportSoon, true);
  }
  // Metadatos de la Media Session y cambios sin evento (navegación interna).
  setInterval(() => report(false), POLL_MS);
  let watchTimer = 0;

  // --- Acciones -----------------------------------------------------------------

  function setVolume(volume) {
    const v = clamp01(volume);
    const yt = ytPlayer();
    if (yt && typeof yt.setVolume === 'function') {
      yt.setVolume(Math.round(v * 100));
      if (v > 0 && yt.isMuted()) yt.unMute();
      return;
    }
    const media = mainMedia();
    if (!media) return;
    media.volume = v;
    if (v > 0) media.muted = false;
  }

  function seekTo(seconds) {
    const media = mainMedia();
    if (!media) return;
    const target = Math.max(0, Math.min(finite(seconds), finite(media.duration, seconds)));
    if (runHandler('seekto', { seekTime: target, fastSeek: false })) return;
    const yt = ytPlayer();
    if (yt && typeof yt.seekTo === 'function') {
      yt.seekTo(target, true);
      return;
    }
    if (!NO_DIRECT_SEEK) media.currentTime = target;
  }

  function seekBy(delta) {
    const media = mainMedia();
    if (!media) return;
    const action = delta >= 0 ? 'seekforward' : 'seekbackward';
    if (runHandler(action, { seekOffset: Math.abs(delta) })) return;
    seekTo(media.currentTime + delta);
  }

  const actions = {
    toggle() {
      const media = mainMedia();
      const yt = ytPlayer();
      // Con YouTube manda el estado de su reproductor (1 = reproduciendo).
      const playing = yt ? yt.getPlayerState() === 1 : !!media && !media.paused && !media.ended;
      if (runHandler(playing ? 'pause' : 'play')) return;
      if (yt) {
        if (playing) yt.pauseVideo(); else yt.playVideo();
        return;
      }
      if (!media) return;
      if (playing) media.pause(); else media.play().catch(() => {});
    },
    // Al bloquear la app: pausar si suena (nunca reanuda).
    pause() {
      const media = mainMedia();
      const yt = ytPlayer();
      const playing = yt ? yt.getPlayerState() === 1 : !!media && !media.paused && !media.ended;
      if (!playing) return;
      if (runHandler('pause')) return;
      if (yt) yt.pauseVideo();
      else if (media) media.pause();
    },
    next() {
      if (runHandler('nexttrack')) return;
      const yt = ytPlayer();
      if (yt && typeof yt.nextVideo === 'function') yt.nextVideo();
    },
    previous() {
      if (runHandler('previoustrack')) return;
      const yt = ytPlayer();
      if (yt && typeof yt.previousVideo === 'function') {
        yt.previousVideo();
        return;
      }
      seekTo(0);
    },
    'seek-by': (seconds) => seekBy(finite(Number(seconds))),
    'seek-to': (seconds) => seekTo(finite(Number(seconds))),
    volume: (value) => setVolume(finite(Number(value), 1)),
    'volume-by': (delta) => {
      const state = readState();
      setVolume((state.muted ? 0 : state.volume) + finite(Number(delta)));
    },
    mute() {
      const yt = ytPlayer();
      if (yt && typeof yt.isMuted === 'function') {
        if (yt.isMuted()) yt.unMute(); else yt.mute();
        return;
      }
      const media = mainMedia();
      if (media) media.muted = !media.muted;
    },
  };

  function run(action, arg) {
    const fn = Object.prototype.hasOwnProperty.call(actions, action) ? actions[action] : null;
    if (!fn) return;
    try { fn(arg); } catch (_) { /* el sitio cambió: no romper nada */ }
    reportSoon();
  }

  function watch(on) {
    watching = !!on;
    clearInterval(watchTimer);
    if (watching) {
      watchTimer = setInterval(() => report(false), WATCH_MS);
      report(true);
    } else {
      // Nueva referencia sin posición: no hay nada nuevo que contar.
      lastSent = keyFor(readState());
    }
  }

  Object.defineProperty(window, '__ytmpMedia', { value: run });
  Object.defineProperty(window, '__ytmpMediaWatch', { value: watch });
  Object.defineProperty(window, '__ytmpMediaReport', { value: () => report(true) });
})();
