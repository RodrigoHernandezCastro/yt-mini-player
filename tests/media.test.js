// Tests of src/media.js with a simulated Media Session, YouTube player,
// generic video and clock.
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

function env({ host = 'www.example.com', medias = [], yt = null, title = 'Page', ad = false, netflix = null } = {}) {
  const sent = [], timers = [], listeners = {};
  let now = 0;
  const originalCalls = [];
  class MediaSession { setActionHandler(action, handler) { originalCalls.push(action); } }
  const mediaSession = new MediaSession();
  const window = {};
  window.top = window;
  if (netflix) window.netflix = netflix;
  window.MediaSession = MediaSession;
  window.ipc = { postMessage: (m) => sent.push(m) };
  const document = {
    title,
    querySelectorAll: () => medias,
    getElementById: (id) => (id === 'movie_player' ? yt : null),
    querySelector: (sel) => (ad && sel.includes('ad-showing') ? {} : null),
    addEventListener: (t, f) => (listeners[t] ||= []).push(f),
  };
  const ctx = {
    window, document, URL,
    // The authenticated channel comes from webview.rs' wrapper.
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

// --- Controls declared by the site ------------------------------------------
{
  const v = video({ paused: false });
  const t = env({ host: 'www.netflix.com', medias: [v] });
  const calls = [];
  t.mediaSession.setActionHandler('nexttrack', (d) => calls.push(d.action));
  t.mediaSession.setActionHandler('seekto', (d) => calls.push('seekto:' + d.seekTime));
  t.mediaSession.setActionHandler('seekforward', (d) => calls.push('fwd:' + d.seekOffset));
  assert.deepStrictEqual(t.originalCalls, ['nexttrack', 'seekto', 'seekforward'], 'the original is still called');

  t.window.__ytmpMedia('next');
  t.window.__ytmpMedia('seek-to', 42);
  t.window.__ytmpMedia('seek-by', 10);
  assert.deepStrictEqual(calls, ['nexttrack', 'seekto:42', 'fwd:10']);

  // Removing a control (null handler) stops using it.
  t.mediaSession.setActionHandler('seekto', null);
  t.window.__ytmpMedia('seek-to', 99);
  assert.strictEqual(v.currentTime, 0, 'on Netflix currentTime is never touched directly');
  t.window.__ytmpMedia('seek-by', -10);
  assert.strictEqual(v.currentTime, 0);
}

// --- Generic video ---------------------------------------------------------------
{
  const v = video();
  const t = env({ medias: [v] });
  t.window.__ytmpMedia('toggle');
  assert.strictEqual(v.paused, false, 'plays');
  t.window.__ytmpMedia('toggle');
  assert.strictEqual(v.paused, true, 'pauses');
  t.window.__ytmpMedia('seek-to', 120);
  assert.strictEqual(v.currentTime, 120);
  t.window.__ytmpMedia('seek-by', -30);
  assert.strictEqual(v.currentTime, 90);
  t.window.__ytmpMedia('seek-to', 9999);
  assert.strictEqual(v.currentTime, 300, 'does not go past the end');
  t.window.__ytmpMedia('volume', 0.4);
  assert.strictEqual(v.volume, 0.4);
  t.window.__ytmpMedia('volume-by', 0.1);
  assert(Math.abs(v.volume - 0.5) < 1e-9);
  t.window.__ytmpMedia('volume-by', 5);
  assert.strictEqual(v.volume, 1, 'does not go over 100%');
  t.window.__ytmpMedia('mute');
  assert.strictEqual(v.muted, true);
  t.window.__ytmpMedia('volume-by', 0.1);
  assert.strictEqual(v.muted, false, 'turning the volume up unmutes');
  t.window.__ytmpMedia('previous');
  assert.strictEqual(v.currentTime, 0, 'previous without a list = back to the start');
  t.window.__ytmpMedia('borrar-todo');
  t.window.__ytmpMedia('constructor');
}

// --- YouTube player ----------------------------------------------------------------
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
  assert.strictEqual(muted, false, 'setting the volume unmutes');
  assert.strictEqual(vol, 80);
}

// --- State report ------------------------------------------------------------------
{
  const v = video({ paused: false, currentTime: 61.7, duration: 200, volume: 0.5 });
  const t = env({ host: 'www.crunchyroll.com', medias: [v] });
  t.mediaSession.metadata = {
    title: 'Episode 3', artist: 'A series',
    artwork: [
      { src: 'https://img.example/96.jpg', sizes: '96x96' },
      { src: 'https://img.example/512.jpg', sizes: '512x512' },
      { src: 'https://img.example/2000.jpg', sizes: '2000x2000' },
    ],
  };
  t.mediaSession.setActionHandler('nexttrack', () => {});
  t.window.__ytmpMediaReport();
  const s = t.states().pop();
  assert.strictEqual(s.title, 'Episode 3');
  assert.strictEqual(s.artist, 'A series');
  assert.strictEqual(s.artwork, 'https://img.example/512.jpg', 'the biggest no larger than 640 px');
  assert.strictEqual(s.playing, true);
  assert.strictEqual(s.duration, 200);
  assert.strictEqual(s.volume, 0.5);
  assert.strictEqual(s.canNext, true);
  assert.strictEqual(s.canSeek, true);

  // Without changes nothing repeats; a change is reported.
  const before = t.states().length;
  t.advance(4100);
  assert.strictEqual(t.states().length, before, 'no changes, no messages');
  v.paused = true;
  t.fire('pause');
  t.advance(200);
  assert.strictEqual(t.states().length, before + 1);
  assert.strictEqual(t.states().pop().playing, false);

  // The position moving on is only reported while watched.
  v.paused = false;
  t.fire('play');
  t.advance(200);
  const base = t.states().length;
  v.currentTime = 70; t.advance(2100);
  assert.strictEqual(t.states().length, base, 'unwatched, the position sends no messages');
  t.window.__ytmpMediaWatch(true);
  const watching = t.states().length;
  for (let i = 1; i <= 3; i++) { v.currentTime = 70 + i; t.advance(1000); }
  assert.strictEqual(t.states().length, watching + 3, 'watched: one per second');
  t.window.__ytmpMediaWatch(false);
  const stopped = t.states().length;
  for (let i = 1; i <= 3; i++) { v.currentTime = 80 + i; t.advance(1000); }
  assert.strictEqual(t.states().length, stopped, 'when no longer watched, it stops');
}

// --- YouTube without metadata: fallback title and thumbnail ----------------------------
{
  const yt = { getPlayerState: () => 2, getVolume: () => 30, isMuted: () => true, nextVideo() {} };
  const t = env({ host: 'www.youtube.com', medias: [video()], yt, title: 'My song - YouTube' });
  t.window.__ytmpMediaReport();
  const s = t.states().pop();
  assert.strictEqual(s.title, 'My song');
  assert.strictEqual(s.artwork, 'https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg');
  assert.strictEqual(s.volume, 0.3);
  assert.strictEqual(s.muted, true);
}

// --- No media, on the start screen and in iframes it does nothing ----------------------
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

// --- Watch together: play only, speed, adverts, exact seeks -------------------------
{
  const v = video();
  const t = env({ medias: [v] });
  t.window.__ytmpMedia('play');
  assert.strictEqual(v.paused, false);
  t.window.__ytmpMedia('play');
  assert.strictEqual(v.plays, 1, '"play" never pauses or restarts');
  t.window.__ytmpMedia('rate', 1.5);
  assert.strictEqual(v.playbackRate, 1.5);
  t.window.__ytmpMedia('rate', 99);
  assert.strictEqual(v.playbackRate, 4, 'bounded');
  t.window.__ytmpMedia('seek-to', 61.005);
  assert.strictEqual(v.currentTime, 61.005, 'to the millisecond');
  t.window.__ytmpMediaReport();
  const s = t.states().pop();
  assert.strictEqual(s.rate, 4);
  assert.strictEqual(s.ad, false);
}
{
  let rate = 1;
  const yt = {
    getPlayerState: () => 2, getVolume: () => 50, isMuted: () => false,
    playVideo() { this.played = true; }, setPlaybackRate(r) { rate = r; },
  };
  const t = env({ host: 'www.youtube.com', medias: [video()], yt, ad: true });
  t.window.__ytmpMedia('play');
  assert.ok(yt.played, 'YouTube plays with its own player');
  t.window.__ytmpMedia('rate', 2);
  assert.strictEqual(rate, 2);
  t.window.__ytmpMediaReport();
  assert.strictEqual(t.states().pop().ad, true, 'an advert is reported');
}
{
  // Netflix: never currentTime; its player's API when it is there.
  const seeks = [];
  const player = { seek: (ms) => seeks.push(ms) };
  const netflix = { appContext: { state: { playerApp: { getAPI: () => ({ videoPlayer: {
    getAllPlayerSessionIds: () => ['a', 'b'],
    getVideoPlayerBySessionId: (id) => (id === 'b' ? player : null),
  } }) } } } };
  const v = video({ currentTime: 10 });
  const t = env({ host: 'www.netflix.com', medias: [v], netflix });
  t.window.__ytmpMedia('seek-to', 125.4);
  assert.deepStrictEqual(seeks, [125400]);
  assert.strictEqual(v.currentTime, 10, 'currentTime untouched');
  const bare = env({ host: 'www.netflix.com', medias: [video({ currentTime: 10 })] });
  bare.window.__ytmpMedia('seek-to', 50);
}

// --- A page that asks for reports in a loop cannot flood the app -----------------------
{
  const t = env({ medias: [video()] });
  for (let i = 0; i < 1000; i++) {
    t.mediaSession.metadata = { title: 'Title ' + i };
    t.window.__ytmpMediaReport();
  }
  assert.strictEqual(t.states().length, 1, 'one report at once');
  t.advance(250);
  assert.strictEqual(t.states().length, 2, 'then one more after the gap');
  assert.strictEqual(t.states().pop().title, 'Title 999', 'with the last state');
  t.advance(2000);
  assert.strictEqual(t.states().length, 2, 'and nothing else');
}

console.log('media.js: all tests pass');
