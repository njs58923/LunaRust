(() => {
  const ops = Deno.core.ops, waiting = new Map(), listeners = new Map();
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
    listen(targets, callback, options = {}) {
      if (typeof callback !== 'function') throw new TypeError('Expected a callback');
      if (!Array.isArray(targets)) targets = [targets];
      if (options.priority != null && !['input', 'normal'].includes(options.priority)) throw new TypeError('Invalid listen priority');
      if (options.delivery != null && !['latest', 'history'].includes(options.delivery)) throw new TypeError('Invalid listen delivery');
      const resolved = targets.map(target => {
        if (typeof target === 'string') return { key: target, node: null };
        if (!(target instanceof HSMLElement) || !target._isResolved()) throw new TypeError('Listen requires a resolved element');
        return { key: null, node: target.nodeId };
      });
      const id = ops.op_anchor_listen(resolved, options.priority === 'input', options.delivery === 'history');
      listeners.set(id, { callback, targets: targets.slice() });
      return Object.freeze({ dispose() { listeners.delete(id); ops.op_anchor_unlisten(id); } });
    },
  });
  const dimension = globalThis.dimension || (globalThis.dimension = {});
  Object.defineProperty(dimension, 'anchors', { value: anchors });
  Object.defineProperty(HSMLRootElement.prototype, 'anchors', { get() { return anchors; } });
  globalThis.__luna_anchor_input_pump = inputOnly => {
    for (const frame of ops.op_anchor_samples(inputOnly)) {
      const listener = listeners.get(frame.id);
      if (!listener) continue;
      for (const sample of frame.samples) {
        sample.target = listener.targets[sample.index];
        const p = sample.position, q = sample.rotation, s = sample.scale;
        sample.position = { x: p[0], y: p[1], z: p[2] };
        sample.rotation = { x: q[0], y: q[1], z: q[2], w: q[3] };
        sample.scale = { x: s[0], y: s[1], z: s[2] };
      }
      delete frame.id;
      try { listener.callback(frame); } catch (e) { console.error('anchors.listen:', e); }
    }
  };
  globalThis.__luna_anchor_pump = () => {
    for (const reply of ops.op_anchor_poll()) {
      const pending = waiting.get(reply.id);
      if (!pending) continue;
      waiting.delete(reply.id);
      if (reply.error) pending.reject(new Error(reply.error)); else pending.resolve(reply.id);
    }
  };
})();
