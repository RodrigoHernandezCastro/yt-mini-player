// Memory upkeep in long sessions.
//
// YouTube's SPA grows over the hours: internal navigations do not free the
// page. This script reloads the document when ALL of these hold:
//   - no video is playing,
//   - the user has not interacted for IDLE,
//   - the document has been loaded for at least MIN_AGE,
// AND one of these holds too:
//   - the document is older than MAX_AGE, or
//   - the JavaScript heap is over HEAP_LIMIT.
//
// When reloading it keeps the video and the exact second (the `t`
// parameter) and, on the way back, stops the video from resuming on its own
// for RESUME_GUARD. Limitation: the temporary queue ("Add to queue") is not in
// the URL and is lost. Playlists (`list=`) are kept.
(() => {
  'use strict';

  if (window.top !== window || window.__ytMiniPlayerMaintenance) return;
  Object.defineProperty(window, '__ytMiniPlayerMaintenance', { value: true });
  // Do not touch Google's sign-in pages.
  if (!/(^|\.)youtube\.com$/.test(location.hostname)) return;

  const MINUTE = 60 * 1000;
  const CHECK_EVERY = 5 * MINUTE;
  const MIN_AGE = 30 * MINUTE;
  const MAX_AGE = 12 * 60 * MINUTE;
  const HEAP_LIMIT = 768 * 1024 * 1024; // bytes of JS heap in use
  const IDLE = 15 * MINUTE;
  const RESUME_GUARD = 60 * 1000;
  const FLAG = 'ytmp-maintenance-reload';

  let lastInput = Date.now();
  let guardUntil = 0;

  function onInput() {
    lastInput = Date.now();
    guardUntil = 0; // if the user interacts, they decide
  }
  for (const type of ['pointerdown', 'keydown', 'wheel']) {
    window.addEventListener(type, onInput, { capture: true, passive: true });
  }

  // --- Back from an upkeep reload -----------------------------------------------

  try {
    if (sessionStorage.getItem(FLAG)) {
      sessionStorage.removeItem(FLAG);
      guardUntil = Date.now() + RESUME_GUARD;
    }
  } catch (_) { /* storage not available: no guard */ }

  // `play` does not bubble, but it goes through window in the capture phase.
  window.addEventListener('play', (e) => {
    if (Date.now() < guardUntil && e.target && typeof e.target.pause === 'function') {
      e.target.pause();
    }
  }, true);

  // --- Periodic check ---------------------------------------------------------

  function isPlaying() {
    return Array.from(document.querySelectorAll('video')).some((v) => !v.paused && !v.ended);
  }

  function heapBytes() {
    const m = performance.memory; // not standard; available in Chromium
    return (m && m.usedJSHeapSize) || 0;
  }

  function mainVideo() {
    return document.querySelector('#movie_player video') || document.querySelector('video');
  }

  function reloadPreservingPosition() {
    const url = new URL(location.href);
    const video = mainVideo();
    if (url.pathname === '/watch' && video && video.currentTime > 1) {
      url.searchParams.set('t', Math.floor(video.currentTime) + 's');
    }
    try { sessionStorage.setItem(FLAG, '1'); } catch (_) {}
    if (url.href === location.href) location.reload();
    else location.replace(url.href);
  }

  setInterval(() => {
    const age = performance.now(); // since the document loaded, not the SPA
    if (age < MIN_AGE || Date.now() - lastInput < IDLE || isPlaying()) return;
    if (age >= MAX_AGE || heapBytes() >= HEAP_LIMIT) reloadPreservingPosition();
  }, CHECK_EVERY);
})();
