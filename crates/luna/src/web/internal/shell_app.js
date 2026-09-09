// ─────────────────────────────────────────────────────────────────────────────
// shell_app.js — el shell de Luna: qué se ve, cuándo, y quién manda.
//
// Es la máquina de estados que vivía adentro de `ux_vr.hsml`. Dibuja con
// `shell_ui.js` y **no toca el host**: todo lo que sale de este módulo hacia el
// motor —abrir una pestaña, cerrarla, mandarle un mensaje a una app, leer la
// pose del visitante— entra por `cfg`. El controller que lo monta decide cómo
// se cumplen esas operaciones, y por eso el mismo shell sirve en VR y en
// escritorio aunque los dos lleguen a él por caminos distintos.
//
// Va como script y no como `<include>`: un include es otro documento, con su
// propio isolate y sus propios permisos, y desde ahí no habría forma de hablarle
// a `dimention.tabs` ni de mandarle el slot a una app.
//
// El modelo de estados es el de `docs/embedded_apps.md`:
//
//   panel = bookmarks_visible && (no hay foco || el foco está minimizado)
//   fijada_i = !minimizada && (siempre_visible || bookmarks_visible)
//
// **Regla de oro: cualquier cambio en `state` va seguido de `applyVisibility()`.**
// Nada dibuja por su cuenta; todo se deriva del estado en un solo lugar.
// ─────────────────────────────────────────────────────────────────────────────
(function () {
  const UI = globalThis.ShellUI;
  if (!UI) {
    console.error('[shell_app] falta shell_ui.js — cargalo antes');
    return;
  }

  function mount(cfg) {
    const root = cfg.root;
    const panel = cfg.panel;
    const bottomBar = cfg.bottomBar;
    const focusZone = cfg.focusZone;
    const anchoredZone = cfg.anchoredZone;
    const BOOKMARKS = cfg.bookmarks || [];
    const DIST = cfg.distance || 1.5;

    // ── Puentes al host ──────────────────────────────────────────────────
    // Envueltos en funciones tolerantes en vez de usarse directo: el shell se
    // monta antes de que el host termine de armarse, y una llamada temprana no
    // puede tirar abajo la construcción de la interfaz.
    const tabs = cfg.tabs || {};
    const send = typeof cfg.send === 'function' ? cfg.send : function () {};
    const poll = typeof cfg.poll === 'function' ? cfg.poll : function () { return []; };
    const viewerPose = typeof cfg.viewerPose === 'function' ? cfg.viewerPose : function () { return null; };
    const navigate = typeof cfg.navigate === 'function' ? cfg.navigate : function () {};

    function tabOpen(url, opts) {
      if (typeof tabs.open !== 'function') { navigate(url); return; }
      tabs.open(url, opts);
    }
    function tabClose(id) { if (typeof tabs.close === 'function') tabs.close(id); }
    function tabVisible(id, v) { if (typeof tabs.setVisible === 'function') tabs.setVisible(id, v); }

    function sendMessageToApp(tabId, type, extra) {
      if (!(tabId > 0)) return;
      send(tabId, JSON.stringify(Object.assign({ type: type }, extra || {})));
    }

    // ── Medidas de una ventana ───────────────────────────────────────────
    // Van por la escala de la interfaz: si la grilla mide el doble y la ventana
    // no, la app queda de juguete al lado del menú que la abrió. El tamaño se
    // le manda a la app en el slot, así que esto es su lienzo.
    const S = UI.M.escala || 1;
    const SLOT_W = 1.0 * S;
    const SLOT_H = 0.7 * S;
    const SLOT_D = 0.1 * S;
    // Offsets laterales para apartar las fijadas del frente del shell. Se
    // apilan a la derecha; el par de números es pragmático, no un layout.
    const ANCHORED_BASE_X = 1.35 * S;
    const ANCHORED_STACK_DX = 0.12 * S;

    // ── Estado ───────────────────────────────────────────────────────────
    const state = {
      bookmarksVisible: false,
      focusApp: null,   // { tabId, minimized, ready, title, ... }
      anchored: [],     // [{ tabId, position, rotation, minimized, alwaysOn, frame }]
    };

    function shouldShowBookmarks() {
      if (!state.bookmarksVisible) return false;
      // Con una app en primer plano el panel estorba: ocupa el mismo lugar.
      if (state.focusApp && !state.focusApp.minimized && state.focusApp.ready) return false;
      return true;
    }

    // La regla del doc era `bookmarks_visible && (foco || fijadas)`: la barra
    // existía sólo para volver desde una app. Ahora lleva **estado** —usuario,
    // mundo, avisos— y eso tiene que estar mientras el shell esté, haya
    // ventanas o no. Con la regla vieja, abrir el menú sin ninguna app mostraba
    // una grilla flotando sin nada abajo.
    function shouldShowBottomBar() {
      return state.bookmarksVisible;
    }

    // ── El tablero ───────────────────────────────────────────────────────
    const board = UI.dashboard(panel, {
      title: cfg.title || 'Aplicaciones',
      apps: BOOKMARKS,
      rail: cfg.rail || [],
      onOpen: openBookmark,
    });

    // La barra cuelga del borde de abajo del tablero, no de una altura fija:
    // así sigue al panel cuando cambia la escala o la cantidad de filas.
    const barY = board.centerY - board.height / 2 - UI.M.barH / 2 - 0.06 * S;
    const bar = UI.bar(bottomBar, { y: barY });

    // ── Animación de entrada y salida ────────────────────────────────────
    // Una cascada: los iconos entran de a uno, con un rebote corto. Es lo único
    // que separa «el menú apareció» de «el menú estaba y no lo veías».
    const ANIM_MS = 160;
    const STAGGER_MS = 12;
    function easeOutBack(t) {
      const c1 = 1.70158, c3 = c1 + 1;
      return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
    }
    function easeInBack(t) {
      const c1 = 1.70158, c3 = c1 + 1;
      return c3 * t * t * t - c1 * t * t;
    }

    let animStartTs = 0;
    let animActive = false;
    let animMode = null;

    function applyScale(tile, factor) {
      const f = Math.max(0.001, factor);
      tile.group.scale = { x: f, y: f, z: 1 };
      // Escalar mueve el hitbox, así que mientras dura la cascada no puede
      // recibir nada: el objetivo estaría en otro lado que el dibujo.
      tile.piece.hit.setAttribute('touchable', factor > 0.95 ? 'true' : 'false');
    }

    function startEnterAnimation() {
      for (const t of board.tiles) applyScale(t, 0);
      animStartTs = -1;
      animMode = 'enter';
      animActive = true;
      requestAnimationFrame(tickAnim);
    }

    function startExitAnimation() {
      animStartTs = -1;
      animMode = 'exit';
      animActive = true;
      requestAnimationFrame(tickAnim);
    }

    function tickAnim(ts) {
      if (!animActive) return;
      if (animStartTs < 0) animStartTs = ts;
      const elapsed = ts - animStartTs;
      const n = board.tiles.length;
      let running = false;
      for (let i = 0; i < n; i++) {
        const tile = board.tiles[i];
        // Al salir, la cascada va al revés: entra por el primero y se va por
        // el último, que es como se lee un barrido.
        const order = animMode === 'exit' ? (n - 1 - i) : i;
        const own = elapsed - order * STAGGER_MS;
        if (own < 0) { applyScale(tile, animMode === 'enter' ? 0 : 1); running = true; continue; }
        if (own >= ANIM_MS) { applyScale(tile, animMode === 'enter' ? 1 : 0); continue; }
        const t = own / ANIM_MS;
        applyScale(tile, animMode === 'enter' ? easeOutBack(t) : (1 - easeInBack(t)));
        running = true;
      }
      if (running) { requestAnimationFrame(tickAnim); return; }
      animActive = false;
      // Recién al terminar la salida se apaga el panel: apagarlo antes se
      // comería la animación que justo se está mirando.
      if (animMode === 'exit' && panel) panel.setAttribute('visible', 'false');
      animMode = null;
    }

    // ── La barra de abajo ────────────────────────────────────────────────
    // Se redibuja sólo si la composición cambió. `applyVisibility()` corre en
    // cada transición de estado, y rehacer los nodos en cada una hacía
    // parpadear la barra entera.
    let barSignature = null;

    function windowButtons() {
      const out = [];
      if (state.focusApp) {
        out.push({
          key: 'focus',
          dim: !!state.focusApp.minimized,
          onTap: onTapBarFocus,
        });
      }
      for (const a of state.anchored) {
        out.push({
          key: 'anc:' + a.tabId,
          dim: !!a.minimized,
          onTap: (function (entry) {
            return function () { onTapBarAnchored(state.anchored.indexOf(entry)); };
          })(a),
        });
      }
      return out;
    }

    function rebuildBottomBar() {
      const wins = windowButtons();
      const signature = wins.map(w => w.key + (w.dim ? ':m' : '')).join('|');
      if (signature === barSignature) return;
      barSignature = signature;

      const estado = (cfg.statusButtons || [
        { glyph: 'user', onTap: null },
        { glyph: 'home', onTap: null },
        { glyph: 'bell', onTap: null },
      ]).map(b => ({ glyph: b.glyph, color: UI.C.blue, onTap: b.onTap || function () {} }));

      // El botón de apps es la puerta al menú; los que le siguen, las ventanas
      // vivas. Por eso comparten pastilla y el estado del sistema no.
      const apps = [{ glyph: 'apps', color: UI.C.blue, selected: true, onTap: onTapBarMenu }];
      for (const w of wins) {
        apps.push({
          glyph: 'app',
          color: w.dim ? UI.C.pill : UI.C.blue,
          name: 'bar-' + w.key,
          onTap: w.onTap,
        });
      }
      bar.render([estado, apps]);
    }

    // ── Handlers de la barra ─────────────────────────────────────────────
    function onTapBarMenu() {
      if (state.focusApp && !state.focusApp.minimized) {
        state.focusApp.minimized = true;
        if (state.focusApp.tabId > 0) {
          tabVisible(state.focusApp.tabId, false);
          sendMessageToApp(state.focusApp.tabId, 'blur');
        }
        state.bookmarksVisible = true;
        applyVisibility();
        return;
      }
      // Sin foco ya estamos en el menú: no hay nada que hacer.
    }

    function onTapBarFocus() {
      if (!state.focusApp || !state.focusApp.ready) return;
      state.focusApp.minimized = false;
      if (state.focusApp.tabId > 0) {
        tabVisible(state.focusApp.tabId, true);
        sendMessageToApp(state.focusApp.tabId, 'focus');
        sendFocusSlotToApp();
      }
      applyVisibility();
    }

    function onTapBarAnchored(idx) {
      const a = state.anchored[idx];
      if (!a) return;
      a.minimized = !a.minimized;
      sendMessageToApp(a.tabId, a.minimized ? 'suspend' : 'resume');
      applyVisibility();
    }

    // ── Visibilidad: todo se deriva del estado, en un solo lugar ─────────
    function applyVisibility() {
      if (panel) panel.setAttribute('visible', shouldShowBookmarks() ? 'true' : 'false');
      if (bottomBar) bottomBar.setAttribute('visible', shouldShowBottomBar() ? 'true' : 'false');
      rebuildBottomBar();

      // El marco del foco espera a que la app tenga tabId y esté lista: si no,
      // se ve un marco vacío durante los ~100 ms entre abrir la pestaña y el
      // primer mensaje de la app.
      const showFocus = state.focusApp
        && !state.focusApp.minimized
        && state.focusApp.tabId > 0 && state.focusApp.ready;
      if (focusZone) focusZone.setAttribute('visible', showFocus ? 'true' : 'false');
      if (state.focusApp && state.focusApp.tabId > 0) {
        const entry = state.focusApp;
        if (entry._lastVisible !== !!showFocus) {
          tabVisible(entry.tabId, !!showFocus);
          sendMessageToApp(entry.tabId, showFocus ? 'resume' : 'suspend');
          entry._lastVisible = !!showFocus;
        }
      }

      applyAnchoredVisibility();
    }

    function applyAnchoredVisibility() {
      for (let i = 0; i < state.anchored.length; i++) {
        const a = state.anchored[i];
        ensureAnchoredFrame(a);
        updateAnchoredFramePose(a);

        const visible = !a.minimized && (a.alwaysOn || state.bookmarksVisible);
        if (a.frame && a.frame.group) {
          a.frame.group.setAttribute('visible', visible ? 'true' : 'false');
        }
        if (a.tabId > 0) tabVisible(a.tabId, visible);
        if (a._lastVisible !== visible) {
          // Sólo en la transición: si no, cada `applyVisibility` bombardearía
          // al worker con el mismo mensaje.
          if (a.tabId > 0) sendMessageToApp(a.tabId, visible ? 'resume' : 'suspend');
          a._lastVisible = visible;
        }
      }
    }

    // ── Plantar el shell al frente del visitante ─────────────────────────
    function repositionShellAtViewer() {
      const pose = viewerPose();
      if (!pose) return;
      const fwdX = -Math.sin(pose.yaw);
      const fwdZ = -Math.cos(pose.yaw);
      const px = pose.px + fwdX * DIST;
      const py = pose.py;
      const pz = pose.pz + fwdZ * DIST;
      const ry = pose.yaw;

      for (const g of [panel, bottomBar, focusZone]) {
        if (!g) continue;
        g.position = { x: px, y: py, z: pz };
        g.rotation = { x: 0, y: ry, z: 0 };
      }

      if (state.focusApp && !state.focusApp.minimized && state.focusApp.tabId > 0) {
        sendFocusSlotToApp();
      }

      // Las fijadas que NO son «siempre visibles» arrastran con el shell: se
      // recalcula su pose desde el offset local. Las otras quedan clavadas en
      // el mundo, que es lo que las hace fijas.
      for (let i = 0; i < state.anchored.length; i++) {
        const a = state.anchored[i];
        if (a.alwaysOn) continue;
        const wp = computeAnchoredWorldPose(a.offset);
        a.position = wp.position;
        a.rotation = wp.rotation;
        if (a.frame && a.frame.group) {
          a.frame.group.position = a.position;
          a.frame.group.rotation = a.rotation;
        }
        if (a.tabId > 0) sendAnchoredSlotToApp(i);
      }
    }

    // ── Abrir y cerrar el shell ──────────────────────────────────────────
    // Tres transiciones, en este orden:
    //   con una app en primer plano → se minimiza y vuelve el menú
    //   shell apagado              → se planta al frente y entra
    //   shell encendido            → se va
    function toggleShell() {
      if (state.focusApp && !state.focusApp.minimized) {
        state.focusApp.minimized = true;
        if (state.focusApp.tabId > 0) {
          tabVisible(state.focusApp.tabId, false);
          sendMessageToApp(state.focusApp.tabId, 'blur');
        }
        applyVisibility();
        startEnterAnimation();
        return;
      }

      const estaba = state.bookmarksVisible;
      state.bookmarksVisible = !estaba;
      if (!estaba) {
        repositionShellAtViewer();
        applyVisibility();
        startEnterAnimation();
      } else {
        applyVisibility();
        startExitAnimation();
      }
    }

    // ── El slot que se le manda a una app ────────────────────────────────
    // El host sigue a la entidad del marco cuadro a cuadro y la app recibe
    // coordenadas locales: así un worker ocupado no puede dejar su contenido
    // atrás.
    let nextSlotRevision = 0;
    function sendWindowSlot(entry, frame, size) {
      if (!entry || entry.tabId <= 0 || !frame) return;
      const revision = ++nextSlotRevision;
      entry.slotRevision = revision;
      frame._onResolved(function (anchorNodeId) {
        // La entrada pudo cerrarse o pedir otro slot mientras el nodo se
        // resolvía; mandarlo igual movería una ventana que ya no existe.
        if (entry.slotRevision !== revision ||
            (state.focusApp !== entry && !state.anchored.includes(entry))) return;
        sendMessageToApp(entry.tabId, 'slot', {
          coordinateSpace: 'window-local', anchorNodeId: anchorNodeId, revision: revision,
          position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0 }, size: size,
        });
      });
    }

    function sendFocusSlotToApp() {
      // El slot del foco es la pose del **shell**, no la del marco: los dos
      // comparten origen, y leer del shell evita que la ventana salte si el
      // visitante movió la cabeza entre abrir el menú y abrir la app.
      sendWindowSlot(state.focusApp, focusZone, { x: SLOT_W, y: SLOT_H, z: SLOT_D });
    }

    // ── El marco del foco ────────────────────────────────────────────────
    let focusFrame = null;

    function clearFocusFrame() {
      if (focusFrame) focusFrame.destroy();
      focusFrame = null;
    }

    function buildFocusFrame(title) {
      clearFocusFrame();
      if (!focusZone) return;
      focusFrame = UI.windowFrame(focusZone, {
        width: SLOT_W, height: SLOT_H, title: title,
        onClose: onTapFocusClose,
        onMinimize: onTapFocusMinimize,
        onAnchor: onTapFocusAnchor,
      });
    }

    function onTapFocusClose() {
      if (!state.focusApp) return;
      const tabId = state.focusApp.tabId;
      if (tabId <= 0) {
        // Todavía no llegó el tabId del host: se marca cancelada y el
        // reconocimiento la cierra cuando llegue.
        state.focusApp.cancelled = true;
        state.focusApp.supersededBy = null;
        state.focusApp.minimized = true;
        state.bookmarksVisible = true;
        applyVisibility();
        return;
      }
      sendMessageToApp(tabId, 'close');
      tabClose(tabId);
      clearFocusFrame();
      state.focusApp = null;
      state.bookmarksVisible = true;
      applyVisibility();
    }

    function onTapFocusMinimize() {
      if (!state.focusApp) return;
      state.focusApp.minimized = true;
      state.bookmarksVisible = true;
      if (state.focusApp.tabId > 0) tabVisible(state.focusApp.tabId, false);
      sendMessageToApp(state.focusApp.tabId, 'blur');
      applyVisibility();
    }

    function onTapFocusAnchor() {
      if (!state.focusApp) return;
      const tabId = state.focusApp.tabId;
      if (tabId <= 0) return;   // sin tabId todavía: esperar a 'ready'

      const idx = state.anchored.length;
      const offsetLocal = { x: ANCHORED_BASE_X + idx * ANCHORED_STACK_DX, y: 0, z: 0 };
      const worldPose = computeAnchoredWorldPose(offsetLocal);

      const entry = {
        tabId: tabId,
        title: state.focusApp.title || 'App',
        offset: offsetLocal,   // local: se recalcula cuando el shell se mueve
        position: worldPose.position,
        rotation: worldPose.rotation,
        minimized: false,
        alwaysOn: false,
        frame: null,
      };
      state.anchored.push(entry);
      sendMessageToApp(tabId, 'blur');

      // Se limpia el foco ANTES de mandar el slot nuevo: `sendAnchoredSlot`
      // lee de la entrada recién agregada, no del foco.
      state.focusApp = null;
      clearFocusFrame();
      sendAnchoredSlotToApp(state.anchored.length - 1);
      applyVisibility();
    }

    // ── Ventanas fijadas ─────────────────────────────────────────────────
    // Cada una vive en su propio grupo dentro de la zona de fijadas. El grupo
    // lleva la pose del mundo; el marco de adentro es local a él.
    function computeAnchoredWorldPose(offsetLocal) {
      const fallback = {
        position: { x: 0, y: 1.5, z: -1.5 },
        rotation: { x: 0, y: 0, z: 0 },
      };
      if (!panel) return fallback;
      let p, r;
      try { p = panel.position; } catch (_) { return fallback; }
      try { r = panel.rotation; } catch (_) { return fallback; }
      const yaw = Number(r.y || 0);
      const cos = Math.cos(yaw);
      const sin = Math.sin(yaw);
      const lx = Number(offsetLocal.x || 0);
      const ly = Number(offsetLocal.y || 0);
      const lz = Number(offsetLocal.z || 0);
      return {
        position: {
          x: Number(p.x || 0) + lx * cos + lz * sin,
          y: Number(p.y || 0) + ly,
          z: Number(p.z || 0) - lx * sin + lz * cos,
        },
        rotation: { x: 0, y: yaw, z: 0 },
      };
    }

    // La inversa: de una pose del mundo al offset local del shell. Se usa al
    // apagar «siempre visible», para que la ventana no salte al volver a
    // seguir al menú.
    function worldOffsetFromShell(worldPos) {
      if (!panel) return null;
      let p, r;
      try { p = panel.position; } catch (_) { return null; }
      try { r = panel.rotation; } catch (_) { return null; }
      const yaw = Number(r.y || 0);
      const cos = Math.cos(yaw);
      const sin = Math.sin(yaw);
      const dx = Number(worldPos.x) - Number(p.x || 0);
      const dy = Number(worldPos.y) - Number(p.y || 0);
      const dz = Number(worldPos.z) - Number(p.z || 0);
      return { x: dx * cos - dz * sin, y: dy, z: dx * sin + dz * cos };
    }

    function ensureAnchoredFrame(anc) {
      if (anc.frame) return anc.frame;
      if (!anchoredZone) return null;

      const group = root.createElement('group');
      group.position = anc.position;
      group.rotation = anc.rotation;
      anchoredZone.appendChild(group);

      const win = UI.windowFrame(group, {
        width: SLOT_W, height: SLOT_H, title: anc.title,
        onClose: function () { onTapAnchoredClose(state.anchored.indexOf(anc)); },
        onMinimize: function () { onTapAnchoredMinimize(state.anchored.indexOf(anc)); },
        onAnchor: function () { onTapAnchoredAlwaysOn(state.anchored.indexOf(anc)); },
      });

      anc.frame = { group: group, win: win };
      updateAnchoredFrameAOColor(anc);
      return anc.frame;
    }

    function clearAnchoredFrame(anc) {
      if (!anc || !anc.frame) return;
      if (anc.frame.win) anc.frame.win.destroy();
      try { if (anc.frame.group) anc.frame.group.remove(); } catch (_) {}
      anc.frame = null;
    }

    function updateAnchoredFramePose(anc) {
      if (!anc || !anc.frame || !anc.frame.group) return;
      anc.frame.group.position = anc.position;
      anc.frame.group.rotation = anc.rotation;
    }

    // Verde cuando la ventana se queda quieta en el mundo aunque el menú se
    // vaya; el color de la barra cuando sigue al shell.
    function updateAnchoredFrameAOColor(anc) {
      if (!anc || !anc.frame || !anc.frame.win || !anc.frame.win.anchorPiece) return;
      UI.recolor(anc.frame.win.anchorPiece, anc.alwaysOn ? '#2F7D5B' : UI.C.blue);
    }

    function sendAnchoredSlotToApp(idx) {
      const a = state.anchored[idx];
      if (!a || a.tabId <= 0) return;
      ensureAnchoredFrame(a);
      sendWindowSlot(a, a.frame && a.frame.group, { x: SLOT_W, y: SLOT_H, z: SLOT_D });
    }

    function onTapAnchoredClose(idx) { unmountAnchored(idx); }

    function onTapAnchoredMinimize(idx) {
      const a = state.anchored[idx];
      if (!a) return;
      a.minimized = true;
      sendMessageToApp(a.tabId, 'suspend');
      applyVisibility();
    }

    function onTapAnchoredAlwaysOn(idx) {
      const a = state.anchored[idx];
      if (!a) return;
      a.alwaysOn = !a.alwaysOn;
      // Al volver a seguir al shell hay que recalcular el offset contra la
      // pose ACTUAL: con el offset viejo la ventana daría un salto.
      if (!a.alwaysOn) {
        const off = worldOffsetFromShell(a.position);
        if (off) a.offset = off;
      }
      updateAnchoredFrameAOColor(a);
      applyVisibility();
    }

    function unmountAnchored(idx) {
      const a = state.anchored[idx];
      if (!a) return;
      sendMessageToApp(a.tabId, 'close');
      tabClose(a.tabId);
      clearAnchoredFrame(a);
      state.anchored.splice(idx, 1);
      barSignature = null;   // cambió la composición de la barra
      applyVisibility();
    }

    // ── Abrir un marcador ────────────────────────────────────────────────
    function openBookmark(bm) {
      if (typeof tabs.open !== 'function') {
        console.warn('[shell_app] sin tabs — navegando directo');
        navigate(bm.url);
        return;
      }
      if (bm.kind === 'app-embedded') { openEmbeddedApp(bm); return; }
      tabOpen(bm.url, { kind: bm.kind || 'spatial' });
      state.bookmarksVisible = false;
      applyVisibility();
      startExitAnimation();
    }

    // Una app embebida no cierra el menú: se queda como foco y el panel se
    // oculta solo, por la regla de visibilidad.
    function openEmbeddedApp(bm) {
      state.bookmarksVisible = true;

      // Si ya hay una abriéndose sin tabId, se la marca superada en vez de
      // abrir dos: el reconocimiento del host llega igual y cerraría la
      // primera dejando una pestaña huérfana.
      if (state.focusApp && state.focusApp.tabId === 0) {
        state.focusApp.cancelled = true;
        state.focusApp.supersededBy = bm;
        return;
      }
      if (state.focusApp) {
        sendMessageToApp(state.focusApp.tabId, 'close');
        tabClose(state.focusApp.tabId);
        clearFocusFrame();
        state.focusApp = null;
      }

      // La identidad la asigna el host en su reconocimiento. Un mensaje de una
      // app no puede reclamar una ventana pendiente.
      state.focusApp = {
        tabId: 0,
        minimized: false,
        ready: false,
        title: bm.name,
        pendingUrl: bm.url,
      };
      barSignature = null;
      tabOpen(bm.url, { kind: 'app-embedded' });
      applyVisibility();
    }

    // ── Mensajes de las apps ─────────────────────────────────────────────
    function handleAppMessage(msg) {
      const fromTabId = Number(msg.fromTabId || 0);
      let parsed = null;
      try { parsed = JSON.parse(msg.payload); } catch (_) {}
      if (!parsed || typeof parsed !== 'object') return;
      const type = String(parsed.type || '').toLowerCase();

      // Sólo el host asigna identidad: sin esto, una app ajena o tardía podría
      // quedarse con la ventana pendiente mandando su primer mensaje.
      if (type === 'tabopened' && fromTabId === 0) {
        const entry = state.focusApp;
        const tabId = Number(parsed.tabId);
        if (!entry || entry.tabId !== 0 || !(tabId > 0)) return;
        entry.tabId = tabId;
        if (entry.cancelled) {
          const replacement = entry.supersededBy;
          sendMessageToApp(tabId, 'close');
          tabClose(tabId);
          state.focusApp = null;
          if (replacement) openEmbeddedApp(replacement);
          else applyVisibility();
          return;
        }
        buildFocusFrame(entry.title || 'App');
        barSignature = null;
        applyVisibility();
        return;
      }

      if (state.focusApp && state.focusApp.tabId === fromTabId
          && (type === 'ready' || type === 'requestslot') && !state.focusApp.ready) {
        state.focusApp.ready = true;
        sendFocusSlotToApp();
        applyVisibility();
      }

      if (type === 'requestslot') {
        // Se le contesta sólo al que pidió, con su propio slot.
        if (state.focusApp && state.focusApp.tabId === fromTabId) {
          sendFocusSlotToApp();
        } else {
          const idx = state.anchored.findIndex(a => a.tabId === fromTabId);
          if (idx >= 0) sendAnchoredSlotToApp(idx);
        }
      } else if (type === 'closeself') {
        if (state.focusApp && state.focusApp.tabId === fromTabId) {
          onTapFocusClose();
        } else {
          const idx = state.anchored.findIndex(a => a.tabId === fromTabId);
          if (idx >= 0) unmountAnchored(idx);
        }
      }
    }

    function pollInbox() {
      const msgs = poll();
      if (Array.isArray(msgs) && msgs.length) {
        for (const m of msgs) handleAppMessage(m);
      }
      setTimeout(pollInbox, 16);
    }
    pollInbox();

    applyVisibility();

    return {
      state: state,
      toggle: toggleShell,
      applyVisibility: applyVisibility,
      reposition: repositionShellAtViewer,
      openBookmark: openBookmark,
      board: board,
      bar: bar,
    };
  }

  globalThis.ShellApp = { mount: mount };
  console.log('[shell_app] listo');
})();
