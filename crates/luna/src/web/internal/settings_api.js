// luna://internal/settings_api.js
// API de la pagina de ajustes — `dimention.settings.*`.
//
// Reemplaza al buzon de nodos invisibles. Antes el host le contaba el estado al
// documento **inyectando JavaScript** que le escribia atributos a nodos de ids
// fijos: el id quedaba acordado a mano entre `agent.rs` y el `.hsml`, el dato
// pasaba por tres conversiones, y si alguien borraba el nodo la pagina no se
// enteraba nunca, sin un solo error.
//
// Ahora es el mismo mecanismo que ya usan `fetch`, las capturas y el hover: el
// host deja el dato en una cola del OpState y el isolate lo saca con un op.
//
// Contrato:
//
//     dimention.settings.state              // lo ultimo publicado, o null
//     dimention.settings.on(fn)             // fn(estado) en cada cambio; devuelve
//                                           // una funcion para desuscribirse
//     dimention.settings.set(parche)        // configuracion raiz, campo por campo
//     dimention.settings.setMcp(bool)       // prender/apagar el servidor ahora
//     dimention.settings.setMcpAutoStart(b) // y que arranque solo, guardado
//
// **Quien puede** no lo decide este archivo. Las tres escrituras encolan una
// accion que el host acepta unicamente si el documento es `luna://settings`
// (`is_settings_document`, agent.rs); desde cualquier otro lado se descartan
// sin avisar. Y la lectura llega vacia, porque el host solo publica ahi.
//
// Se carga con <script src>, no por un bundle: no hace falta una capacidad
// nueva para algo cuya autoridad es la URL del documento. Pedir un permiso
// ademas seria sugerir que se puede conceder, y no se puede.
(function (global) {
  const core = Deno.core;
  const ops = core.ops;
  const root = global.hiperspace && global.hiperspace.dimention;
  if (!root) return;
  if (root.settings) return; // Recargar el script no arranca un segundo sondeo.

  if (!ops.op_settings_read) {
    console.error('[settings_api] falta op_settings_read: Luna sin el buzon de ajustes');
    return;
  }

  let estado = null;
  let revision = 0;
  const oyentes = new Set();

  function avisar() {
    for (const fn of [...oyentes]) {
      try { fn(estado); } catch (e) { console.error(e); }
    }
  }

  // El sondeo es de un entero: el op compara la revision que ya vimos y
  // devuelve el JSON **solo** cuando cambio. Un cuadro en el que no paso nada
  // cuesta una llamada y ninguna copia.
  function mirar() {
    requestAnimationFrame(mirar);
    let leido;
    try { leido = ops.op_settings_read(revision); } catch (e) { return; }
    if (!leido || !leido.json) return;
    revision = leido.revision;
    try { estado = Object.freeze(JSON.parse(leido.json)); }
    catch (e) { console.error('[settings_api] publicacion ilegible', e); return; }
    avisar();
  }
  requestAnimationFrame(mirar);

  const api = Object.freeze({
    get state() { return estado; },
    get revision() { return revision; },

    /** Avisar en cada cambio. Si ya hay estado, se entrega en el acto: al que
     *  se suscribe le importa el valor, no haberse perdido el evento. */
    on(fn) {
      if (typeof fn !== 'function') return function () {};
      oyentes.add(fn);
      if (estado) { try { fn(estado); } catch (e) { console.error(e); } }
      return function () { oyentes.delete(fn); };
    },

    /** Parche de la configuracion raiz. Mandar **solo lo que cambio**: el host
     *  aplica campo por campo, asi que una pagina vieja no pisa una preferencia
     *  que todavia no conoce. Campos: autoLoadHome, homeUrl, renderMode
     *  ('desktop' | 'vr') y permission {origin, key, decision}, con decision
     *  'allow' | 'deny' | 'ask'. */
    set(patch) {
      ops.op_luna_root_settings(JSON.stringify(patch || {}));
    },

    /** El servidor, ahora. No se guarda: vale para esta sesion. */
    setMcp(enabled) {
      ops.op_luna_mcp_enabled(Boolean(enabled));
    },

    /** Que arranque con Luna. Esto si se escribe en disco, al toque. */
    setMcpAutoStart(enabled) {
      ops.op_luna_mcp_auto_start(Boolean(enabled));
    },
  });

  Object.defineProperty(root, 'settings', { value: api, enumerable: true });
})(globalThis);
