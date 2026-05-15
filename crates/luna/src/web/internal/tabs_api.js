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

  const tabsApi = {
    // Abre nueva tab cargando `url`. Devuelve void; el tab_id real lo asigna
    // el host. Para identificar tabs por url, usar list() (TODO).
    open(url, _opts) {
      ops.op_tab_open(String(url));
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
