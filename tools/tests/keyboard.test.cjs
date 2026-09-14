const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');
const base = join(__dirname, '../..');
function setup() {
  let inbox = [], commands = [], frames = [];
  const listeners = new Map();
  const root = { createElement: () => node(), getElementById: () => node(),
    dispatchEvent(e) { for (const f of listeners.get(e.type)||[]) f(e); } };
  function node() { return {setAttribute(){}, appendChild(){},addEventListener(){},remove(){}}; }
  const ctx = vm.createContext({console,hiperspace:{dimention:root},UI_CFG:{},
    Deno:{core:{ops:{op_keyboard_command(v){commands.push(v);},op_keyboard_read(){const q=inbox;inbox=[];return q;}}}},
    requestAnimationFrame:f=>{frames.push(f);return frames.length;},cancelAnimationFrame(){},setTimeout,clearTimeout});
  for (const path of ['crates/js_runtime/keyboard.js','crates/luna/src/web/internal/ui.js']) vm.runInContext(readFileSync(join(base,path),'utf8'),ctx);
  const app = {datos:{value:''},manejadores:{},invalidar(){},invalidateRender(){},focus(c){ctx.keyboard.focus(c,{editable:true});},focusNext(){}};
  const box = new ctx.UI.tipos.TextBox({Text:'{Binding value, Mode=TwoWay}'});
  box.aplicarEnlaces(app.datos,app);app.focus(box);
  function input(packet) { inbox.push({type:'input',packet});ctx.__luna_keyboard_pump(); }
  return {ctx,box,app,commands,frames,listeners,input,state(s){inbox.push({type:'state',...s});ctx.__luna_keyboard_pump();}};
}
test('typing updates TwoWay once, preserves Unicode, and redraws only on demand', () => {
  const e=setup();let before=0, inputs=0;
  e.box.addEventListener('beforeinput',()=>before++);e.box.addEventListener('input',()=>inputs++);
  e.input({type:'keydown',key:'ñ',code:'KeyN',text:'ñ😀'});
  assert.equal(e.box.text,'ñ😀');assert.equal(e.app.datos.value,'ñ😀');assert.equal(before,1);assert.equal(inputs,1);
  e.input({type:'keydown',key:'Backspace'});assert.equal(e.box.text,'ñ');
  assert.equal(e.frames.length,0);
});
test('keydown and beforeinput cancellation prevent editing, including at document root', () => {
  const e=setup();e.listeners.set('keydown',[event=>event.preventDefault()]);
  e.input({type:'keydown',key:'a',text:'a'});assert.equal(e.box.text,'');
  e.listeners.clear();e.box.addEventListener('beforeinput',event=>event.preventDefault());
  e.input({type:'keydown',key:'b',text:'b'});assert.equal(e.box.text,'');
});
test('selection, replacement, undo/redo, readonly and Enter commit', () => {
  const e=setup();let committed=0;e.box.addEventListener('change',()=>committed++);
  e.input({type:'keydown',key:'a',text:'abc'});e.box.setSelectionRange(1,2);
  e.input({type:'keydown',key:'x',text:'X'});assert.equal(e.box.text,'aXc');
  e.input({type:'keydown',key:'z',ctrlKey:true});assert.equal(e.box.text,'abc');
  e.input({type:'keydown',key:'y',ctrlKey:true});assert.equal(e.box.text,'aXc');
  e.box.soloLectura=true;e.input({type:'keydown',key:'Backspace'});assert.equal(e.box.text,'aXc');
  e.input({type:'keydown',key:'Enter'});assert.equal(committed,1);assert.equal(e.ctx.keyboard.activeElement,null);
});
test('focus loss releases held keys with keyup and commits once', () => {
  const e=setup();const events=[];e.box.addEventListener('keyup',v=>events.push(v.type+':'+v.key));
  e.input({type:'keydown',key:'Shift',code:'ShiftLeft'});e.state({focused:false});
  assert.deepEqual(events,['keyup:Shift']);assert.equal(e.ctx.keyboard.activeElement,null);
});
test('composition commits once and clipboard shortcuts preserve selection', () => {
  const e=setup();e.input({type:'compositionstart'});e.input({type:'compositionupdate',text:'に'});
  assert.equal(e.box.text,'');e.input({type:'compositionend',text:'日'});e.input({type:'text',text:'日'});
  assert.equal(e.box.text,'日');e.box.select();
  e.input({type:'keydown',key:'v',ctrlKey:true,clipboardText:'https://ejemplo.test/'});
  assert.equal(e.box.text,'https://ejemplo.test/');e.box.select();
  e.input({type:'keydown',key:'c',ctrlKey:true});assert.equal(e.commands.at(-1).action,'clipboard');
});
