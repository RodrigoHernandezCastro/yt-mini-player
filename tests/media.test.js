// Pruebas de src/media.js con Media Session, reproductor de YouTube, video
// genérico y reloj simulados.
const fs = require('fs'), vm = require('vm'), path = require('path'), assert = require('assert');
const src = fs.readFileSync(path.join(__dirname, '..', 'src', 'media.js'), 'utf8');

function video(props = {}) {
  return {
    currentSrc: 'blob:x', srcObject: null, paused: true, ended: false, currentTime: 0, duration: 300,
    volume: 1, muted: false, plays: 0,
    getBoundingClientRect: () => ({ width: 640, height: 360 }),
    play() { this.paused = false; this.plays++; return Promise.resolve(); },
    pause() { this.paused = true; },
    ...props,
  };
}

function env({ host = 'www.example.com', medias = [], yt = null, title = 'Página' } = {}) {
  const sent = [], timers = [], listeners = {};
  let now = 0;
  const originalCalls = [];
  class MediaSession { setActionHandler(action, handler) { originalCalls.push(action); } }
  const mediaSession = new MediaSession();
  const window = {};
  window.top = window;
  window.MediaSession = MediaSession;
  window.ipc = { postMessage: (m) => sent.push(m) };
  const document = {
    title,
    querySelectorAll: () => medias,
    getElementById: (id) => (id === 'movie_player' ? yt : null),
    addEventListener: (t, f) => (listeners[t] ||= []).push(f),
  };
  const ctx = {
    window, document, URL,
    // El canal autenticado lo pone el envoltorio de webview.rs.
    __ytmpSend: (m) => sent.push(m),
    location: { hostname: host, href: `https://${host}/watch?v=dQw4w9WgXcQ` },
    navigator: { mediaSession },
    setTimeout: (f, ms) => { timers.push({ f, at: now + ms, every: 0 }); return timers.length; },
    clearTimeout: (id) => { if (timers[id - 1]) timers[id - 1].f = null; },
    setInterval: (f, ms) => { timers.push({ f, at: now + ms, every: ms }); return timers.length; },
    clearInterval: (id) => { if (timers[id - 1]) timers[id - 1].f = null; },
  };
  vm.runInNewContext(src, ctx);
  const advance = (ms) => {
    const end = now + ms;
    for (;;) {
      const due = timers.filter((t) => t.f && t.at <= end).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      now = due.at;
      const f = due.f;
      if (due.every) due.at += due.every; else due.f = null;
      f();
    }
    now = end;
  };
  const fire = (type) => (listeners[type] || []).forEach((f) => f({ type }));
  const states = () => sent.filter((m) => m.startsWith('media-state:')).map((m) => JSON.parse(m.slice(12)));
  return { window, mediaSession, sent, states, advance, fire, originalCalls };
}

// --- Controles declarados por el sitio --------------------------------------
{
  const v = video({ paused: false });
  const t = env({ host: 'www.netflix.com', medias: [v] });
  const calls = [];
  t.mediaSession.setActionHandler('nexttrack', (d) => calls.push(d.action));
  t.mediaSession.setActionHandler('seekto', (d) => calls.push('seekto:' + d.seekTime));
  t.mediaSession.setActionHandler('seekforward', (d) => calls.push('fwd:' + d.seekOffset));
  assert.deepStrictEqual(t.originalCalls, ['nexttrack', 'seekto', 'seekforward'], 'se sigue llamando al original');

  t.window.__ytmpMedia('next');
  t.window.__ytmpMedia('seek-to', 42);
  t.window.__ytmpMedia('seek-by', 10);
  assert.deepStrictEqual(calls, ['nexttrack', 'seekto:42', 'fwd:10']);

  // Quitar un control (handler null) deja de usarlo.
  t.mediaSession.setActionHandler('seekto', null);
  t.window.__ytmpMedia('seek-to', 99);
  assert.strictEqual(v.currentTime, 0, 'en Netflix nunca se toca currentTime directamente');
  t.window.__ytmpMedia('seek-by', -10);
  assert.strictEqual(v.currentTime, 0);
}

// --- Video genérico -------------------------------------------------------------
{
  const v = video();
  const t = env({ medias: [v] });
  t.window.__ytmpMedia('toggle');
  assert.strictEqual(v.paused, false, 'reproduce');
  t.window.__ytmpMedia('toggle');
  assert.strictEqual(v.paused, true, 'pausa');
  t.window.__ytmpMedia('seek-to', 120);
  assert.strictEqual(v.currentTime, 120);
  t.window.__ytmpMedia('seek-by', -30);
  assert.strictEqual(v.currentTime, 90);
  t.window.__ytmpMedia('seek-to', 9999);
  assert.strictEqual(v.currentTime, 300, 'no pasa del final');
  t.window.__ytmpMedia('volume', 0.4);
  assert.strictEqual(v.volume, 0.4);
  t.window.__ytmpMedia('volume-by', 0.1);
  assert(Math.abs(v.volume - 0.5) < 1e-9);
  t.window.__ytmpMedia('volume-by', 5);
  assert.strictEqual(v.volume, 1, 'no pasa de 100 %');
  t.window.__ytmpMedia('mute');
  assert.strictEqual(v.muted, true);
  t.window.__ytmpMedia('volume-by', 0.1);
  assert.strictEqual(v.muted, false, 'subir el volumen quita el silencio');
  t.window.__ytmpMedia('previous');
  assert.strictEqual(v.currentTime, 0, 'anterior sin lista = volver al inicio');
  t.window.__ytmpMedia('borrar-todo');
  t.window.__ytmpMedia('constructor');
}

