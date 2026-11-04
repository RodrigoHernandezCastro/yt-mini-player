// Reading and controlling playback, injected into every site (see
// webview.rs). It lets the video be handled with the app hidden: the music
// bar, the tray and the global shortcuts end up here.
//
// For each action it uses, in this order:
// 1. The controls the site itself declares to the browser with the Media
//    Session API (YouTube, Netflix, Crunchyroll... register "next",
//    "previous", "seek forward", etc.). This script runs before the page
//    and notes those controls, so it can press them just as the system
//    would.
// 2. YouTube's player API (#movie_player).
// 3. The <video>/<audio> element directly.
//
// The page tells the app with "media-state:<json>" when something changes;
// the position is only sent every second while the music bar is in sight
// (__ytmpMediaWatch).
(() => {
  'use strict';

  if (window.top !== window || window.__ytmpMediaInstalled) return;
  Object.defineProperty(window, '__ytmpMediaInstalled', { value: true });
  // Of the app's own pages, only "My music" plays anything.
  if (location.hostname === 'ytmp.localhost' && location.pathname !== '/music') return;

  // Netflix stops playback (error M7375) if currentTime is changed from
  // outside; there it only seeks with its own controls or its player's API.
  const NO_DIRECT_SEEK = /(^|\.)netflix\.com$/.test(location.hostname);
  const POLL_MS = 2000;
  const WATCH_MS = 1000;

  // Authenticated channel with the app (session token, see webview.rs).
  const send = (message) => __ytmpSend(message);

  // --- 1. Controls declared by the site ------------------------------------------

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
  } catch (_) { /* no Media Session: the other ways are used */ }

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

  // --- 2 and 3. YouTube player and media element ----------------------------------

  function ytPlayer() {
    const player = document.getElementById('movie_player');
    return player && typeof player.getPlayerState === 'function' ? player : null;
  }

  // The "main" media: the one playing or, if none, the biggest.
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

  // Thumbnail: the biggest the site declares, no larger than 640px.
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

  // Netflix's own player, if the page exposes it (watch together seeks to
  // an exact moment with it).
  function netflixPlayer() {
    try {
      const api = window.netflix.appContext.state.playerApp.getAPI().videoPlayer;
      const ids = api.getAllPlayerSessionIds();
      const player = ids && ids.length ? api.getVideoPlayerBySessionId(ids[ids.length - 1]) : null;
      return player && typeof player.seek === 'function' ? player : null;
    } catch (_) {
      return null;
    }
  }

  // An advert is playing (YouTube marks its player while one is on).
  function adShowing() {
    try {
      return !!document.querySelector('#movie_player.ad-showing, .html5-video-player.ad-showing');
    } catch (_) {
      return false;
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
      rate: media ? finite(media.playbackRate, 1) : 1,
      ad: adShowing(),
    };
  }

  // --- Report to the app ----------------------------------------------------------

  let lastSent = '';
  let watching = false;

  // The position only counts as a change while someone watches the progress.
  const keyFor = (state) => JSON.stringify(Object.assign({}, state, { position: watching ? Math.floor(state.position) : 0 }));

  // At most one report every REPORT_GAP_MS, whoever asks: the page itself
  // can call __ytmpMediaReport in a loop, and every report reaches the app
  // (tray, music bar, the guests of a watch together host). What is asked
  // for meanwhile goes out once the gap is over, so the last state always
  // arrives.
  const REPORT_GAP_MS = 250;
  let cooling = false;
  let pending = null; // null: nothing waiting; otherwise whether it was forced

  function report(force) {
    if (cooling) {
      pending = !!(pending || force);
      return;
    }
    const state = readState();
    const key = keyFor(state);
    if (!force && key === lastSent) return;
    lastSent = key;
    send('media-state:' + JSON.stringify(state));
    cooling = true;
    setTimeout(() => {
      cooling = false;
      if (pending !== null) {
        const forced = pending;
        pending = null;
        report(forced);
      }
    }, REPORT_GAP_MS);
  }

  let soon = 0;
  function reportSoon() {
    clearTimeout(soon);
    soon = setTimeout(() => report(false), 120);
  }

  // Media events do not bubble, but they go through the capture phase.
  for (const type of ['play', 'pause', 'ended', 'emptied', 'loadedmetadata', 'durationchange', 'volumechange', 'seeked', 'ratechange']) {
    document.addEventListener(type, reportSoon, true);
  }
  // Media Session metadata and changes without an event (internal navigation).
  setInterval(() => report(false), POLL_MS);
  let watchTimer = 0;

  // --- Actions --------------------------------------------------------------------

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
    if (!NO_DIRECT_SEEK) {
      media.currentTime = target;
      return;
    }
    const netflix = netflixPlayer();
    if (netflix) netflix.seek(Math.round(target * 1000));
  }

  function seekBy(delta) {
    const media = mainMedia();
    if (!media) return;
    const action = delta >= 0 ? 'seekforward' : 'seekbackward';
    if (runHandler(action, { seekOffset: Math.abs(delta) })) return;
    seekTo(media.currentTime + delta);
  }

  // Whether it is playing, as the site sees it (YouTube's state rules).
  function isPlaying() {
    const media = mainMedia();
    const yt = ytPlayer();
    return yt ? yt.getPlayerState() === 1 : !!media && !media.paused && !media.ended;
  }

  const actions = {
    // Watch together: play only (never pauses).
    play() {
      if (isPlaying()) return;
      if (runHandler('play')) return;
      const yt = ytPlayer();
      if (yt) {
        yt.playVideo();
        return;
      }
      const media = mainMedia();
      if (media) media.play().catch(() => {});
    },
    // Watch together: the host's speed.
    rate(value) {
      const rate = Math.min(4, Math.max(0.25, finite(Number(value), 1)));
      const yt = ytPlayer();
      if (yt && typeof yt.setPlaybackRate === 'function') {
        yt.setPlaybackRate(rate);
        return;
      }
      const media = mainMedia();
      if (media) media.playbackRate = rate;
    },
    toggle() {
      const media = mainMedia();
      const yt = ytPlayer();
      // With YouTube, its player's state rules (1 = playing).
      const playing = yt ? yt.getPlayerState() === 1 : !!media && !media.paused && !media.ended;
      if (runHandler(playing ? 'pause' : 'play')) return;
      if (yt) {
        if (playing) yt.pauseVideo(); else yt.playVideo();
        return;
      }
      if (!media) return;
      if (playing) media.pause(); else media.play().catch(() => {});
    },
    // When the app locks: pause if playing (never resumes).
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
    try { fn(arg); } catch (_) { /* the site changed: break nothing */ }
    reportSoon();
  }

  function watch(on) {
    watching = !!on;
    clearInterval(watchTimer);
    if (watching) {
      watchTimer = setInterval(() => report(false), WATCH_MS);
      report(true);
    } else {
      // New reference without a position: nothing new to tell.
      lastSent = keyFor(readState());
    }
  }

  Object.defineProperty(window, '__ytmpMedia', { value: run });
  Object.defineProperty(window, '__ytmpMediaWatch', { value: watch });
  Object.defineProperty(window, '__ytmpMediaReport', { value: () => report(true) });
})();
