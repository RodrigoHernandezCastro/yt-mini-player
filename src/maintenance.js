// Mantenimiento de memoria en sesiones largas (Fase 4).
//
// La SPA de YouTube crece con las horas: las navegaciones internas no
// liberan la página. Este script recarga el documento cuando se cumplen
// TODAS estas condiciones:
//   - no hay ningún video reproduciéndose,
//   - el usuario no ha interactuado en IDLE,
//   - el documento lleva al menos MIN_AGE cargado,
// y ADEMÁS se cumple alguna de estas:
//   - el documento supera MAX_AGE, o
//   - el heap de JavaScript supera HEAP_LIMIT.
//
// Al recargar conserva el video y el segundo exacto (parámetro `t`) y, al
// volver, impide que el video se reanude solo durante RESUME_GUARD.
// Limitación: la cola temporal ("Añadir a la cola") no está en la URL y se
// pierde. Las listas de reproducción (`list=`) sí se conservan.
(() => {
  'use strict';

  if (window.top !== window || window.__ytMiniPlayerMaintenance) return;
  Object.defineProperty(window, '__ytMiniPlayerMaintenance', { value: true });
  // No tocar las páginas de inicio de sesión de Google.
  if (!/(^|\.)youtube\.com$/.test(location.hostname)) return;

  const MINUTE = 60 * 1000;
  const CHECK_EVERY = 5 * MINUTE;
  const MIN_AGE = 30 * MINUTE;
  const MAX_AGE = 12 * 60 * MINUTE;
  const HEAP_LIMIT = 768 * 1024 * 1024; // bytes de heap JS usado
  const IDLE = 15 * MINUTE;
  const RESUME_GUARD = 60 * 1000;
  const FLAG = 'ytmp-maintenance-reload';

  let lastInput = Date.now();
  let guardUntil = 0;

  function onInput() {
    lastInput = Date.now();
    guardUntil = 0; // si el usuario interactúa, decide él
  }
  for (const type of ['pointerdown', 'keydown', 'wheel']) {
    window.addEventListener(type, onInput, { capture: true, passive: true });
  }

  // --- Vuelta de una recarga de mantenimiento ------------------------------

  try {
    if (sessionStorage.getItem(FLAG)) {
      sessionStorage.removeItem(FLAG);
      guardUntil = Date.now() + RESUME_GUARD;
    }
  } catch (_) { /* almacenamiento no disponible: sin protección */ }

  // `play` no burbujea, pero sí pasa por window en la fase de captura.
  window.addEventListener('play', (e) => {
    if (Date.now() < guardUntil && e.target && typeof e.target.pause === 'function') {
      e.target.pause();
    }
  }, true);

  // --- Comprobación periódica ----------------------------------------------

  function isPlaying() {
    return Array.from(document.querySelectorAll('video')).some((v) => !v.paused && !v.ended);
  }

  function heapBytes() {
    const m = performance.memory; // no estándar; disponible en Chromium
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
    const age = performance.now(); // desde la carga del documento, no de la SPA
    if (age < MIN_AGE || Date.now() - lastInput < IDLE || isPlaying()) return;
    if (age >= MAX_AGE || heapBytes() >= HEAP_LIMIT) reloadPreservingPosition();
  }, CHECK_EVERY);
})();
