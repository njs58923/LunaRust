// luna://internal/tabs_api.js
// chrome.tabs-like API expuesta como `dimention.tabs`. Auto-inyectado vía
// bundle `manage_tabs` (sólo para spaces UX shell managed-by=dimension.luna).
// Las ops pushean a colas internas; el host (luna js_tick_system) drena,
// chequea cap, y reenvía a SpaceMountQueue / SpaceUnmountQueue.

(function (global) {
  const core = Deno.core;
  const ops = core.ops;
  if (!ops.op_tab_open) {
    console.error('[tabs_api] op_tab_open missing — runtime lacks tabs ops');
    return;
  }

  // Kinds soportados. La policy real (cerrar otras spatial, etc) se aplica en
  // root_api.js — acá sólo propagamos el string al host.
  //   "spatial" — al abrir, el shell cierra otras tabs spatial.
  //   "app"     — aditiva, persiste entre cambios de spatial.
  // Default si no se pasa: "spatial" (equivale a window.open con _blank).
  const VALID_KINDS = new Set(['spatial', 'app']);

  const tabsApi = {
    // Abre nueva tab cargando `url`. `opts.kind` define semantics.
    // Devuelve void; el tab_id lo asigna el host. Para identificar por url,
    // usar list() (TODO).
    open(url, opts) {
      let kind = 'spatial';
      if (opts && typeof opts.kind === 'string') {
        if (VALID_KINDS.has(opts.kind)) {
          kind = opts.kind;
        } else {
          console.warn('[tabs] unknown kind', opts.kind, '— defaulting to spatial');
        }
      }
      ops.op_tab_open(String(url), kind);
    },
    close(tabId) {
      ops.op_tab_close(BigInt(tabId));
    },
    setVisible(tabId, visible) {
      ops.op_tab_set_visible(BigInt(tabId), !!visible);
    },
    // TODO(tabs-list): expone una snapshot consultable. Hoy no implementado.
    list() {
      console.warn('[tabs] list() not yet implemented');
      return [];
    },
  };

  // Expone dos handles:
  //   global.dimention.tabs       (forma compacta, post setHiperSpace)
  //   hiperspace.dimention.tabs   (idem, mismo objeto)
  function attach(target) {
    if (target && !target.tabs) {
      target.tabs = tabsApi;
    }
  }

  attach(global.hiperspace && global.hiperspace.dimention);
  attach(global.dimention);

  console.log('[tabs_api] dimention.tabs ready');
})(globalThis);
