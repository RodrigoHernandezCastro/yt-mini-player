const fs = require('fs'), vm = require('vm'), assert = require('assert');
const src = fs.readFileSync(require('path').join(__dirname, '..', 'src', 'inject.js'), 'utf8');

function env() {
  const listeners = {}, sent = [], attrs = {};
  const root = { setAttribute:(k,v)=>attrs[k]=v, removeAttribute:k=>delete attrs[k], appendChild(){} };
  const window = {
    innerWidth: 640, innerHeight: 360, sent,
    addEventListener:(t,f,o)=>(listeners[t] ||= []).push(f),
    ipc: { postMessage: m => sent.push(m) },
  };
  window.top = window;
  const document = { readyState:'complete', documentElement: root, head: root,
    createElement:()=>({}), addEventListener(){}, querySelector:()=>null };
  class Element {}
  vm.runInNewContext(src, { window, document, Element, setTimeout, PointerEvent: class {}, MouseEvent: class {} });
  const fire = (t, props) => {
    const e = { defaultPrevented:false, stopped:false, altKey:false, button:0, buttons:0,
      pointerType:'mouse', clientX:320, clientY:180, ...props,
      preventDefault(){this.defaultPrevented=true}, stopImmediatePropagation(){this.stopped=true} };
    (listeners[t]||[]).forEach(f=>f(e)); return e;
  };
  return { fire, sent, attrs };
}

// Alt+clic en el centro → drag, y se consume el click posterior
let t = env();
let e = t.fire('pointerdown', { altKey:true });
assert.deepStrictEqual(t.sent, ['drag']); assert(e.defaultPrevented && e.stopped);
assert(t.fire('mousedown', {altKey:true}).stopped);
assert(t.fire('click', { altKey:true }).stopped);
assert(!t.fire('click', {}).stopped, 'solo se consume un click');

// Clic normal en el centro → nada
t = env(); e = t.fire('pointerdown', {});
assert.deepStrictEqual(t.sent, []); assert(!e.defaultPrevented);
assert(!t.fire('click', {}).stopped);

// Bordes y esquinas
const cases = [[320,2,'n'],[320,357,'s'],[2,180,'w'],[637,180,'e'],
  [2,2,'nw'],[10,2,'nw'],[2,10,'nw'],[637,2,'ne'],[2,357,'sw'],[637,357,'se'],[20,2,'n'],[320,6,null]];
for (const [x,y,z] of cases) {
  t = env(); t.fire('pointerdown', { clientX:x, clientY:y });
  assert.deepStrictEqual(t.sent, z ? ['resize:'+z] : [], `${x},${y}`);
}

// Alt tiene prioridad sobre el borde; botón derecho y táctil se ignoran
t = env(); t.fire('pointerdown', { clientX:2, clientY:2, altKey:true }); assert.deepStrictEqual(t.sent, ['drag']);
t = env(); t.fire('pointerdown', { button:2, altKey:true }); assert.deepStrictEqual(t.sent, []);
t = env(); t.fire('pointerdown', { pointerType:'touch', altKey:true }); assert.deepStrictEqual(t.sent, []);

// Cursor: atributo solo en bordes, se limpia al salir
t = env();
t.fire('pointermove', { clientX:637, clientY:180 }); assert.strictEqual(t.attrs['data-ytmp-resize'], 'e');
t.fire('pointermove', { clientX:320, clientY:180 }); assert(!('data-ytmp-resize' in t.attrs));
t.fire('pointermove', { clientX:2, clientY:2 }); t.fire('mouseout', { relatedTarget:null });
assert(!('data-ytmp-resize' in t.attrs));

// dblclick con Alt no llega a YouTube (pantalla completa)
t = env(); assert(t.fire('dblclick', { altKey:true }).stopped);

// iframe: no se instala
{ const l={}; const w={addEventListener:(t,f)=>(l[t]||=[]).push(f)}; w.top={};
  vm.runInNewContext(src,{window:w,document:{}}); assert.strictEqual(Object.keys(l).length,0); }

console.log('inject.js: todas las pruebas pasan');
