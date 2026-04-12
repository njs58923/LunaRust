// Luna Runtime - Browser-like HSML DOM API
// Version: 0.1.0
// Architecture: Rust ops backend, JS proxy frontend

(function(global) {
  'use strict';

  const core = Deno.core;

  const __lunaElementCache = new Map();

  function _findNodeById(el, targetId) {
    if (!el) return null;
    if (el.nodeId === targetId) return el;
    const children = el.children || [];
    for (const child of children) {
      const found = _findNodeById(child, targetId);
      if (found) return found;
    }
    return null;
  }

  global.__luna_dispatch_dom_events = function(events) {
    const root = global.hiperspace && global.hiperspace.dimention;
    if (!root) return;

    for (const evt of events) {
      const target = _findNodeById(root, evt.nodeId);
      if (!target) continue;

      const normalized = {
        type: String(evt.type || ''),
        nodeId: evt.nodeId,
      };
      if (evt.x != null) normalized.x = evt.x;
      if (evt.y != null) normalized.y = evt.y;
      if (evt.z != null) normalized.z = evt.z;

      target.dispatchEvent(normalized);
    }
  };

  // ---------------------------------------------------------------------------
  // Helper: Poll async ops (createElement, fetch)
  // ---------------------------------------------------------------------------

  function resolveCreatedElementAsync(requestId, onReady, attempts = 0, maxAttempts = 600) {
    const result = core.ops.op_hsml_poll_created_element(requestId);
    if (result && result !== -1) {
      onReady(result);
      return;
    }
    if (attempts >= maxAttempts) {
      console.error(`[Luna Runtime] createElement timeout for request ${requestId}`);
      return;
    }
    setTimeout(() => resolveCreatedElementAsync(requestId, onReady, attempts + 1, maxAttempts), 0);
  }

  // ---------------------------------------------------------------------------
  // Vec3 Proxy Helper
  // ---------------------------------------------------------------------------

  class ProxyVec3 {
    constructor(getOp, setOp, nodeId) {
      this._getOp = getOp;
      this._setOp = setOp;
      this._nodeId = nodeId;
      this._cache = null;
    }

    _refresh() {
      const arr = this._getOp(this._nodeId);
      this._cache = { x: arr[0], y: arr[1], z: arr[2] };
      return this._cache;
    }

    get x() { return (this._cache || this._refresh()).x; }
    set x(v) {
      const curr = this._cache || this._refresh();
      this._setOp(this._nodeId, v, curr.y, curr.z);
      this._cache = { x: v, y: curr.y, z: curr.z };
    }

    get y() { return (this._cache || this._refresh()).y; }
    set y(v) {
      const curr = this._cache || this._refresh();
      this._setOp(this._nodeId, curr.x, v, curr.z);
      this._cache = { x: curr.x, y: v, z: curr.z };
    }

    get z() { return (this._cache || this._refresh()).z; }
    set z(v) {
      const curr = this._cache || this._refresh();
      this._setOp(this._nodeId, curr.x, curr.y, v);
      this._cache = { x: curr.x, y: curr.y, z: v };
    }

    toJSON() {
      const v = this._cache || this._refresh();
      return { x: v.x, y: v.y, z: v.z };
    }
  }

  // ---------------------------------------------------------------------------
  // DOMTokenList (classList)
  // ---------------------------------------------------------------------------

  class DOMTokenList extends Array {
    constructor(getClassNameFn, setClassNameFn) {
      super();
      this._get = getClassNameFn;
      this._set = setClassNameFn;
      const current = this._get() || '';
      this.push(...current.split(' ').filter(s => s.length > 0));
    }

    _save() {
      this._set(this.join(' '));
    }

    add(name) {
      if (!this.contains(name)) {
        Array.prototype.push.call(this, name);
        this._save();
      }
    }

    remove(name) {
      const idx = this.indexOf(name);
      if (idx !== -1) {
        this.splice(idx, 1);
        this._save();
      }
    }

    contains(name) {
      return this.indexOf(name) !== -1;
    }

    toggle(name) {
      if (this.contains(name)) {
        this.remove(name);
        return false;
      } else {
        this.add(name);
        return true;
      }
    }
  }

  // ---------------------------------------------------------------------------
  // HSMLElement (base class)
  // ---------------------------------------------------------------------------

  class HSMLElement {
    constructor(nodeId) {
      if (typeof nodeId !== 'number') {
        throw new Error(`Invalid node ID: ${nodeId}`);
      }
      this._nodeId = nodeId;
      this._tagHint = '';
      this._positionProxy = null;
      this._rotationProxy = null;
      this._scaleProxy = null;
      this._globalPositionProxy = null;
      this._classList = null;
      this._resolveCallbacks = [];
      this._eventListeners = new Map();
      this._eventHandlers = Object.create(null);
    }

    _isResolved() {
      return this._nodeId >= 0;
    }

    _setTagHint(tagName) {
      this._tagHint = String(tagName || '');
    }

    _onResolved(fn) {
      if (this._isResolved()) {
        fn(this._nodeId);
        return;
      }
      this._resolveCallbacks.push(fn);
    }

    _resolveNodeId(nodeId) {
      if (this._isResolved()) return;

      const oldId = this._nodeId;
      this._nodeId = nodeId;

      if (__lunaElementCache.get(oldId) === this) {
        __lunaElementCache.delete(oldId);
      }
      __lunaElementCache.set(nodeId, this);

      if (this._positionProxy) this._positionProxy._nodeId = nodeId;
      if (this._rotationProxy) this._rotationProxy._nodeId = nodeId;
      if (this._scaleProxy) this._scaleProxy._nodeId = nodeId;
      if (this._globalPositionProxy) this._globalPositionProxy._nodeId = nodeId;

      const callbacks = this._resolveCallbacks.splice(0, this._resolveCallbacks.length);
      for (const cb of callbacks) {
        try { cb(nodeId); } catch (e) { console.error(e); }
      }
    }

    // --- Identity ---

    get nodeId() { return this._nodeId; }

    get tagName() {
      if (!this._isResolved()) return this._tagHint;
      return core.ops.op_hsml_get_tag(this._nodeId);
    }

    // --- Attributes ---

    getAttribute(key) {
      if (!this._isResolved()) return '';
      return core.ops.op_hsml_get_attr(this._nodeId, String(key));
    }

    setAttribute(key, value) {
       const k = String(key);
       const v = String(value);
       if (this._isResolved()) {
         core.ops.op_hsml_set_attr(this._nodeId, k, v);
       } else {
         this._onResolved((nodeId) => core.ops.op_hsml_set_attr(nodeId, k, v));
       }
    }

    get id() { return this.getAttribute('id'); }
    set id(v) { this.setAttribute('id', v); }

    get className() { return this.getAttribute('class'); }
    set className(v) { this.setAttribute('class', v); }

    get classList() {
      if (!this._classList) {
        this._classList = new DOMTokenList(
          () => this.className,
          (v) => { this.className = v; }
        );
      }
      return this._classList;
    }

    // --- Transform (position, rotation, scale) ---

    get position() {
      if (!this._isResolved()) {
        return { x: 0, y: 0, z: 0 };
      }
      if (!this._positionProxy) {
        this._positionProxy = new ProxyVec3(
          core.ops.op_hsml_get_position,
          core.ops.op_hsml_set_position,
          this._nodeId
        );
      }
      return this._positionProxy;
    }

    set position(v) {
      if (typeof v === 'object' && v.x != null && v.y != null && v.z != null) {
        if (this._isResolved()) {
          core.ops.op_hsml_set_position(this._nodeId, v.x, v.y, v.z);
        } else {
          this._onResolved((nodeId) => {
            core.ops.op_hsml_set_position(nodeId, v.x, v.y, v.z);
          });
        }
        if (this._positionProxy) {
          this._positionProxy._cache = { x: v.x, y: v.y, z: v.z };
        }
      }
    }

    get rotation() {
      if (!this._isResolved()) {
        return { x: 0, y: 0, z: 0 };
      }
      if (!this._rotationProxy) {
        this._rotationProxy = new ProxyVec3(
          core.ops.op_hsml_get_rotation,
          core.ops.op_hsml_set_rotation,
          this._nodeId
        );
      }
      return this._rotationProxy;
    }

    set rotation(v) {
      if (typeof v === 'object' && v.x != null && v.y != null && v.z != null) {
        if (this._isResolved()) {
          core.ops.op_hsml_set_rotation(this._nodeId, v.x, v.y, v.z);
        } else {
          this._onResolved((nodeId) => {
            core.ops.op_hsml_set_rotation(nodeId, v.x, v.y, v.z);
          });
        }
        if (this._rotationProxy) {
          this._rotationProxy._cache = { x: v.x, y: v.y, z: v.z };
        }
      }
    }

    get scale() {
      if (!this._isResolved()) {
        return { x: 1, y: 1, z: 1 };
      }
      if (!this._scaleProxy) {
        this._scaleProxy = new ProxyVec3(
          core.ops.op_hsml_get_scale,
          core.ops.op_hsml_set_scale,
          this._nodeId
        );
      }
      return this._scaleProxy;
    }

    set scale(v) {
      if (typeof v === 'number') {
        if (this._isResolved()) {
          core.ops.op_hsml_set_scale(this._nodeId, v, v, v);
        } else {
          this._onResolved((nodeId) => {
            core.ops.op_hsml_set_scale(nodeId, v, v, v);
          });
        }
        if (this._scaleProxy) {
          this._scaleProxy._cache = { x: v, y: v, z: v };
        }
      } else if (typeof v === 'object' && v.x != null && v.y != null && v.z != null) {
        this._onResolved((nodeId) => {
          core.ops.op_hsml_set_scale(nodeId, v.x, v.y, v.z);
        });
        if (this._scaleProxy) {
          this._scaleProxy._cache = { x: v.x, y: v.y, z: v.z };
        }
      }
    }

    get globalPosition() {
      if (!this._isResolved()) {
        return { x: 0, y: 0, z: 0 };
      }
      if (!this._globalPositionProxy) {
        this._globalPositionProxy = new ProxyVec3(
          core.ops.op_hsml_get_global_position,
          core.ops.op_hsml_set_global_position,
          this._nodeId
        );
      }
      return this._globalPositionProxy;
    }

    set globalPosition(v) {
      if (typeof v === 'object' && v.x != null && v.y != null && v.z != null) {
        this._onResolved((nodeId) => {
          core.ops.op_hsml_set_global_position(nodeId, v.x, v.y, v.z);
        });
        if (this._globalPositionProxy) {
          this._globalPositionProxy._cache = { x: v.x, y: v.y, z: v.z };
        }
      }
    }

    // --- Hierarchy ---

    get parent() {
      if (!this._isResolved()) return null;
      const parentId = core.ops.op_hsml_get_parent(this._nodeId);
      return parentId >= 0 ? _wrapElement(parentId) : null;
    }

    get children() {
      if (!this._isResolved()) return [];
      const ids = core.ops.op_hsml_get_children(this._nodeId);
      return ids.map(id => _wrapElement(id));
    }

    appendChild(child) {
      if (!(child instanceof HSMLElement)) {
        throw new Error('appendChild: argument must be HSMLElement');
      }
      this._onResolved((parentId) => {
        child._onResolved((childId) => {
          core.ops.op_hsml_append_child(parentId, childId);
        });
      });
    }

    remove() {
      this._onResolved((nodeId) => {
        core.ops.op_hsml_remove(nodeId);
      });
    }

    // --- Events ---

    addEventListener(type, listener) {
      if (typeof listener !== 'function') return;
      const eventType = String(type || '').toLowerCase();
      if (!eventType) return;

      let listeners = this._eventListeners.get(eventType);
      if (!listeners) {
        listeners = [];
        this._eventListeners.set(eventType, listeners);
      }

      if (!listeners.includes(listener)) {
        listeners.push(listener);
      }
    }

    removeEventListener(type, listener) {
      const eventType = String(type || '').toLowerCase();
      if (!eventType) return;
      const listeners = this._eventListeners.get(eventType);
      if (!listeners) return;

      const idx = listeners.indexOf(listener);
      if (idx !== -1) {
        listeners.splice(idx, 1);
      }
      if (listeners.length === 0) {
        this._eventListeners.delete(eventType);
      }
    }

    dispatchEvent(event) {
      const evt = (event && typeof event === 'object') ? event : { type: String(event || '') };
      const eventType = String(evt.type || '').toLowerCase();
      if (!eventType) return true;

      if (evt.target == null) evt.target = this;
      evt.currentTarget = this;
      if (typeof evt.preventDefault !== 'function') {
        evt.defaultPrevented = false;
        evt.preventDefault = function() { this.defaultPrevented = true; };
      }

      const listeners = this._eventListeners.get(eventType);
      if (listeners && listeners.length > 0) {
        for (const listener of [...listeners]) {
          try {
            listener.call(this, evt);
          } catch (e) {
            console.error(e);
          }
        }
      }

      const propHandler = this._eventHandlers[`on${eventType}`];
      if (typeof propHandler === 'function') {
        try {
          propHandler.call(this, evt);
        } catch (e) {
          console.error(e);
        }
      }

      return !evt.defaultPrevented;
    }

    click() {
      this.dispatchEvent({ type: 'click' });
    }

    get onclick() {
      return this._eventHandlers.onclick || null;
    }

    set onclick(handler) {
      this._eventHandlers.onclick = (typeof handler === 'function') ? handler : null;
    }

    // --- Query methods ---

    getElementById(id) {
      return this._querySelector((elem) => elem.id === id);
    }

    getElementsByClass(className) {
      return this._querySelectorAll((elem) => elem.classList.contains(className));
    }

    getElementsByName(name) {
      return this._querySelectorAll((elem) => elem.getAttribute('name') === name);
    }

    getElementByClass(className) {
      return this._querySelector((elem) => elem.classList.contains(className));
    }

    getElementByName(name) {
      return this._querySelector((elem) => elem.getAttribute('name') === name);
    }

    // Helper: depth-first search
    _querySelector(predicate) {
      for (const child of this.children) {
        if (predicate(child)) return child;
        const found = child._querySelector(predicate);
        if (found) return found;
      }
      return null;
    }

    _querySelectorAll(predicate) {
      const results = [];
      for (const child of this.children) {
        if (predicate(child)) results.push(child);
        results.push(...child._querySelectorAll(predicate));
      }
      return results;
    }
  }

  // ---------------------------------------------------------------------------
  // HSMLRootElement (document root)
  // ---------------------------------------------------------------------------

  class HSMLRootElement extends HSMLElement {
    constructor(nodeId) {
      super(nodeId);
    }

    createElement(tagName) {
      const requestId = core.ops.op_hsml_create_element(String(tagName));
      const pendingNodeId = -Math.abs(requestId);
      const element = _wrapElement(pendingNodeId, String(tagName));

      resolveCreatedElementAsync(requestId, (nodeId) => {
        element._resolveNodeId(nodeId);
      });

      return element;
    }
    
    // Batch de transforms locales del contexto actual.
    // Layout plano:
    // [nodeId, px, py, pz, rx, ry, rz, ...]
    setTransformBatch(updates) {
      if (!Array.isArray(updates) || updates.length === 0) return;
      if ((updates.length % 7) !== 0) {
        throw new Error('setTransformBatch: updates.length must be multiple of 7');
      }
      core.ops.op_hsml_set_transform_batch(updates);
    }
  }

  // ---------------------------------------------------------------------------
  // HSMLModelElement (3D model)
  // ---------------------------------------------------------------------------

  class HSMLModelElement extends HSMLElement {
    constructor(nodeId) {
      super(nodeId);
    }

    get src() { return this.getAttribute('src'); }
    set src(v) { this.setAttribute('src', v); }

    get rigidbody() { return this.getAttribute('rigidbody') === 'true'; }
    set rigidbody(v) { this.setAttribute('rigidbody', String(!!v)); }

    get collider() { return this.getAttribute('collider') === 'true'; }
    set collider(v) { this.setAttribute('collider', String(!!v)); }
  }

  // ---------------------------------------------------------------------------
  // HSMLButtonElement (button UI)
  // ---------------------------------------------------------------------------

  class HSMLButtonElement extends HSMLElement {
    constructor(nodeId) {
      super(nodeId);
    }

    get text() { return this.getAttribute('text'); }
    set text(v) { this.setAttribute('text', v); }
  }

  // ---------------------------------------------------------------------------
  // HSMLVideoElement (<video src="..."/>)
  // ---------------------------------------------------------------------------

  class HSMLVideoElement extends HSMLElement {
    constructor(nodeId) {
      super(nodeId);
    }

    get src() { return this.getAttribute('src'); }
    set src(v) { this.setAttribute('src', v); }

    get loop() { return this.getAttribute('loop') === 'true'; }
    set loop(v) { this.setAttribute('loop', String(!!v)); }

    get autoplay() { return this.getAttribute('autoplay') === 'true'; }
    set autoplay(v) { this.setAttribute('autoplay', String(!!v)); }

    get muted() { return this.getAttribute('muted') === 'true'; }
    set muted(v) { this.setAttribute('muted', String(!!v)); }

    play() {
      this._onResolved((nodeId) => {
        core.ops.op_hsml_video_play(nodeId);
      });
    }

    pause() {
      this._onResolved((nodeId) => {
        core.ops.op_hsml_video_pause(nodeId);
      });
    }

    stop() {
      this._onResolved((nodeId) => {
        core.ops.op_hsml_video_stop(nodeId);
      });
    }
  }

  // ---------------------------------------------------------------------------
  // Element factory
  // ---------------------------------------------------------------------------

  function _wrapElement(nodeId, tagHint = '') {
    const cached = __lunaElementCache.get(nodeId);
    if (cached) {
      if (tagHint) cached._setTagHint(tagHint);
      return cached;
    }

    const tag = nodeId >= 0 ? core.ops.op_hsml_get_tag(nodeId) : String(tagHint || '');
    let el;

    switch (tag.toUpperCase()) {
      case 'MODEL': {
        el = new HSMLModelElement(nodeId);
        el._setTagHint('MODEL');
        break;
      }
      case 'BUTTON': {
        el = new HSMLButtonElement(nodeId);
        el._setTagHint('BUTTON');
        break;
      }
      case 'VIDEO': {
        el = new HSMLVideoElement(nodeId);
        el._setTagHint('VIDEO');
        break;
      }
      default: {
        el = new HSMLElement(nodeId);
        el._setTagHint(tagHint || tag);
        break;
      }
    }

    __lunaElementCache.set(nodeId, el);
    return el;
  }

  // ---------------------------------------------------------------------------
  // Location API
  // ---------------------------------------------------------------------------

  class Location {
    get href() {
      return global._lunaCurrentUrl || '';
    }

    set href(url) {
      core.ops.op_navigate(String(url));
      global._lunaCurrentUrl = String(url);
    }
  }

  // ---------------------------------------------------------------------------
  // Fetch API
  // ---------------------------------------------------------------------------

  global.fetch = function(url, options) {
    return new Promise((resolve, reject) => {
      const requestId = core.ops.op_fetch_request(String(url));

      function poll() {
        const result = core.ops.op_fetch_poll(requestId);
        if (result.status === 'pending') {
          setTimeout(poll, 10);
        } else if (result.status === 'ok') {
          resolve({
            ok: true,
            status: 200,
            text: () => Promise.resolve(result.text),
            json: () => Promise.resolve(JSON.parse(result.text)),
          });
        } else {
          reject(new Error(result.error));
        }
      }
      poll();
    });
  };

  // ---------------------------------------------------------------------------
  // hiperspace API (Unity compatibility layer)
  // ---------------------------------------------------------------------------

  global.hiperspace = {
    dimention: null,  // Root element set by setHiperSpace
    location: new Location(),
    open: (url) => {
      console.warn('hiperspace.open() not yet implemented');
    },

    setHiperSpace: function(dimension, runtime) {
      // Called by host (Bevy) to initialize the runtime
      // dimension = { location: string, open: fn }
      // runtime = { _CreateHSMLElement: fn, _SetDimention: fn }

      if (dimension && dimension.open) {
        this.open = dimension.open;
      }

      if (dimension && dimension.location) {
        global._lunaCurrentUrl = dimension.location;
      }

      // _CreateHSMLElement is not needed (we create via ops)
      // _SetDimention sets the root element
      if (runtime && runtime._SetDimention) {
        // Runtime provides root node ID
        // For now, we assume node_id=0 is root (set by host)
        this.dimention = new HSMLRootElement(0);
      }

      console.log('[Luna Runtime] Initialized v0.1.0');
    },

    // Deprecated Unity compatibility
    _CreateHSMLElement: function(native) {
      console.warn('_CreateHSMLElement is deprecated, use createElement instead');
    },

    _SetDimention: function(native) {
      console.warn('_SetDimention is deprecated');
    },
  };

  if (!global.location) {
    global.location = global.hiperspace.location;
  }

  // ---------------------------------------------------------------------------
  // WebSocket
  // ---------------------------------------------------------------------------

  class WebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    constructor(url) {
      this.url = String(url);
      this.readyState = WebSocket.CONNECTING;
      this.onopen = null;
      this.onmessage = null;
      this.onerror = null;
      this.onclose = null;
      this._listeners = {};
      this._connId = core.ops.op_ws_connect(this.url);
      this._poll();
    }

    send(data) {
      if (this.readyState !== WebSocket.OPEN) {
        throw new Error('WebSocket is not open');
      }
      core.ops.op_ws_send(this._connId, String(data));
    }

    close() {
      if (this.readyState === WebSocket.CLOSED) return;
      this.readyState = WebSocket.CLOSING;
      core.ops.op_ws_close(this._connId);
      this.readyState = WebSocket.CLOSED;
      this._emit('close', { type: 'close' });
    }

    addEventListener(type, fn) {
      (this._listeners[type] = this._listeners[type] || []).push(fn);
    }

    removeEventListener(type, fn) {
      if (!this._listeners[type]) return;
      this._listeners[type] = this._listeners[type].filter(f => f !== fn);
    }

    _emit(type, event) {
      const handler = this['on' + type];
      if (handler) { try { handler(event); } catch(e) { console.error(e); } }
      const listeners = this._listeners[type];
      if (listeners) {
        for (const fn of listeners) { try { fn(event); } catch(e) { console.error(e); } }
      }
    }

    _poll() {
      if (this.readyState === WebSocket.CLOSED) return;

      const status = core.ops.op_ws_get_status(this._connId);

      if (status === 'open' && this.readyState === WebSocket.CONNECTING) {
        this.readyState = WebSocket.OPEN;
        this._emit('open', { type: 'open' });
      } else if (status.startsWith('error')) {
        this.readyState = WebSocket.CLOSED;
        this._emit('error', { type: 'error', message: status });
        return;
      } else if (status === 'closed' && this.readyState !== WebSocket.CLOSED) {
        this.readyState = WebSocket.CLOSED;
        this._emit('close', { type: 'close' });
        return;
      }

      // Drain inbox
      if (this.readyState === WebSocket.OPEN) {
        let result;
        while ((result = core.ops.op_ws_recv(this._connId)).status === 'ok') {
          this._emit('message', { type: 'message', data: result.data });
        }
      }

      setTimeout(() => this._poll(), 16);
    }
  }

  // ---------------------------------------------------------------------------
  // Global exports
  // ---------------------------------------------------------------------------

  global.WebSocket = WebSocket;
  global.HSMLElement = HSMLElement;
  global.HSMLRootElement = HSMLRootElement;
  global.HSMLModelElement = HSMLModelElement;
  global.HSMLButtonElement = HSMLButtonElement;
  global.HSMLVideoElement = HSMLVideoElement;
  global.Location = Location;

  // Auto-initialize with default root (node_id=0)
  // Bevy will call setHiperSpace if needed
  if (!global.hiperspace.dimention) {
    global.hiperspace.dimention = new HSMLRootElement(0);
  }

  console.log('[Luna Runtime] Loaded successfully');

})(globalThis);
