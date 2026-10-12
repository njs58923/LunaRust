(() => {
  const ops = Deno.core.ops, waiting = new Map();
  function request(key, native, node, remove, expected = 0) {
    if (waiting.size >= 64) return Promise.reject(new Error('Too many pending anchor requests'));
    return new Promise((resolve, reject) => {
      const id = ops.op_anchor_request(String(key), native, node, remove, expected);
      waiting.set(id, { resolve, reject });
    });
  }
  const anchors = Object.freeze({
    async create(key, source) {
      let registration;
      if (typeof source === 'string') {
        if (!source.startsWith('__')) throw new TypeError('Native references start with __');
        registration = await request(key, source, -1, false);
      } else {
        if (!(source instanceof HSMLElement)) throw new TypeError('Expected an element or native reference');
        if (!source._isResolved()) await new Promise(resolve => source._onResolved(resolve));
        registration = await request(key, '', source.nodeId, false);
      }
      return Object.freeze({ key: String(key), dispose: () => request(key, '', -1, true, registration) });
    },
    remove(key) { return request(key, '', -1, true); },
  });
  const dimension = globalThis.dimension || (globalThis.dimension = {});
  Object.defineProperty(dimension, 'anchors', { value: anchors });
  Object.defineProperty(HSMLRootElement.prototype, 'anchors', { get() { return anchors; } });
  globalThis.__luna_anchor_pump = () => {
    for (const reply of ops.op_anchor_poll()) {
      const pending = waiting.get(reply.id);
      if (!pending) continue;
      waiting.delete(reply.id);
      if (reply.error) pending.reject(new Error(reply.error)); else pending.resolve(reply.id);
    }
  };
})();