// --- Reproductor de YouTube ------------------------------------------------------
{
  const log = [];
  let vol = 50, muted = false;
  const yt = {
    getPlayerState: () => 1,
    playVideo: () => log.push('play'), pauseVideo: () => log.push('pause'),
    nextVideo: () => log.push('next'), previousVideo: () => log.push('prev'),
    seekTo: (s) => log.push('seek:' + s),
    getVolume: () => vol, setVolume: (v) => { vol = v; log.push('vol:' + v); },
    isMuted: () => muted, mute: () => { muted = true; }, unMute: () => { muted = false; },
  };
  const v = video({ paused: false });
  const t = env({ host: 'www.youtube.com', medias: [v], yt });
  t.window.__ytmpMedia('toggle');
  t.window.__ytmpMedia('next');
  t.window.__ytmpMedia('previous');
  t.window.__ytmpMedia('seek-by', 10);
  t.window.__ytmpMedia('volume-by', -0.1);
  assert.deepStrictEqual(log, ['pause', 'next', 'prev', 'seek:10', 'vol:40']);
  t.window.__ytmpMedia('mute');
  assert.strictEqual(muted, true);
  t.window.__ytmpMedia('volume', 0.8);
  assert.strictEqual(muted, false, 'fijar volumen quita el silencio');
  assert.strictEqual(vol, 80);
}

// --- Informe del estado ------------------------------------------------------------
{
  const v = video({ paused: false, currentTime: 61.7, duration: 200, volume: 0.5 });
  const t = env({ host: 'www.crunchyroll.com', medias: [v] });
  t.mediaSession.metadata = {
    title: 'Episodio 3', artist: 'Una serie',
    artwork: [
      { src: 'https://img.example/96.jpg', sizes: '96x96' },
      { src: 'https://img.example/512.jpg', sizes: '512x512' },
      { src: 'https://img.example/2000.jpg', sizes: '2000x2000' },
    ],
  };
  t.mediaSession.setActionHandler('nexttrack', () => {});
  t.window.__ytmpMediaReport();
  const s = t.states().pop();
  assert.strictEqual(s.title, 'Episodio 3');
  assert.strictEqual(s.artist, 'Una serie');
  assert.strictEqual(s.artwork, 'https://img.example/512.jpg', 'la mayor sin pasar de 640 px');
  assert.strictEqual(s.playing, true);
  assert.strictEqual(s.duration, 200);
  assert.strictEqual(s.volume, 0.5);
  assert.strictEqual(s.canNext, true);
  assert.strictEqual(s.canSeek, true);

  // Sin cambios no se repite; un cambio sí se informa.
  const before = t.states().length;
  t.advance(4100);
  assert.strictEqual(t.states().length, before, 'sin cambios, sin mensajes');
  v.paused = true;
  t.fire('pause');
  t.advance(200);
  assert.strictEqual(t.states().length, before + 1);
  assert.strictEqual(t.states().pop().playing, false);

  // El avance de la posición solo se informa mientras se observa.
  v.paused = false;
  t.fire('play');
  t.advance(200);
  const base = t.states().length;
  v.currentTime = 70; t.advance(2100);
  assert.strictEqual(t.states().length, base, 'sin observar, la posición no genera mensajes');
  t.window.__ytmpMediaWatch(true);
  const watching = t.states().length;
  for (let i = 1; i <= 3; i++) { v.currentTime = 70 + i; t.advance(1000); }
  assert.strictEqual(t.states().length, watching + 3, 'observando: uno por segundo');
  t.window.__ytmpMediaWatch(false);
  const stopped = t.states().length;
  for (let i = 1; i <= 3; i++) { v.currentTime = 80 + i; t.advance(1000); }
  assert.strictEqual(t.states().length, stopped, 'al dejar de observar, se detiene');
}

// --- YouTube sin metadatos: título y miniatura de respaldo ---------------------------
{
  const yt = { getPlayerState: () => 2, getVolume: () => 30, isMuted: () => true, nextVideo() {} };
  const t = env({ host: 'www.youtube.com', medias: [video()], yt, title: 'Mi canción - YouTube' });
  t.window.__ytmpMediaReport();
  const s = t.states().pop();
  assert.strictEqual(s.title, 'Mi canción');
  assert.strictEqual(s.artwork, 'https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg');
  assert.strictEqual(s.volume, 0.3);
  assert.strictEqual(s.muted, true);
}

// --- Sin medios, en el menú de inicio y en iframes no hace nada ------------------------
{
  const t = env({ medias: [] });
  t.window.__ytmpMediaReport();
  assert.strictEqual(t.states().pop().hasMedia, false);
  t.window.__ytmpMedia('toggle');

  const home = env({ host: 'ytmp.localhost' });
  assert.strictEqual(home.window.__ytmpMedia, undefined);
  const w = { addEventListener() {} }; w.top = {};
  vm.runInNewContext(src, { window: w, document: {}, location: { hostname: 'x.com' }, __ytmpSend() {} });
  assert.strictEqual(w.__ytmpMedia, undefined);
}

console.log('media.js: todas las pruebas pasan');
