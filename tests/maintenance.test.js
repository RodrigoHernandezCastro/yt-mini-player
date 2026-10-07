// Pruebas de src/maintenance.js con reloj, DOM y almacenamiento simulados.
const fs = require('fs'), vm = require('vm'), path = require('path'), assert = require('assert');
const src = fs.readFileSync(path.join(__dirname, '..', 'src', 'maintenance.js'), 'utf8');
const MIN = 60 * 1000, MB = 1024 * 1024;

function env({ href = 'https://www.youtube.com/watch?v=abc', stored = {}, top = true } = {}) {
  const listeners = {}, calls = [], storage = { ...stored };
  let tick = null;
  const clock = { now: 0, perf: 0, heap: 100 * MB };
  const videos = [];
  const window = { addEventListener: (t, f) => (listeners[t] ||= []).push(f) };
  window.top = top ? window : {};
  const location = {
    href, get hostname() { return new URL(this.href).hostname; },
    reload() { calls.push('reload'); }, replace(u) { calls.push('replace ' + u); },
  };
  const ctx = {
    window, location, URL,
    Date: { now: () => clock.now },
    performance: { now: () => clock.perf, get memory() { return { usedJSHeapSize: clock.heap }; } },
    sessionStorage: {
      getItem: k => storage[k] ?? null, setItem: (k, v) => { storage[k] = v; }, removeItem: k => { delete storage[k]; },
    },
    document: { querySelectorAll: () => videos, querySelector: () => videos[0] || null },
    setInterval: (f, ms) => { assert.strictEqual(ms, 5 * MIN); tick = f; },
  };
  vm.runInNewContext(src, ctx);
  const advance = (ms) => { clock.now += ms; clock.perf += ms; tick && tick(); };
  const fire = (t, e = {}) => (listeners[t] || []).forEach(f => f(e));
  return { advance, fire, calls, storage, clock, videos, listeners, hasTick: () => !!tick };
}
const video = (o = {}) => ({ paused: true, ended: false, currentTime: 0, pause() { this.paused = true; this.pauses = (this.pauses || 0) + 1; }, ...o });

// Sin reproducción ni interacción, recarga a las 12 h conservando el segundo.
let t = env(); t.videos.push(video({ currentTime: 125.7 }));
t.advance(11 * 60 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(60 * MIN);
assert.deepStrictEqual(t.calls, ['replace https://www.youtube.com/watch?v=abc&t=125s']);
assert.strictEqual(t.storage['ytmp-maintenance-reload'], '1');

// Nunca recarga si hay un video sonando.
t = env(); t.videos.push(video({ paused: false }));
t.advance(13 * 60 * MIN); assert.deepStrictEqual(t.calls, []);

// Nunca recarga si hubo interacción en los últimos 15 min.
t = env(); t.clock.now = t.clock.perf = 12 * 60 * MIN; t.fire('keydown');
t.advance(10 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(10 * MIN); assert.strictEqual(t.calls.length, 1);

// El heap alto adelanta la recarga, pero no antes de 30 min.
t = env(); t.clock.heap = 900 * MB;
t.advance(20 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(15 * MIN); assert.deepStrictEqual(t.calls, ['reload']); // portada: URL igual → reload()

// Heap normal y menos de 12 h → nada.
t = env(); t.advance(6 * 60 * MIN); assert.deepStrictEqual(t.calls, []);

// Tras una recarga de mantenimiento, el autoplay se detiene durante 60 s...
t = env({ stored: { 'ytmp-maintenance-reload': '1' } });
assert.strictEqual(t.storage['ytmp-maintenance-reload'], undefined, 'la marca se consume');
let v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, 1);
t.clock.now += 61 * 1000; v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// ...salvo que el usuario haya interactuado.
t = env({ stored: { 'ytmp-maintenance-reload': '1' } }); t.fire('pointerdown');
v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// Sin marca, nunca pausa.
t = env(); v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// No se instala en accounts.google.com ni en iframes.
assert(!env({ href: 'https://accounts.google.com/signin' }).hasTick());
assert(!env({ top: false }).hasTick());

console.log('maintenance.js: todas las pruebas pasan');
