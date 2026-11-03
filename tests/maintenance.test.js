// Tests of src/maintenance.js with a simulated clock, DOM and storage.
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

// With no playback or interaction, reloads at 12 h keeping the second.
let t = env(); t.videos.push(video({ currentTime: 125.7 }));
t.advance(11 * 60 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(60 * MIN);
assert.deepStrictEqual(t.calls, ['replace https://www.youtube.com/watch?v=abc&t=125s']);
assert.strictEqual(t.storage['ytmp-maintenance-reload'], '1');

// Never reloads while a video is playing.
t = env(); t.videos.push(video({ paused: false }));
t.advance(13 * 60 * MIN); assert.deepStrictEqual(t.calls, []);

// Never reloads if there was interaction in the last 15 min.
t = env(); t.clock.now = t.clock.perf = 12 * 60 * MIN; t.fire('keydown');
t.advance(10 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(10 * MIN); assert.strictEqual(t.calls.length, 1);

// A big heap brings the reload forward, but not before 30 min.
t = env(); t.clock.heap = 900 * MB;
t.advance(20 * MIN); assert.deepStrictEqual(t.calls, []);
t.advance(15 * MIN); assert.deepStrictEqual(t.calls, ['reload']); // home page: same URL → reload()

// Normal heap and under 12 h → nothing.
t = env(); t.advance(6 * 60 * MIN); assert.deepStrictEqual(t.calls, []);

// After an upkeep reload, autoplay is stopped for 60 s...
t = env({ stored: { 'ytmp-maintenance-reload': '1' } });
assert.strictEqual(t.storage['ytmp-maintenance-reload'], undefined, 'the mark is used up');
let v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, 1);
t.clock.now += 61 * 1000; v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// ...unless the user has interacted.
t = env({ stored: { 'ytmp-maintenance-reload': '1' } }); t.fire('pointerdown');
v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// Without the mark, it never pauses.
t = env(); v = video({ paused: false }); t.fire('play', { target: v }); assert.strictEqual(v.pauses, undefined);

// Not installed on accounts.google.com or in iframes.
assert(!env({ href: 'https://accounts.google.com/signin' }).hasTick());
assert(!env({ top: false }).hasTick());

console.log('maintenance.js: all tests pass');
