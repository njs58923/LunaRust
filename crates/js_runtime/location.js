// URL parsing is shared with the host. Document identity stays in Rust.
(function(global) {
  'use strict';
  const ops = Deno.core.ops;
  function text(value) {
    if (typeof value === 'symbol') throw new TypeError('Cannot convert Symbol to string');
    return String(value).replace(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g, '\uFFFD');
  }
  function required(count, minimum) {
    if (count < minimum) throw new TypeError('Not enough arguments');
  }
  const paramsState = new WeakMap();
  function state(params) {
    const value = paramsState.get(params);
    if (!value) throw new TypeError('Illegal invocation');
    return value;
  }
  function changed(params) { state(params).update?.(params.toString()); }
  class URLSearchParams {
    constructor(init = '') {
      let pairs;
      if (init !== null && typeof init === 'object') {
        if (typeof init[Symbol.iterator] === 'function') {
          pairs = Array.from(init, pair => {
            if (pair == null || typeof pair === 'string' || typeof pair[Symbol.iterator] !== 'function')
              throw new TypeError('Expected name/value pair');
            const values = Array.from(pair);
            if (values.length !== 2) throw new TypeError('Expected two values');
            return values.map(text);
          });
        } else { pairs = Object.keys(init).map(key => [text(key), text(init[key])]); }
      } else { pairs = ops.op_query_parse(text(init ?? '').replace(/^\?/, '')); }
      paramsState.set(this, {pairs, update:null});
    }
    get size() { return state(this).pairs.length; }
    append(name, value) { required(arguments.length, 2); state(this).pairs.push([text(name), text(value)]); changed(this); }
    get(name) { required(arguments.length, 1); name = text(name); return state(this).pairs.find(p => p[0] === name)?.[1] ?? null; }
    getAll(name) { required(arguments.length, 1); name = text(name); return state(this).pairs.filter(p => p[0] === name).map(p => p[1]); }
    has(name, value) {
      required(arguments.length, 1); name = text(name);
      const matchValue = value !== undefined;
      if (matchValue) value = text(value);
      return state(this).pairs.some(p => p[0] === name && (!matchValue || p[1] === value));
    }
    delete(name, value) {
      required(arguments.length, 1); name = text(name);
      const matchValue = value !== undefined;
      if (matchValue) value = text(value);
      state(this).pairs = state(this).pairs.filter(p => p[0] !== name || (matchValue && p[1] !== value));
      changed(this);
    }
    set(name, value) {
      required(arguments.length, 2); name = text(name); value = text(value);
      let found = false;
      state(this).pairs = state(this).pairs.filter(p => {
        if (p[0] !== name) return true;
        if (found) return false;
        p[1] = value; found = true; return true;
      });
      if (!found) state(this).pairs.push([name, value]);
      changed(this);
    }
    sort() { state(this).pairs.sort((a,b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0); changed(this); }
    *entries() { for (let i = 0; i < state(this).pairs.length; i++) yield [...state(this).pairs[i]]; }
    *keys() { for (const [name] of this.entries()) yield name; }
    *values() { for (const [,value] of this.entries()) yield value; }
    [Symbol.iterator]() { return this.entries(); }
    forEach(callback, thisArg) { if (typeof callback !== 'function') throw new TypeError('Expected callback'); for (const [name,value] of this.entries()) callback.call(thisArg, value, name, this); }
    toString() { return ops.op_query_encode(state(this).pairs); }
    get [Symbol.toStringTag]() { return 'URLSearchParams'; }
  }
  function parse(input, base = '', part = '', value = '') {
    const result = ops.op_url_parse(input, base, part, value);
    if (result.error) throw new TypeError(result.error);
    return result;
  }
  const urls = new WeakMap();
  function urlState(url) {
    const data = urls.get(url);
    if (!data) throw new TypeError('Illegal invocation');
    return data;
  }
  function replaceParts(url, parts) {
    const data = urlState(url);
    data.parts = parts;
    state(data.params).pairs = ops.op_query_parse(parts.search.replace(/^\?/, ''));
  }
  class URL {
    constructor(input, base) {
      required(arguments.length, 1);
      const parts = parse(text(input), base === undefined ? '' : text(base));
      const params = new URLSearchParams(parts.search);
      urls.set(this, {parts, params});
      state(params).update = search => { urlState(this).parts = parse(this.href, '', 'search', search); };
    }
    get href() { return urlState(this).parts.href; }
    set href(value) { replaceParts(this, parse(text(value))); }
    get origin() { return urlState(this).parts.origin; }
    get searchParams() { return urlState(this).params; }
    toString() { return this.href; }
    toJSON() { return this.href; }
    static canParse(input, base) { required(arguments.length, 1); try { new URL(input, base); return true; } catch { return false; } }
    get [Symbol.toStringTag]() { return 'URL'; }
  }
  const fields = ['protocol','host','hostname','port','pathname','search','hash','username','password'];
  for (const field of fields) Object.defineProperty(URL.prototype, field, {
    enumerable:true, configurable:true,
    get() { return urlState(this).parts[field]; },
    set(value) { replaceParts(this, parse(this.href, '', field, text(value))); }
  });

  function current() { return new URL(ops.op_document_location()); }
  function navigate(value) {
    const target = new URL(text(value), ops.op_document_location());
    ops.op_navigate(target.href);
    // A request is not a committed navigation. In particular, blocked requests
    // must never change the URL used by subsequent reads or relative links.
  }
  const location = {
    get href() { return ops.op_document_location(); },
    set href(value) { navigate(value); },
    get origin() { return current().origin; },
    assign(value) { required(arguments.length, 1); navigate(value); },
    // Luna currently has no session-history distinction between these methods.
    replace(value) { required(arguments.length, 1); navigate(value); },
    reload() { navigate(ops.op_document_location()); },
    toString() { return this.href; },
    get [Symbol.toStringTag]() { return 'Location'; }
  };
  for (const field of fields.filter(f => f !== 'username' && f !== 'password')) {
    Object.defineProperty(location, field, {
      enumerable:true,
      get() { return current()[field]; },
      set(value) { const url = current(); url[field] = value; navigate(url.href); }
    });
  }
  global.URL = URL;
  global.URLSearchParams = URLSearchParams;
  class Location { constructor() { throw new TypeError('Illegal constructor'); } }
  Object.setPrototypeOf(location, Location.prototype);
  global.Location = Location;
  // Keep the shared Location object; assigning window.location requests a URL.
  Object.defineProperty(global, 'location', {
    enumerable:true, configurable:false,
    get() { return location; }, set(value) { navigate(value); }
  });
})(globalThis);
