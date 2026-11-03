// Injected by the host into every document (see webview.rs).
//
// Moving the window:
// - Dragging over the video (more than DRAG_THRESHOLD px) → "drag". Works on
//   YouTube and on any site with a big video (Netflix, Crunchyroll...).
// - Areas with the data-ytmp-drag attribute (start screen) → "drag".
//   A click without moving still reaches YouTube (pause/play).
// - Alt + left click anywhere                         → "drag".
// - The side bar's handle (ui.js)                      → "drag".
// Resizing:
// - Left click on an edge strip                        → "resize:<dir>".
//
// One message per gesture; Windows' modal loop does the moving and resizing,
// with no traffic over the IPC bridge.
(() => {
  'use strict';

  // On Windows, wry injects the scripts into iframes too.
  if (window.top !== window || window.__ytMiniPlayer) return;
  Object.defineProperty(window, '__ytMiniPlayer', { value: true });

  const EDGE = 6;            // CSS px of the edge strip
  const CORNER = 12;         // CSS px of the corner area along each edge
  const DRAG_THRESHOLD = 6;  // px the video must be moved before dragging
  const ATTR = 'data-ytmp-resize';
  const CURSORS = {
    n: 'ns-resize', s: 'ns-resize',
    e: 'ew-resize', w: 'ew-resize',
    ne: 'nesw-resize', sw: 'nesw-resize',
    nw: 'nwse-resize', se: 'nwse-resize',
  };
  // Areas of the player that do NOT drag: controls, menus, cards, ads and any
  // button or link.
  const NOT_DRAGGABLE = [
    '.ytp-chrome-bottom', '.ytp-chrome-top', '.ytp-popup', '.ytp-settings-menu',
    '.ytp-ce-element', '.ytp-cards-teaser', '.ytp-cards-button', '.ytp-paid-content-overlay',
    '.ytp-ad-module', '.ytp-ad-overlay-container', '.ytp-player-content',
    '.ytp-suggested-action', '.ytp-autonav-endscreen-countdown-overlay', '.ytp-videowall-still',
    'ytmp-ui', 'button', 'a', 'input', 'textarea', 'select', '[role="button"]', '[contenteditable]',
    // The host's shared window while this guest uses it (stream.html).
    '[data-ytmp-nodrag]',
  ].join(',');

  const isFullscreen = () => !!document.fullscreenElement;

  // Authenticated channel with the app: defined by webview.rs' wrapper (it
  // carries a session token the page's scripts do not know).
  function send(message) {
    __ytmpSend(message);
  }

  // Returns the resize zone under (x, y), or null.
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

  // Is the pressed point a draggable "video surface"?
  const IS_YOUTUBE = /(^|\.)youtube\.com$/.test(location.hostname);
  // Generic controls of other players (Netflix, Crunchyroll...).
  const GENERIC_CONTROLS = '[role="slider"], [role="menu"], [role="menuitem"], [role="dialog"], '
    + '[role="listbox"], [role="option"], [role="tab"], label, summary';
  // On other services, a "main" video covers at least this fraction.
  const MIN_VIDEO_AREA = 0.2;

  // A big video under the point (x, y), on any site.
  function videoAt(x, y) {
    const minArea = innerWidth * innerHeight * MIN_VIDEO_AREA;
    for (const video of document.querySelectorAll('video')) {
      const r = video.getBoundingClientRect();
      if (r.width * r.height >= minArea && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) {
        return video;
      }
    }
    return null;
  }

  // Is the pressed point a draggable "video surface"?
  // Returns the matching <video> element, or null.
  function videoSurface(target, x, y) {
    if (isFullscreen() || !(target instanceof Element)) return null;
    if (target.closest(NOT_DRAGGABLE)) return null;
    if (IS_YOUTUBE) {
      const player = target.closest('#movie_player, .html5-video-player');
      if (player) return player.querySelector('video') || player;
      // Renamed player (YouTube changed its page): any big video still
      // drags, as on other sites.
      if (document.querySelector('#movie_player, .html5-video-player')) return null;
    }
    if (target.closest(GENERIC_CONTROLS)) return null;
    return videoAt(x, y);
  }

  // --- Cursor: worked out locally, without IPC ---------------------------------

  let currentZone = null;
  function setZone(zone) {
    if (zone === currentZone) return; // the DOM is only touched when the zone changes
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

  // --- Press ------------------------------------------------------------------

  // After a drag or resize, the click (and dblclick) that may follow must not
  // reach YouTube, or it would pause the video or go full screen.
  let swallowClick = false;
  // Video drag waiting to pass the threshold.
  let pending = null;

  function consume(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
  }

  function startDrag() {
    swallowClick = true;
    send('drag');
  }

  // pointerdown fires before mousedown; with preventDefault here Chromium
  // does not send the compatibility mousedown/mouseup either.
  window.addEventListener('pointerdown', (e) => {
    // Only the real mouse: a synthetic event from the page does not move the window.
    if (!e.isTrusted) return;
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
    // Areas the app itself marks (the start screen's header, its language
    // menu excluded): they drag right away.
    if (e.target instanceof Element && e.target.closest('[data-ytmp-drag]')
      && !e.target.closest('button, a, input, select, textarea, [role="switch"], [role="menu"]')) {
      consume(e);
      startDrag();
      return;
    }
    const surface = videoSurface(e.target, e.clientX, e.clientY);
    if (surface) {
      // Not consumed: without movement it is an ordinary click on the site.
      const video = surface instanceof HTMLVideoElement ? surface : null;
      pending = {
        x: e.clientX, y: e.clientY, target: e.target,
        video, rate: video ? video.playbackRate : null,
      };
    }
  }, { capture: true });

  window.addEventListener('pointermove', (e) => {
    if (e.pointerType === 'mouse' && e.buttons === 0) {
      // Do not change the cursor in the middle of a selection.
      setZone(e.altKey ? null : zoneAt(e.clientX, e.clientY));
    }
    // Only the real mouse turns a press into a drag (as in pointerdown).
    if (!pending || !e.isTrusted) return;
    if (!(e.buttons & 1)) { pending = null; return; }
    const dx = e.clientX - pending.x, dy = e.clientY - pending.y;
    if (dx * dx + dy * dy < DRAG_THRESHOLD * DRAG_THRESHOLD) return;

    const { target, video, rate } = pending;
    pending = null;
    consume(e);
    // YouTube speeds up to 2x while the video is held down. During the drag
    // Windows keeps the mouseup, so it is sent beforehand so the video is not
    // left "pressed", and the speed is restored if it changed.
    const init = { bubbles: true, cancelable: true, composed: true, clientX: e.clientX, clientY: e.clientY, button: 0 };
    try {
      target.dispatchEvent(new PointerEvent('pointerup', { ...init, pointerType: 'mouse', isPrimary: true }));
      target.dispatchEvent(new MouseEvent('mouseup', init));
    } catch (_) { /* nothing */ }
    if (video && rate !== null) {
      const restore = () => { if (video.playbackRate !== rate) video.playbackRate = rate; };
      setTimeout(restore, 0);
      window.addEventListener('pointermove', restore, { once: true, capture: true });
    }
    startDrag();
  }, { capture: true });

  window.addEventListener('pointerup', () => { pending = null; }, { capture: true });

  window.addEventListener('mouseout', (e) => {
    if (!e.relatedTarget) setZone(null); // the cursor left the window
  }, { capture: true, passive: true });

  // In case a page listener watches mousedown directly.
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

  // For the side bar's handle (ui.js), without exposing it to the page.
  __ytmpShared.startDrag = startDrag;
})();
