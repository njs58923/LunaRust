(function(global) {
  'use strict';
  const ops = Deno.core.ops;
  const token = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
  const headerData = new WeakMap();
  function data(headers) {
    const d = headerData.get(headers);
    if (!d) throw new TypeError('Illegal invocation');
    return d;
  }
  function name(value) {
    value = String(value).toLowerCase();
    if (!token.test(value)) throw new TypeError('Invalid header name');
    return value;
  }
  function value(input) {
    const v = String(input).replace(/^[\t ]+|[\t ]+$/g, '');
    if (/[\x00-\x08\x0A-\x1F\x7F\u0100-\uFFFF]/.test(v)) throw new TypeError('Invalid header value');
    return v;
  }
  function writable(headers) { if (data(headers).immutable) throw new TypeError('Response headers are immutable'); }
  class Headers {
    constructor(init = undefined) {
      headerData.set(this, {map:new Map(), immutable:false});
      if (init == null) return;
      if (typeof init[Symbol.iterator] === 'function') {
        for (const pair of init) {
          if (typeof pair === 'string' || pair == null || typeof pair[Symbol.iterator] !== 'function') throw new TypeError('Invalid header pair');
          const entries = [...pair];
          if (entries.length !== 2) throw new TypeError('Expected header name and value');
          this.append(entries[0], entries[1]);
        }
      } else if (typeof init === 'object') {
        for (const key of Object.keys(init)) this.append(key, init[key]);
      } else throw new TypeError('Invalid headers');
    }
    append(key, val) { writable(this); key = name(key); val = value(val); const m=data(this).map; m.set(key,m.has(key)?m.get(key)+', '+val:val); }
    set(key, val) { writable(this); data(this).map.set(name(key),value(val)); }
    get(key) { return data(this).map.get(name(key)) ?? null; }
    has(key) { return data(this).map.has(name(key)); }
    delete(key) { writable(this); data(this).map.delete(name(key)); }
    *entries() { yield* [...data(this).map.entries()].sort((a,b)=>a[0]<b[0]?-1:a[0]>b[0]?1:0); }
    *keys() { for (const [key] of this) yield key; }
    *values() { for (const [,val] of this) yield val; }
    [Symbol.iterator]() { return this.entries(); }
    forEach(callback, thisArg) { for (const [key,val] of this) callback.call(thisArg,val,key,this); }
    get [Symbol.toStringTag]() { return 'Headers'; }
  }
  function response(payload) {
    let used = false;
    const headers = new Headers(payload.headers);
    data(headers).immutable = true;
    async function consume() {
      if (used) throw new TypeError('Response body already consumed');
      used = true;
      return payload.body;
    }
    return Object.freeze({
      url:payload.url, status:payload.status, statusText:payload.statusText,
      ok:payload.status >= 200 && payload.status < 300,
      redirected:payload.redirected, headers,
      get bodyUsed() { return used; },
      async arrayBuffer() { return (await consume()).slice().buffer; },
      async bytes() { return (await consume()).slice(); },
      async blob() { return new Blob([await consume()],{type:headers.get("content-type") || ""}); },
      async text() { return new TextDecoder().decode(await consume()); },
      async json() { return JSON.parse(new TextDecoder().decode(await consume())); },
      clone() { if (used) throw new TypeError('Response body already consumed'); return response(payload); },
      [Symbol.toStringTag]:'Response'
    });
  }
  const supported = new Set(['method','headers','body','redirect','credentials']);
  global.Headers = Headers;
  global.fetch = function(input, options = {}) {
    return new Promise((resolve,reject) => {
      options = options ?? {};
      for (const key of Object.keys(options)) if (!supported.has(key)) throw new TypeError('Unsupported fetch option: '+key);
      if (options.credentials !== undefined && options.credentials !== 'omit') throw new TypeError('Luna fetch currently supports credentials: omit only');
      const method = String(options.method ?? 'GET').toUpperCase();
      if (!token.test(method) || ['CONNECT','TRACE','TRACK'].includes(method)) throw new TypeError('Invalid fetch method');
      const redirect = options.redirect ?? 'follow';
      if (!['follow','error'].includes(redirect)) throw new TypeError('Unsupported redirect mode');
      const headers = new Headers(options.headers);
      let body = options.body ?? null;
      let binary = null;
      if (body instanceof Blob) {
        if (body.type && !headers.has("content-type")) headers.set("content-type",body.type);
        binary=__lunaBinary.blobBytes(body);body=null;
      } else if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) { binary=__lunaBinary.view(body);body=null; }
      if (body instanceof URLSearchParams) {
        body = body.toString();
        if (!headers.has('content-type')) headers.set('content-type','application/x-www-form-urlencoded;charset=UTF-8');
      } else if (body !== null) {
        if (typeof body !== 'string') throw new TypeError('Body must be a string or URLSearchParams; use JSON.stringify for JSON');
        if (!headers.has('content-type')) headers.set('content-type','text/plain;charset=UTF-8');
      }
      if ((body !== null || binary !== null) && ['GET','HEAD'].includes(method)) throw new TypeError('GET/HEAD cannot have a body');
      const url = new URL(String(input), location.href).href;
      if(url.startsWith('blob:')) {
        const blob=__lunaBinary.resolveBlob(url);
        if(!blob || method!=='GET') throw new TypeError('Unknown Blob URL or unsupported method');
        resolve(response({url,status:200,statusText:'OK',headers:[['content-type',blob.type]],body:__lunaBinary.blobBytes(blob),redirected:false}));return;
      }
      const id = ops.op_fetch_request({url,method,headers:[...headers],body,redirect},binary || new Uint8Array(),binary !== null);
      function poll() {
        try {
          const result = ops.op_fetch_poll(id);
          if (result.status === 'pending') setTimeout(poll,10);
          else if (result.status === 'ok') resolve(response(result.response));
          else reject(new TypeError(result.error));
        } catch (error) { reject(error); }
      }
      poll();
    });
  };
})(globalThis);
