// Web Storage facade. Origin and database path are never accepted from JavaScript.
(function(global) {
  'use strict';
  const op = Deno.core.ops.op_local_storage;
  if (!global.DOMException) {
    global.DOMException = class DOMException extends Error {
      constructor(message = '', name = 'Error') { super(message); this.name = name; this.code = name === 'QuotaExceededError' ? 22 : name === 'SecurityError' ? 18 : 0; }
    };
  }
  function call(action, key = '', value = '') {
    const result = op(action, key, value);
    if (result.error) throw new DOMException('localStorage: ' + result.error, result.error);
    return result.value;
  }
  function str(value) { if (typeof value === 'symbol') throw new TypeError('Cannot convert a Symbol to a string'); return String(value); }
  // JSON string encoding preserves lone UTF-16 surrogates across the Rust boundary.
  const encode = value => JSON.stringify(str(value));
  const decode = value => value === null ? null : JSON.parse(value);
  const brands = new WeakSet();
  function check(receiver, count, required) {
    if (!brands.has(receiver)) throw new TypeError('Illegal invocation');
    if (count < required) throw new TypeError('Not enough arguments');
  }
  class Storage {
    constructor() { throw new TypeError('Illegal constructor'); }
    get length() { check(this, 0, 0); return call('length'); }
    key(index) {
      check(this, arguments.length, 1);
      return decode(call('keys')[(+index) >>> 0] ?? null);
    }
    getItem(key) { check(this, arguments.length, 1); return decode(call('get', encode(key))); }
    setItem(key, value) { check(this, arguments.length, 2); call('set', encode(key), encode(value)); }
    removeItem(key) { check(this, arguments.length, 1); call('remove', encode(key)); }
    clear() { check(this, 0, 0); call('clear'); }
    get [Symbol.toStringTag]() { return 'Storage'; }
  }
  const target = Object.create(Storage.prototype);
  const storage = new Proxy(target, {
    get(t, key, receiver) {
      if (typeof key === 'symbol' || key in t) return Reflect.get(t, key, receiver);
      return decode(call('get', encode(key))) ?? undefined;
    },
    set(t, key, value) {
      if (typeof key === 'symbol') return Reflect.set(t, key, value);
      call('set', encode(key), encode(value)); return true;
    },
    deleteProperty(t, key) {
      if (typeof key === 'symbol') return Reflect.deleteProperty(t, key);
      call('remove', encode(key)); return true;
    },
    has(t, key) { return key in t || (typeof key === 'string' && call('get', encode(key)) !== null); },
    ownKeys(t) { return [...Reflect.ownKeys(t), ...call('keys').map(decode).filter(k => !(k in t))]; },
    getOwnPropertyDescriptor(t, key) {
      const own = Reflect.getOwnPropertyDescriptor(t, key);
      if (own || typeof key !== 'string' || key in t) return own;
      const value = call('get', encode(key));
      return value === null ? undefined : {value:decode(value), writable:true, enumerable:true, configurable:true};
    },
    defineProperty(t, key, descriptor) {
      if (typeof key === 'symbol') return Reflect.defineProperty(t, key, descriptor);
      if (!('value' in descriptor) || descriptor.configurable === false) return false;
      call('set', encode(key), encode(descriptor.value)); return true;
    },
    preventExtensions() { return false; },
  });
  brands.add(storage);
  global.Storage = Storage;
  Object.defineProperty(global, 'localStorage', {enumerable:true, configurable:true, get() { call('check'); return storage; }});

  class StorageEvent {
    constructor(type, init = {}) {
      this.type = str(type); this.key = init.key ?? null;
      this.oldValue = init.oldValue ?? null; this.newValue = init.newValue ?? null;
      this.url = init.url ?? ''; this.storageArea = init.storageArea ?? null;
      this.bubbles = false; this.cancelable = false; this.defaultPrevented = false;
    }
  }
  global.StorageEvent = global.StorageEvent || StorageEvent;
  const listeners = new Map();
  let handler = null, timer = null, watching = false;
  function hasListeners() { return !!handler || (listeners.get('storage') || []).length > 0; }
  function updateWatch() {
    const enabled = hasListeners();
    if (enabled !== watching) { call('watch', '', enabled ? 'true' : 'false'); watching = enabled; }
    if (enabled && timer === null) timer = setTimeout(poll, 20);
    if (!enabled && timer !== null) { clearTimeout(timer); timer = null; }
  }
  function poll() {
    timer = null;
    for (const change of call('events')) {
      global.dispatchEvent(new global.StorageEvent('storage', {
        key:decode(change.key), oldValue:decode(change.oldValue), newValue:decode(change.newValue),
        url:change.url, storageArea:storage,
      }));
    }
    updateWatch();
  }
  global.addEventListener = function(type, listener, options = {}) {
    if (!listener) return;
    type = str(type);
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    const list = listeners.get(type) || [];
    if (!list.some(e => e.listener === listener && e.capture === capture)) {
      list.push({listener, capture, once:!!options?.once}); listeners.set(type, list);
    }
    if (type === 'storage') updateWatch();
  };
  global.removeEventListener = function(type, listener, options = {}) {
    type = str(type);
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    listeners.set(type, (listeners.get(type) || []).filter(e => e.listener !== listener || e.capture !== capture));
    if (type === 'storage') updateWatch();
  };
  global.dispatchEvent = function(event) {
    for (const entry of [...(listeners.get(event.type) || [])]) {
      if (!(listeners.get(event.type) || []).includes(entry)) continue;
      if (entry.once) global.removeEventListener(event.type, entry.listener, entry.capture);
      try {
        if (typeof entry.listener === 'function') entry.listener.call(global,event);
        else entry.listener.handleEvent(event);
      } catch (error) { console.error(error); }
    }
    if (event.type === 'storage' && handler) {
      try { handler.call(global,event); } catch (error) { console.error(error); }
    }
    return !event.defaultPrevented;
  };
  Object.defineProperty(global, 'onstorage', {configurable:true, enumerable:true,
    get() { return handler; }, set(value) { handler = typeof value === 'function' ? value : null; updateWatch(); },
  });
})(globalThis);
