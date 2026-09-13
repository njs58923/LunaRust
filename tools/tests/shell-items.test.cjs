const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const vm = require('node:vm');
const source = readFileSync(join(__dirname, '../../crates/luna/src/web/internal/root_api.js'), 'utf8');
const KEY = 'luna.shell.items.v1';
function setup(storage = new Map()) {
  let next = 1;
  class Node {
    constructor(tag) { this.nodeId = next++; this.tagName = tag; this.children = []; this.attrs = {}; this.listeners = {}; this.replies = []; }
    setAttribute(k, v) { this.attrs[k] = String(v); }
    getAttribute(k) { return this.attrs[k] || ''; }
    appendChild(n) { this.children.push(n); n.parent = this; }
    remove() { this.parent.children = this.parent.children.filter(n => n !== this); this.parent = null; }
    createElement(tag) { return new Node(tag); }
    addEventListener(type, fn) { this.listeners[type] = fn; }
    send(name, data) { this.replies.push({ name, ...JSON.parse(JSON.stringify(data)) }); return Promise.resolve(); }
    request(action, data = {}) { this.listeners['component:shell-items-' + action]({ detail: data }); return this.replies.at(-1); }
  }
  const root = new Node('space');
  const localStorage = { getItem: k => storage.get(k) ?? null, setItem: (k, v) => storage.set(k, v) };
  const context = vm.createContext({ hiperspace: { dimention: root }, localStorage, requestAnimationFrame() {}, console: { log() {}, warn() {} } });
  vm.runInContext(source, context);
  const api = context.dimension.luna;
  function shell(style = 'flat') {
    api.switchMode('desktop', style);
    return root.children.find(n => n.getAttribute('system-shell') === 'true').children[0];
  }
  return { api, root, shell, storage, localStorage };
}
test('both controller styles share persisted entries, including extension fields', () => {
  const env = setup();
  const flat = env.shell();
  assert.equal(flat.props.shellItems.items.length, 5);
  const items = [{ id: 'custom', name: 'Otro mundo', url: 'https://example.test', appearance: { color: 'red' } }];
  assert.equal(flat.request('update', { revision: 0, items }).ok, true);
  const curved = env.shell('curved');
  assert.equal(curved.props.shellItems.items[0].appearance.color, 'red');
  assert.deepEqual(setup(env.storage).shell().request('get').items, items);
});
test('push preserves entries; conflicts and duplicate IDs do not change storage', () => {
  const env = setup(); const inc = env.shell();
  const pushed = inc.request('push', { revision: 0, items: [{ id: 'extra' }], requestId: 'a' });
  assert.equal(pushed.items.length, 6); assert.equal(pushed.requestId, 'a');
  const saved = env.storage.get(KEY);
  assert.equal(inc.request('update', { revision: 0, items: [] }).error, 'revision-conflict');
  assert.equal(inc.request('push', { revision: 1, items: [{ id: 'extra' }] }).ok, false);
  assert.equal(env.storage.get(KEY), saved);
});
test('empty list persists and ordinary worlds have no catalog channel', () => {
  const env = setup(); const inc = env.shell();
  assert.equal(inc.request('update', { revision: 0, items: [] }).ok, true);
  assert.equal(setup(env.storage).shell().props.shellItems.items.length, 0);
  env.api.mountSpace('https://example.test');
  const world = env.root.children.find(n => n.children.some(c => c.getAttribute('src') === 'https://example.test'));
  assert.equal(world.children[0].listeners['component:shell-items-update'], undefined);
});
test('storage failure rejects mutation and keeps the last published snapshot', () => {
  const env = setup(); const inc = env.shell();
  env.localStorage.setItem = () => { throw new Error('quota'); };
  const result = inc.request('update', { revision: 0, items: [] });
  assert.equal(result.ok, false); assert.equal(result.error, 'quota');
  assert.equal(inc.props.shellItems.items.length, 5);
  assert.equal(result.revision, 0);
});
test('corrupt saved data falls back without destroying it; oversized writes fail', () => {
  const env = setup(new Map([[KEY, '{broken']])); const inc = env.shell();
  assert.equal(inc.props.shellItems.items.length, 5);
  assert.equal(env.storage.get(KEY), '{broken');
  assert.equal(inc.request('update', { revision: 0, items: [{ id: 'huge', value: 'x'.repeat(8001) }] }).ok, false);
});
test('unmounted controllers cannot mutate root state', () => {
  const env = setup(); const old = env.shell(); const current = env.shell('curved');
  old.request('update', { revision: 0, items: [] });
  assert.equal(current.request('get').items.length, 5);
});
