(function(g) {
  'use strict';
  const ops = Deno.core.ops, listeners = new Map(), held = new Map();
  let target = null, localEditable = false, session = { revision: 0, focused: false, editable: false, vr: false };
  function event(type, data, cancelable = true) {
    return Object.assign({ bubbles: true, cancelable, isTrusted: false,
      defaultPrevented: false, preventDefault() { if (this.cancelable) this.defaultPrevented = true; },
      stopPropagation() { this._stopped = true; },
      stopImmediatePropagation() { this._stopped = this._immediateStopped = true; }
    }, data, {type});
  }
  function notify(type, data) {
    for (const fn of [...(listeners.get(type) || [])]) { try { fn(data); } catch(e) { console.error(e); } }
  }
  function dispatch(type, data, cancelable = true) {
    const root = g.hiperspace && g.hiperspace.dimention;
    const e = event(type, data, cancelable); e.target = target || root;
    const visited = new Set();
    for (let n = e.target; n && !visited.has(n); n = n === root ? null : (n.parent || root)) {
      visited.add(n); e.currentTarget = n;
      if (typeof n.dispatchEvent === 'function') n.dispatchEvent(e);
      if (e._stopped || !e.bubbles) break;
    }
    return e;
  }
  function release() {
    for (const data of held.values()) dispatch('keyup', { ...data, repeat: false }, false);
    held.clear();
    if (target) dispatch('blur', {}, false);
    target = null;
  }
  const api = Object.freeze({
    get activeElement() { return target; },
    get state() { return Object.freeze({ ...session }); },
    addEventListener(type, fn) { if (!listeners.has(type)) listeners.set(type, new Set()); listeners.get(type).add(fn); },
    removeEventListener(type, fn) { listeners.get(type)?.delete(fn); },
    focus(element, options = {}) {
      if (!element || typeof element.dispatchEvent !== 'function') throw new TypeError('Focus requires an event target');
      if (target === element && localEditable === !!options.editable) return;
      if (target !== element) { release(); target = element; dispatch('focus', {}, false); }
      localEditable = !!options.editable;
      ops.op_keyboard_command({ action: 'focus', editable: !!options.editable });
    },
    blur() { release(); ops.op_keyboard_command({ action: 'blur' }); },
    copy(text) { ops.op_keyboard_command({action:'clipboard',revision:session.revision,text:String(text).slice(0,4096)}); },
    // Host accepts injection only from the currently mounted trusted controller.
    send(packet, revision = session.revision) { ops.op_keyboard_command({ action: 'send', revision, packet }); },
  });
  Object.defineProperty(g, 'keyboard', { value: api });
  if (g.HSMLElement) {
    HSMLElement.prototype.focus = function() { api.focus(this); };
    HSMLElement.prototype.blur = function() { if (target === this) api.blur(); };
  }
  g.__luna_keyboard_pump = function() {
    for (const message of ops.op_keyboard_read()) {
      if (message.type === 'state') {
        session = message;
        if (!message.focused) release();
        notify('statechange', api.state);
      } else if (message.type === 'device') {
        notify('deviceinput', message);
      } else if (message.type === 'input') {
        const p = message.packet;
        if (p.type === 'keydown') {
          held.set(p.code || p.key, p);
          const e = dispatch('keydown', p);
          if (!e.defaultPrevented && target && typeof target.defaultKeyDown === 'function') target.defaultKeyDown(e);
          if (!e.defaultPrevented && p.text && (!p.ctrlKey || p.altKey) && !p.metaKey && !p.isComposing) {
            const before = dispatch('beforeinput', { data: p.text, inputType: 'insertText', isComposing: false });
            if (!before.defaultPrevented && target && typeof target.insertText === 'function') target.insertText(p.text, 'insertText');
          }
        } else if (p.type === 'keyup') {
          held.delete(p.code || p.key); dispatch('keyup', p);
        } else if (p.type === 'text') {
          const before = dispatch('beforeinput', { data: p.text, inputType: 'insertText', isComposing: false });
          if (!before.defaultPrevented && target && typeof target.insertText === 'function') target.insertText(p.text, 'insertText');
        } else if (p.type.startsWith('composition')) dispatch(p.type, { data: p.text || '', isComposing: p.type !== 'compositionend' }, false);
      }
    }
  };
})(globalThis);
