const {test}=require('node:test');
const assert=require('node:assert/strict');
const vm=require('node:vm');
const fs=require('node:fs');
const path=require('node:path');
function setup(){
  let now=1000,id=0;const frames=new Map(),nodes=[],sent=[];
  const group={appendChild(){}};
  const root={getElementById(){return group;},createElement(tag){
    const n={tag,attrs:{},listeners:{},setAttribute(k,v){this.attrs[k]=v;},addEventListener(k,f){this.listeners[k]=f;}};
    nodes.push(n);return n;
  }};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname,'../../crates/luna/src/web/internal/keyboard_virtual.js'),'utf8'),{
    hiperspace:{dimention:root},component:{props:{revision:7},emit(type,data){sent.push({type,data});return Promise.resolve();},addEventListener(){}},
    Date:{now:()=>now},console,requestAnimationFrame(f){frames.set(++id,f);return id;},cancelAnimationFrame(id){frames.delete(id);}
  });
  return {nodes,sent,frames,key(label){const index=nodes.findIndex(n=>n.tag==='text'&&n.attrs.value===label&&n.attrs.visible!=='false');return nodes[index-1];},
    advance(ms){now+=ms;const work=[...frames.values()];frames.clear();work.forEach(f=>f());}};
}
test('hover and click animate then stop; both pointers retain hover',()=>{
  const e=setup(),key=e.key('q'),rest=key.attrs.color;
  assert.equal(e.frames.size,0);
  key.listeners.pointerenter({pointerId:1});e.advance(130);
  assert.notEqual(key.attrs.color,rest);assert.equal(e.frames.size,0);
  key.listeners.pointerenter({pointerId:2});key.listeners.pointerleave({pointerId:1});e.advance(130);
  assert.notEqual(key.attrs.color,rest);
  key.listeners.toque();e.advance(16);assert.ok(Number(key.attrs.z)<0.022);
  assert.equal(e.sent[0].data.packet.type,'keydown');assert.equal(e.sent[1].data.packet.type,'keyup');
  e.advance(200);assert.equal(e.frames.size,0);
  key.listeners.pointerleave({pointerId:2});e.advance(130);assert.equal(key.attrs.color,rest);
});
test('Shift and layout changes reuse key nodes',()=>{
  const e=setup(),count=e.nodes.length;
  e.key('⇧').listeners.toque();assert.ok(e.key('Q'));assert.equal(e.nodes.length,count);
  e.key('123').listeners.toque();assert.ok(e.key('1'));assert.equal(e.nodes.length,count);
  e.key('ABC').listeners.toque();assert.ok(e.key('Q'));assert.equal(e.nodes.length,count);
});
