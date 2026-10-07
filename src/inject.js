// Inyectado por el host en cada documento (ver webview.rs).
//
// Mover la ventana:
// - Arrastrar sobre el video (más de DRAG_THRESHOLD px) → "drag".
//   Un clic sin mover sigue llegando a YouTube (pausa/reproduce).
// - Alt + clic izquierdo en cualquier punto           → "drag".
// - El asa de la barra lateral (ui.js)                 → "drag".
// Redimensionar:
// - Clic izquierdo en una franja de borde              → "resize:<dir>".
//
// Un solo mensaje por gesto; el movimiento y el redimensionado los ejecuta
// el bucle modal de Windows, sin tráfico por el puente IPC.
(() => {
  'use strict';

  // En Windows, wry inyecta los scripts también en iframes.
  if (window.top !== window || window.__ytMiniPlayer) return;
  Object.defineProperty(window, '__ytMiniPlayer', { value: true });

  const EDGE = 6;            // px CSS de franja de borde
  const CORNER = 12;         // px CSS de zona de esquina a lo largo de cada borde
  const DRAG_THRESHOLD = 6;  // px que hay que mover el video para arrastrar
  const ATTR = 'data-ytmp-resize';
  const CURSORS = {
    n: 'ns-resize', s: 'ns-resize',
    e: 'ew-resize', w: 'ew-resize',
    ne: 'nesw-resize', sw: 'nesw-resize',
    nw: 'nwse-resize', se: 'nwse-resize',
  };
  // Zonas del reproductor que NO sirven para arrastrar: controles, menús,
  // tarjetas, anuncios y cualquier botón o enlace.
  const NOT_DRAGGABLE = [
    '.ytp-chrome-bottom', '.ytp-chrome-top', '.ytp-popup', '.ytp-settings-menu',
    '.ytp-ce-element', '.ytp-cards-teaser', '.ytp-cards-button', '.ytp-paid-content-overlay',
    '.ytp-ad-module', '.ytp-ad-overlay-container', '.ytp-player-content',
    '.ytp-suggested-action', '.ytp-autonav-endscreen-countdown-overlay', '.ytp-videowall-still',
    'ytmp-ui', 'button', 'a', 'input', 'textarea', 'select', '[role="button"]', '[contenteditable]',
  ].join(',');

  const isFullscreen = () => !!document.fullscreenElement;

  function send(message) {
    try { window.ipc.postMessage(message); } catch (_) { /* fuera del host */ }
  }

  // Devuelve la zona de redimensionado bajo (x, y) o null.
  function zoneAt(x, y) {
    if (isFullscreen()) return null;
    const w = window.innerWidth;
    const h = window.innerHeight;
    const top = y < EDGE, bottom = y >= h - EDGE;
    const left = x < EDGE, right = x >= w - EDGE;
    if (!(top || bottom || left || right)) return null;

    const nearTop = y < CORNER, nearBottom = y >= h - CORNER;
    const nearLeft = x < CORNER, nearRight = x >= w - CORNER;
    if ((top && nearLeft) || (left && nearTop)) return 'nw';
    if ((top && nearRight) || (right && nearTop)) return 'ne';
    if ((bottom && nearLeft) || (left && nearBottom)) return 'sw';
    if ((bottom && nearRight) || (right && nearBottom)) return 'se';
    if (top) return 'n';
    if (bottom) return 's';
    if (left) return 'w';
    return 'e';
  }

  // ¿El punto de la pulsación es "superficie de video" arrastrable?
  function isVideoSurface(target) {
    if (isFullscreen() || !(target instanceof Element)) return false;
    const player = target.closest('#movie_player, .html5-video-player');
    return !!player && !target.closest(NOT_DRAGGABLE);
  }

  // --- Cursor: se resuelve localmente, sin IPC -----------------------------

  let currentZone = null;
  function setZone(zone) {
    if (zone === currentZone) return; // solo tocamos el DOM si cambia la zona
    currentZone = zone;
    const root = document.documentElement;
    if (!root) return;
    if (zone) root.setAttribute(ATTR, zone);
    else root.removeAttribute(ATTR);
  }

  function installCursorStyle() {
    const css = Object.entries(CURSORS)
      .map(([z, c]) => `html[${ATTR}="${z}"], html[${ATTR}="${z}"] * { cursor: ${c} !important; }`)
      .join('\n');
    const style = document.createElement('style');
    style.textContent = css;
    (document.head || document.documentElement).appendChild(style);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', installCursorStyle, { once: true });
  } else {
    installCursorStyle();
  }

  // --- Pulsación ------------------------------------------------------------

  // Tras un arrastre o redimensionado, el click (y el dblclick) que pueda
  // llegar no debe alcanzar a YouTube, o pausaría el video o entraría en
  // pantalla completa.
  let swallowClick = false;
  // Arrastre del video pendiente de superar el umbral.
  let pending = null;

  function consume(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }

  function startDrag() {
    swallowClick = true;
    send('drag');
  }

  // pointerdown se dispara antes que mousedown; con preventDefault aquí
  // Chromium tampoco emite los mousedown/mouseup de compatibilidad.
  window.addEventListener('pointerdown', (e) => {
    swallowClick = false;
    pending = null;
    if (e.pointerType !== 'mouse' || e.button !== 0) return;

    if (e.altKey) {
      consume(e);
      startDrag();
      return;
    }
    const zone = zoneAt(e.clientX, e.clientY);
    if (zone) {
      consume(e);
      swallowClick = true;
      send('resize:' + zone);
      return;
    }
    if (isVideoSurface(e.target)) {
      // No se consume: si no hay movimiento, es un clic normal de YouTube.
      const video = document.querySelector('#movie_player video, .html5-video-player video');
      pending = {
        x: e.clientX, y: e.clientY, target: e.target,
        video, rate: video ? video.playbackRate : null,
      };
    }
  }, { capture: true });

  window.addEventListener('pointermove', (e) => {
    if (e.pointerType === 'mouse' && e.buttons === 0) {
      // No cambiar el cursor en mitad de una selección.
      setZone(e.altKey ? null : zoneAt(e.clientX, e.clientY));
    }
    if (!pending) return;
    if (!(e.buttons & 1)) { pending = null; return; }
    const dx = e.clientX - pending.x, dy = e.clientY - pending.y;
    if (dx * dx + dy * dy < DRAG_THRESHOLD * DRAG_THRESHOLD) return;

    const { target, video, rate } = pending;
    pending = null;
    consume(e);
    // YouTube acelera a 2x si se mantiene pulsado el video. Durante el
    // arrastre Windows se queda con el mouseup, así que se lo enviamos
    // antes para que no quede "pulsado", y se restaura la velocidad si
    // llegó a cambiar.
    const init = { bubbles: true, cancelable: true, composed: true, clientX: e.clientX, clientY: e.clientY, button: 0 };
    try {
      target.dispatchEvent(new PointerEvent('pointerup', { ...init, pointerType: 'mouse', isPrimary: true }));
      target.dispatchEvent(new MouseEvent('mouseup', init));
    } catch (_) { /* nada */ }
    if (video && rate !== null) {
      const restore = () => { if (video.playbackRate !== rate) video.playbackRate = rate; };
      setTimeout(restore, 0);
      window.addEventListener('pointermove', restore, { once: true, capture: true });
    }
    startDrag();
  }, { capture: true });

  window.addEventListener('pointerup', () => { pending = null; }, { capture: true });

  window.addEventListener('mouseout', (e) => {
    if (!e.relatedTarget) setZone(null); // el cursor salió de la ventana
  }, { capture: true, passive: true });

  // Por si algún listener de la página escucha mousedown directamente.
  window.addEventListener('mousedown', (e) => {
    if (swallowClick) consume(e);
  }, { capture: true });

  window.addEventListener('click', (e) => {
    if (!swallowClick) return;
    swallowClick = false;
    consume(e);
  }, { capture: true });

  window.addEventListener('dblclick', (e) => {
    if (e.altKey || zoneAt(e.clientX, e.clientY)) consume(e);
  }, { capture: true });

  // Expuesto para el asa de la barra lateral (ui.js).
  Object.defineProperty(window, '__ytmpStartDrag', { value: startDrag });
})();
