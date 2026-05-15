// luna://internal/embedded_api.js
// API para apps embedded — `dimention.embedded.*`.
// Auto-inyectado vía bundle `ux_embed` (elevated). Solo apps montadas con
// kind='app-embedded' desde el shell trusted reciben este script.
//
// Contrato:
//   - App pide slot vía requestSlot(opts).
//   - Shell responde con 'slot' event (puede llegar después, async).
//   - Shell puede mandar 'suspend' / 'resume' / 'close' / 'focus' / 'blur' / 'slot'.
//   - App cierra con closeSelf() (cleanup en 'close' callback).
//
// Bajo el capó: ops Rust op_shell_send_message + op_shell_poll_messages.
// El "shell" es el space ux_vr (system-shell="true"). target_tab_id=0 en la
// op significa "rutea al shell".

(function (global) {
  const core = Deno.core;
  const ops = core.ops;
  if (!ops.op_shell_send_message || !ops.op_shell_poll_messages) {
    console.error('[embedded_api] missing ops — runtime sin shell message bus');
    return;
  }

  // ── Context detection ────────────────────────────────────────────────────
  // En este isolate puede no haber forma de auto-detectar el tab_id propio.
  // El shell, al mountear la app, le pasa el tab_id como atributo del space:
  // data-luna-tab-id. La app puede leerlo de su propio root.
  const root = global.hiperspace && global.hiperspace.dimention;
  let myTabId = 0;
  try {
    const t = root && root.getAttribute && root.getAttribute('data-luna-tab-id');
    if (t) myTabId = Number(t) || 0;
  } catch (_) {}

  // ── Event bus interno (solo para listeners locales de la app) ────────────
  const listeners = new Map(); // eventType → Set<fn>
  function on(type, cb) {
    if (typeof cb !== 'function') return;
    const t = String(type || '').toLowerCase();
    if (!listeners.has(t)) listeners.set(t, new Set());
    listeners.get(t).add(cb);
  }
  function off(type, cb) {
    const t = String(type || '').toLowerCase();
    const s = listeners.get(t);
    if (s) s.delete(cb);
  }
  function emit(type, evt) {
    const s = listeners.get(String(type || '').toLowerCase());
    if (!s) return;
    for (const cb of [...s]) {
      try { cb(evt); } catch (e) { console.error(e); }
    }
  }

  // ── Poll loop del inbox ──────────────────────────────────────────────────
  // Lee los mensajes del shell cada tick y emite eventos según el `type` del
  // payload (JSON-encoded por convención).
  function pollOnce() {
    const msgs = ops.op_shell_poll_messages();
    if (!Array.isArray(msgs) || msgs.length === 0) return;
    for (const m of msgs) {
      let parsed = null;
      try { parsed = JSON.parse(m.payload); } catch (_) {}
      if (!parsed || typeof parsed !== 'object') {
        console.warn('[embedded_api] received non-JSON shell message:', m.payload);
        continue;
      }
      const evtType = String(parsed.type || '').toLowerCase();
      const evt = Object.assign({}, parsed, { fromTabId: Number(m.fromTabId || 0) });
      emit(evtType, evt);
    }
  }
  function pollTick() {
    pollOnce();
    setTimeout(pollTick, 16);
  }
  pollTick();

  // ── Public API ───────────────────────────────────────────────────────────
  function sendToShell(type, extra) {
    const payload = JSON.stringify(Object.assign({ type: String(type) }, extra || {}));
    ops.op_shell_send_message(BigInt(0), payload);
  }

  const api = {
    context: {
      embedded: true,
      tabId: myTabId,
      kind: 'app-embedded',
    },

    /// Pide al shell un slot para dibujar.
    /// opts: { minSize?: {x,y,z}, preferredSize?: {x,y,z}, title?: string }
    /// Respuesta llega como event 'slot' con { position, rotation, size }.
    requestSlot(opts) {
      sendToShell('requestSlot', { opts: opts || {} });
    },

    /// Pide al shell que cierre este tab. Shell mandará 'close' event primero
    /// (la app debería cleanup), después hace unmount.
    closeSelf() {
      sendToShell('closeSelf', {});
    },

    /// Notifica al shell que la app está lista (post-init).
    notifyReady() {
      sendToShell('ready', {});
    },

    on,
    off,
  };

  function attach(target) {
    if (target && !target.embedded) {
      target.embedded = api;
    }
  }
  attach(global.hiperspace && global.hiperspace.dimention);
  attach(global.dimention);

  console.log('[embedded_api] dimention.embedded ready (tabId=' + myTabId + ')');
})(globalThis);
