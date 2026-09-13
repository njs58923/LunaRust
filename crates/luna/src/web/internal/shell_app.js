// ─────────────────────────────────────────────────────────────────────────────
// shell_app.js — la mitad del shell que mira al host.
//
// El estado, las ventanas de apps embebidas y todo lo que sale hacia el motor.
// El **menú** no está acá: vive en `luna://shell_menu`, montado como
// `<include>` con su propio isolate, y se comunica por el canal de componentes
// —props para abajo, eventos `component:*` para arriba—.
//
// **Por qué justo ahí pasa el corte.** El motor exige que el nodo ancla de una
// ventana embebida pertenezca al space que manda el slot: `bind_embedded_slot`
// compara `find_owner_space_id(anchor)` contra el que envía y si no coinciden
// rechaza. El space que envía es el del shell, que es el primer `<space>` bajo
// el wrapper marcado `system-shell` — o sea éste, no el del include. Un marco de
// ventana dibujado del otro lado tendría el ancla en otro space y el slot sería
// rechazado en silencio. Por eso el menú se va y las ventanas se quedan.
//
// Lo demás sí cruza sin problema: los eventos de puntero sobre los nodos del
// include se entregan a su propio isolate —`find_owner_space_id` sube hasta el
// space más cercano— así que el menú maneja sus botones solo y sólo avisa qué
// se tocó.
//
// `cfg`:
//   root, menuInclude, focusZone, anchoredZone   — nodos
//   bookmarks   — [{ id?, name, url, kind }] para montajes independientes
//   sharedItems — usar el catálogo publicado por el root en component.props
//   tabs        — { open(url, opts), close(id), setVisible(id, visible) }
//   send(tabId, payload) / poll()   — mensajería con las apps
//   viewerPose()                    — { px, py, pz, yaw } o null
//   navigate(url)                   — salida de emergencia sin pestañas
//
// **Regla de oro: todo cambio en `shellState` va seguido de `applyVisibility()`.**
// Nada dibuja por su cuenta; el menú entero es función de las props que se le
// mandan desde ahí.
// ─────────────────────────────────────────────────────────────────────────────
(function () {
  function mount(cfg) {
    const root = cfg.root;
    const D = globalThis.ShellDraw.init(root);
    const M = D.M;

    const menuInclude = cfg.menuInclude;
    const focusZone = cfg.focusZone;
    const anchoredZone = cfg.anchoredZone;
    let BOOKMARKS = cfg.bookmarks || [];

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
    // El lienzo de una app. **Va con la distancia a la que se planta el
    // shell**: el mismo tamaño a 2,6 m se ve la mitad que a 1,5, y la ventana
    // queda flotando chiquita al fondo en vez de ocupar el lugar donde estaba
    // la grilla. Cada controller pasa el suyo.
    const slot = cfg.slot || {};
    const FOCUS_SLOT_W = slot.w || 1.0;
    const FOCUS_SLOT_H = slot.h || 0.7;
    const FOCUS_SLOT_D = slot.d || 0.1;
    // La barra de título va **debajo** de la ventana y suelta, como una
    // pastilla flotante. Arriba competía con el contenido: le comía el borde
    // superior y, con la ventana a la altura de los ojos, tapaba justo lo que
    // se estaba mirando.
    const TITLE_H = 0.075;
    const TITLE_GAP = 0.026;      // de la ventana a la pastilla
    const TITLE_RADIUS = 0.032;   // METROS: es una caja, no un plano
    const TITLE_SIZE = 0.030;
    const FOCUS_BTN = 0.046;
    const FOCUS_BTN_GAP = 0.013;
    const FOCUS_MARGIN = 0.022;   // del último botón al filo de la pastilla
    const SHELL_SPAWN_DISTANCE = cfg.distance || 1.5;

    /** Cuánto te podés alejar antes de que el menú se cierre solo, en metros.
     *
     *  El menú se planta donde estabas parado y no te sigue: dar dos pasos lo
     *  deja flotando de costado, o atrás. Cerrarlo es la lectura correcta de
     *  ese gesto — si te fuiste, no lo estabas mirando. */
    const CIERRE_POR_DISTANCIA = cfg.closeDistance || 0.5;

    /** Dónde estaba el visitante cuando se plantó el menú. Es su posición, no
     *  la del menú: la pregunta es cuánto te moviste vos. */
    let anclaVisitante = null;
    let ultimaMirada = 0;

    // Cuánto se levanta la ventana respecto de la altura de la mirada.
    //
    // Con la barra de título abajo, el borde inferior de una ventana llega
    // más lejos que antes, y en escritorio —donde la ventana es grande y el
    // menú se planta lejos— terminaba pisando la barra del shell. La barra
    // vive dentro del include del menú, así que el shell no puede medir dónde
    // está: el número lo pone el controller, que es quien eligió las dos
    // cosas que chocan.
    const WINDOW_LIFT = cfg.windowLift || 0;

    // Offsets laterales para apartar las fijadas del frente del shell. Se
    // apilan a la derecha; el par de números es pragmático, no un layout.
    const ANCHORED_BASE_X = 1.15;
    const ANCHORED_STACK_DX = 0.10;
    const ANCHORED_SLOT_W = FOCUS_SLOT_W;
    const ANCHORED_SLOT_H = FOCUS_SLOT_H;
    const ANCHORED_SLOT_D = FOCUS_SLOT_D;

    // ── Estado ───────────────────────────────────────────────────────────
    const shellState = {
      bookmarksVisible: false,
      focusApp: null,   // { tabId, minimized, ready, title, … }
      anchored: [],     // [{ tabId, position, rotation, minimized, alwaysOn, frame }]
    };

    function shouldShowBookmarks() {
      if (!shellState.bookmarksVisible) return false;
      // Con una app en primer plano el panel estorba: ocupa el mismo lugar.
      if (shellState.focusApp && !shellState.focusApp.minimized && shellState.focusApp.ready) return false;
      return true;
    }

    // La barra sobrevive al panel: el panel se va cuando una app toma el
    // frente, la barra se queda mientras dure la sesión del shell. Es lo único
    // que permite volver al menú desde adentro de una app, y además ahora lleva
    // estado —usuario, mundo, avisos—, que no tiene por qué desaparecer porque
    // haya una ventana abierta.
    function shouldShowBottomBar() {
      return shellState.bookmarksVisible;
    }

    // ── El menú, por props ───────────────────────────────────────────────
    // Se le manda el estado ya masticado: qué marcadores hay, qué ventanas hay
    // y si se ve. El menú no consulta nada; dibuja lo que le llega.
    //
    // Se compara contra lo último enviado porque `applyVisibility()` corre en
    // cada transición y escribir props iguales despierta al isolate del include
    // para nada.
    let lastProps = null;

    // **La ventana espera a que el menú termine de irse.** Al abrir una app
    // embebida las dos cosas caían en el mismo cuadro: el menú arrancaba su
    // cascada de salida y la ventana aparecía encima, y eso no se lee como
    // una transición sino como un parpadeo. El menú avisa por el canal
    // cuando terminó —es el único que sabe cuánto dura, porque depende de
    // cuántos marcadores tenga— y hasta entonces la ventana se queda quieta.
    let menuMostrado = false;
    let esperandoMenu = false;
    let esperaTimer = null;

    function empezarEspera() {
      if (esperandoMenu) return;
      esperandoMenu = true;
      // Red de seguridad: si el aviso no llega —el menú no llegó a dibujarse,
      // su isolate murió, el include no montó— la ventana no puede quedarse
      // escondida para siempre. El plazo es holgado contra la cascada.
      if (esperaTimer) clearTimeout(esperaTimer);
      esperaTimer = setTimeout(terminarEspera, 600);
    }

    function terminarEspera() {
      if (!esperandoMenu) return;
      esperandoMenu = false;
      if (esperaTimer) { clearTimeout(esperaTimer); esperaTimer = null; }
      applyVisibility();
    }

    function pushMenuProps() {
      if (!menuInclude) return;
      const windows = [];
      if (shellState.focusApp) windows.push({ dim: !!shellState.focusApp.minimized });
      for (const a of shellState.anchored) windows.push({ dim: !!a.minimized });
      const next = {
        // Dos banderas, no una. La barra sobrevive al panel a propósito.
        panelVisible: shouldShowBookmarks(),
        barVisible: shouldShowBottomBar(),
        // `glyph` sólo si el marcador lo trae: un `undefined` no cruza de
        // isolate y la excepción corta el montaje entero del shell.
        bookmarks: BOOKMARKS.map(function (b) { return { id: b.id || null, name: b.name, glyph: b.glyph || null }; }),
        windows: windows,
      };
      const firma = JSON.stringify(next);
      if (firma === lastProps) return;
      lastProps = firma;
      menuInclude.props = next;
    }

    // Lo que el menú avisa. Llega como `component:<tipo>` sobre el nodo del
    // include, en este isolate.
    function wireMenuEvents() {
      if (!menuInclude) return;
      menuInclude.addEventListener('component:open', function (e) {
        const idx = e.detail && e.detail.index;
        const id = e.detail && e.detail.id;
        const bm = typeof id === 'string' ? BOOKMARKS.find(b => b.id === id) : BOOKMARKS[idx];
        if (bm) openBookmark(bm);
      });
      menuInclude.addEventListener('component:menu', onTapBarMenu);
      menuInclude.addEventListener('component:hidden', terminarEspera);
      menuInclude.addEventListener('component:close', function (e) {
        const idx = e.detail && e.detail.index;
        if (typeof idx !== 'number') return;
        const anc = mapaVentana(idx);
        if (anc === null) onTapFocusClose();
        else unmountAnchored(anc);
      });
      menuInclude.addEventListener('component:window', function (e) {
        const idx = e.detail && e.detail.index;
        if (typeof idx !== 'number') return;
        const anc = mapaVentana(idx);
        if (anc === null) onTapBarFocus();
        else onTapBarAnchored(anc);
      });
    }

    // El índice que manda el menú sigue el orden con que se armaron las props:
    // primero el foco —si lo hay— y después las fijadas. Devuelve `null` para
    // el foco, o el índice dentro de `anchored`.
    function mapaVentana(idx) {
      if (shellState.focusApp) return idx === 0 ? null : idx - 1;
      return idx;
    }

    // Handlers de la barra ─
    function onTapBarMenu() {
      if (shellState.focusApp && !shellState.focusApp.minimized) {
        shellState.focusApp.minimized = true;
        if (shellState.focusApp.tabId > 0) {
          tabVisible(shellState.focusApp.tabId, false);
          sendMessageToApp(shellState.focusApp.tabId, 'blur');
        }
        shellState.bookmarksVisible = true;
        applyVisibility();
        return;
      }
      // Sin foco ya estamos en el menú: no hay nada que hacer.
    }

    function onTapBarFocus() {
      if (!shellState.focusApp || !shellState.focusApp.ready) return;
      shellState.focusApp.minimized = false;
      if (shellState.focusApp.tabId > 0) {
        tabVisible(shellState.focusApp.tabId, true);
        sendMessageToApp(shellState.focusApp.tabId, 'focus');
        sendFocusSlotToApp();
      }
      applyVisibility();
    }

    function onTapBarAnchored(idx) {
      const a = shellState.anchored[idx];
      if (!a) return;
      a.minimized = !a.minimized;
      sendMessageToApp(a.tabId, a.minimized ? 'suspend' : 'resume');
      applyVisibility();
    }

    // ── Visibilidad: todo se deriva del estado, en un solo lugar ─────────
    function applyVisibility() {
      const quiereMenu = shouldShowBookmarks();
      const seVaElMenu = menuMostrado && !quiereMenu;
      menuMostrado = quiereMenu;
      pushMenuProps();

      // El marco del foco espera a que la app tenga tabId y esté lista: si no,
      // se ve un marco vacío durante los ~100 ms entre abrir la pestaña y el
      // primer mensaje de la app.
      const quiereFoco = shellState.focusApp
        && !shellState.focusApp.minimized
        && shellState.focusApp.tabId > 0 && shellState.focusApp.ready;

      // Sólo se espera si el menú de verdad estaba puesto: si ya no se veía
      // no hay ninguna animación en curso y esperar sería demorar por nada.
      if (seVaElMenu && quiereFoco) empezarEspera();
      const showFocus = quiereFoco && !esperandoMenu;
      if (focusZone) focusZone.setAttribute('visible', showFocus ? 'inherit' : 'false');
      if (shellState.focusApp && shellState.focusApp.tabId > 0) {
        const entry = shellState.focusApp;
        if (entry._lastVisible !== !!showFocus) {
          tabVisible(entry.tabId, !!showFocus);
          sendMessageToApp(entry.tabId, showFocus ? 'resume' : 'suspend');
          entry._lastVisible = !!showFocus;
        }
      }

      applyAnchoredVisibility();
    }

    function applyAnchoredVisibility() {
      for (let i = 0; i < shellState.anchored.length; i++) {
        const a = shellState.anchored[i];
        ensureAnchoredFrame(a);
        updateAnchoredFramePose(a);

        const visible = !a.minimized && (a.alwaysOn || shellState.bookmarksVisible);
        if (a.frame && a.frame.group) {
          a.frame.group.setAttribute('visible', visible ? 'inherit' : 'false');
        }
        if (a.tabId > 0) tabVisible(a.tabId, visible);
        if (a._lastVisible !== visible) {
          // Sólo en la transición: si no, cada `applyVisibility` bombardearía al
          // worker con el mismo mensaje.
          if (a.tabId > 0) sendMessageToApp(a.tabId, visible ? 'resume' : 'suspend');
          a._lastVisible = visible;
        }
      }
    }

    // ── Plantar el shell al frente del visitante ─────────────────────────
    function repositionShellAtViewer() {
      const pose = viewerPose();
      if (!pose) return;
      // Acá es donde el menú deja de seguirte: se planta y se queda. Por eso
      // este es el punto contra el que se mide después.
      anclaVisitante = { px: pose.px, pz: pose.pz };
      ultimaMirada = 0;
      const fwdX = -Math.sin(pose.yaw);
      const fwdZ = -Math.cos(pose.yaw);
      const px = pose.px + fwdX * SHELL_SPAWN_DISTANCE;
      const py = pose.py;
      const pz = pose.pz + fwdZ * SHELL_SPAWN_DISTANCE;
      const ry = pose.yaw;

      // El include se mueve como cualquier nodo: es el padre quien lo ubica, y
      // adentro el menú dibuja en coordenadas locales sin enterarse.
      if (menuInclude) {
        menuInclude.position = { x: px, y: py, z: pz };
        menuInclude.rotation = { x: 0, y: ry, z: 0 };
      }
      if (focusZone) {
        focusZone.position = { x: px, y: py + WINDOW_LIFT, z: pz };
        focusZone.rotation = { x: 0, y: ry, z: 0 };
      }

      if (shellState.focusApp && !shellState.focusApp.minimized && shellState.focusApp.tabId > 0) {
        sendFocusSlotToApp();
      }

      // Las fijadas que NO son «siempre visibles» arrastran con el shell: se
      // recalcula su pose desde el offset local. Las otras quedan clavadas en el
      // mundo, que es lo que las hace fijas.
      for (let i = 0; i < shellState.anchored.length; i++) {
        const a = shellState.anchored[i];
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
      if (shellState.focusApp && !shellState.focusApp.minimized) {
        shellState.focusApp.minimized = true;
        if (shellState.focusApp.tabId > 0) {
          tabVisible(shellState.focusApp.tabId, false);
          sendMessageToApp(shellState.focusApp.tabId, 'blur');
        }
        applyVisibility();
        return;
      }

      const estaba = shellState.bookmarksVisible;
      shellState.bookmarksVisible = !estaba;
      if (!estaba) repositionShellAtViewer();
      applyVisibility();
    }

    // ── El marco de una ventana ──────────────────────────────────────────
    // Lo dibuja el padre, no el include, por lo del ancla. Es la única parte de
    // la interfaz que quedó de este lado.
    function windowTitle(title) {
      const value = title || 'App';
      return value.length > 24 ? value.slice(0, 23) + '…' : value;
    }

    function buildWindowFrame(parent, opts) {
      const W = opts.width;
      const H = opts.height;
      const nodes = [];
      const pieces = [];
      const add = function (n) { nodes.push(n); return n; };

      // El contenido y su fondo pertenecen a la app. El shell conserva sólo
      // la barra de ventana y el ancla; las apps transparentes no llevan placa.

      // La pastilla del título: **debajo**, suelta, y sólo tan ancha como lo
      // que lleva. Ocupar todo el ancho de la ventana la convertiría otra vez
      // en un marco.
      const botones = [
        { glyph: 'close', color: '#B0413E', onTap: opts.onClose },
        { glyph: 'minimize', color: M.PLACA_ALTA, onTap: opts.onMinimize },
        { glyph: 'anchor', color: '#30435F', onTap: opts.onAnchor },
      ].filter(function (b) { return typeof b.onTap === 'function'; });

      const btnBlock = botones.length * FOCUS_BTN + (botones.length - 1) * FOCUS_BTN_GAP;
      const titleW = Math.max(0.30, btnBlock + 0.26);
      const ty = -H / 2 - TITLE_GAP - TITLE_H / 2;
      add(D.pill(parent, { x: 0, y: ty, z: -0.004, w: titleW, h: TITLE_H,
                           r: TITLE_RADIUS, color: M.PLACA }));

      // El título se centra en lo que sobra a la izquierda de los botones, no
      // en la pastilla entera: centrado en la pastilla queda debajo de ellos.
      const btnLeft = titleW / 2 - FOCUS_MARGIN - btnBlock;
      const title = root.createElement('text');
      title.setAttribute('value', windowTitle(opts.title));
      title.setAttribute('size', String(TITLE_SIZE));
      title.setAttribute('color', M.TENUE);
      title.setAttribute('touchable', 'false');
      title.position = { x: (-titleW / 2 + btnLeft) / 2, y: ty, z: 0.006 };
      parent.appendChild(title);
      nodes.push(title);

      // El orden es el de **colocación**, que va de derecha a izquierda: el
      // primero queda pegado al filo. Así, leídos de izquierda a derecha, salen
      // fijar · minimizar · cerrar, con el rojo en la punta.
      let bx = titleW / 2 - FOCUS_MARGIN - FOCUS_BTN / 2;
      for (const b of botones) {
        pieces.push(D.piece(parent, {
          x: bx, y: ty, size: FOCUS_BTN, corner: 0.5, color: b.color,
          glyph: b.glyph, padding: 0.30,
          // Sobre la pastilla, no dentro de ella: la caja tiene espesor y a la
          // misma z las dos caras pelean por el mismo píxel.
          zOffset: 0.008,
          onTap: b.onTap,
        }));
        bx -= FOCUS_BTN + FOCUS_BTN_GAP;
      }

      return {
        nodes: nodes, pieces: pieces, title: title,
        anchorPiece: pieces[pieces.length - 1] || null,
        destroy: function () {
          for (const p of pieces) D.dropPiece(p);
          for (const n of nodes) { try { n.remove(); } catch (_) {} }
        },
      };
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
      focusFrame = buildWindowFrame(focusZone, {
        width: FOCUS_SLOT_W, height: FOCUS_SLOT_H, title: title,
        onClose: onTapFocusClose,
        onMinimize: onTapFocusMinimize,
        onAnchor: onTapFocusAnchor,
      });
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
            (shellState.focusApp !== entry && !shellState.anchored.includes(entry))) return;
        sendMessageToApp(entry.tabId, 'slot', {
          coordinateSpace: 'window-local', anchorNodeId: anchorNodeId, revision: revision,
          position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0 }, size: size,
        });
      });
    }

    function sendFocusSlotToApp() {
      sendWindowSlot(shellState.focusApp, focusZone,
        { x: FOCUS_SLOT_W, y: FOCUS_SLOT_H, z: FOCUS_SLOT_D });
    }

    // ── Handlers de la barra de título del foco ──────────────────────────
    function onTapFocusClose() {
      if (!shellState.focusApp) return;
      const tabId = shellState.focusApp.tabId;
      if (tabId <= 0) {
        // Todavía no llegó el tabId del host: se marca cancelada y el
        // reconocimiento la cierra cuando llegue.
        shellState.focusApp.cancelled = true;
        shellState.focusApp.supersededBy = null;
        shellState.focusApp.minimized = true;
        shellState.bookmarksVisible = true;
        applyVisibility();
        return;
      }
      sendMessageToApp(tabId, 'close');
      tabClose(tabId);
      clearFocusFrame();
      shellState.focusApp = null;
      shellState.bookmarksVisible = true;
      applyVisibility();
    }

    function onTapFocusMinimize() {
      if (!shellState.focusApp) return;
      shellState.focusApp.minimized = true;
      shellState.bookmarksVisible = true;
      if (shellState.focusApp.tabId > 0) tabVisible(shellState.focusApp.tabId, false);
      sendMessageToApp(shellState.focusApp.tabId, 'blur');
      applyVisibility();
    }

    function onTapFocusAnchor() {
      if (!shellState.focusApp) return;
      const tabId = shellState.focusApp.tabId;
      if (tabId <= 0) return;   // sin tabId todavía: esperar a 'ready'

      const idx = shellState.anchored.length;
      const offsetLocal = { x: ANCHORED_BASE_X + idx * ANCHORED_STACK_DX, y: 0, z: 0 };
      const worldPose = computeAnchoredWorldPose(offsetLocal);

      const entry = {
        tabId: tabId,
        title: shellState.focusApp.title || 'App',
        offset: offsetLocal,   // local: se recalcula cuando el shell se mueve
        position: worldPose.position,
        rotation: worldPose.rotation,
        minimized: false,
        alwaysOn: false,
        frame: null,
      };
      shellState.anchored.push(entry);
      sendMessageToApp(tabId, 'blur');

      // Se limpia el foco ANTES de mandar el slot nuevo: `sendAnchoredSlot` lee
      // de la entrada recién agregada, no del foco.
      shellState.focusApp = null;
      clearFocusFrame();
      sendAnchoredSlotToApp(shellState.anchored.length - 1);
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
      // La pose del shell se lee del include, que es el nodo que el shell mueve
      // al plantarse. Antes se leía del panel; ahora el panel está adentro.
      if (!menuInclude) return fallback;
      let p, r;
      try { p = menuInclude.position; } catch (_) { return fallback; }
      try { r = menuInclude.rotation; } catch (_) { return fallback; }
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
    // apagar «siempre visible», para que la ventana no salte al volver a seguir
    // al menú.
    function worldOffsetFromShell(worldPos) {
      if (!menuInclude) return null;
      let p, r;
      try { p = menuInclude.position; } catch (_) { return null; }
      try { r = menuInclude.rotation; } catch (_) { return null; }
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

      const win = buildWindowFrame(group, {
        width: ANCHORED_SLOT_W, height: ANCHORED_SLOT_H, title: anc.title,
        onClose: function () { onTapAnchoredClose(shellState.anchored.indexOf(anc)); },
        onMinimize: function () { onTapAnchoredMinimize(shellState.anchored.indexOf(anc)); },
        onAnchor: function () { onTapAnchoredAlwaysOn(shellState.anchored.indexOf(anc)); },
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
      D.recolorPiece(anc.frame.win.anchorPiece, anc.alwaysOn ? '#2F7D5B' : '#30435F');
    }

    function sendAnchoredSlotToApp(idx) {
      const a = shellState.anchored[idx];
      if (!a || a.tabId <= 0) return;
      ensureAnchoredFrame(a);
      sendWindowSlot(a, a.frame && a.frame.group,
        { x: ANCHORED_SLOT_W, y: ANCHORED_SLOT_H, z: ANCHORED_SLOT_D });
    }

    function onTapAnchoredClose(idx) { unmountAnchored(idx); }

    function onTapAnchoredMinimize(idx) {
      const a = shellState.anchored[idx];
      if (!a) return;
      a.minimized = true;
      sendMessageToApp(a.tabId, 'suspend');
      applyVisibility();
    }

    function onTapAnchoredAlwaysOn(idx) {
      const a = shellState.anchored[idx];
      if (!a) return;
      a.alwaysOn = !a.alwaysOn;
      // Al volver a seguir al shell hay que recalcular el offset contra la pose
      // ACTUAL: con el offset viejo la ventana daría un salto.
      if (!a.alwaysOn) {
        const off = worldOffsetFromShell(a.position);
        if (off) a.offset = off;
      }
      updateAnchoredFrameAOColor(a);
      applyVisibility();
    }

    function unmountAnchored(idx) {
      const a = shellState.anchored[idx];
      if (!a) return;
      sendMessageToApp(a.tabId, 'close');
      tabClose(a.tabId);
      clearAnchoredFrame(a);
      shellState.anchored.splice(idx, 1);
      applyVisibility();
    }

    // ── Abrir un marcador ────────────────────────────────────────────────
    function openBookmark(bm) {
      if (typeof tabs.open !== 'function') {
        console.warn('[shell_app] sin pestañas — navegando directo');
        navigate(bm.url);
        return;
      }
      if (bm.kind === 'app-embedded') { openEmbeddedApp(bm); return; }
      tabOpen(bm.url, { kind: bm.kind || 'spatial' });
      shellState.bookmarksVisible = false;
      applyVisibility();
    }

    // Una app embebida no cierra el menú: se queda como foco y el panel se
    // oculta solo, por la regla de visibilidad.
    function openEmbeddedApp(bm) {
      shellState.bookmarksVisible = true;

      // Si ya hay una abriéndose sin tabId, se la marca superada en vez de abrir
      // dos: el reconocimiento del host llega igual y cerraría la primera
      // dejando una pestaña huérfana.
      if (shellState.focusApp && shellState.focusApp.tabId === 0) {
        shellState.focusApp.cancelled = true;
        shellState.focusApp.supersededBy = bm;
        return;
      }
      if (shellState.focusApp) {
        sendMessageToApp(shellState.focusApp.tabId, 'close');
        tabClose(shellState.focusApp.tabId);
        clearFocusFrame();
        shellState.focusApp = null;
      }

      // La identidad la asigna el host en su reconocimiento. Un mensaje de una
      // app no puede reclamar una ventana pendiente.
      shellState.focusApp = {
        tabId: 0,
        minimized: false,
        ready: false,
        title: bm.name,
        pendingUrl: bm.url,
      };
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
        const entry = shellState.focusApp;
        const tabId = Number(parsed.tabId);
        if (!entry || entry.tabId !== 0 || !(tabId > 0)) return;
        entry.tabId = tabId;
        if (entry.cancelled) {
          const replacement = entry.supersededBy;
          sendMessageToApp(tabId, 'close');
          tabClose(tabId);
          shellState.focusApp = null;
          if (replacement) openEmbeddedApp(replacement);
          else applyVisibility();
          return;
        }
        buildFocusFrame(entry.title || 'App');
        applyVisibility();
        return;
      }

      if (shellState.focusApp && shellState.focusApp.tabId === fromTabId
          && (type === 'ready' || type === 'requestslot') && !shellState.focusApp.ready) {
        shellState.focusApp.ready = true;
        sendFocusSlotToApp();
        applyVisibility();
      }

      if (type === 'requestslot') {
        // Se le contesta sólo al que pidió, con su propio slot.
        if (shellState.focusApp && shellState.focusApp.tabId === fromTabId) {
          sendFocusSlotToApp();
        } else {
          const idx = shellState.anchored.findIndex(function (a) { return a.tabId === fromTabId; });
          if (idx >= 0) sendAnchoredSlotToApp(idx);
        }
      } else if (type === 'closeself') {
        if (shellState.focusApp && shellState.focusApp.tabId === fromTabId) {
          onTapFocusClose();
        } else {
          const idx = shellState.anchored.findIndex(function (a) { return a.tabId === fromTabId; });
          if (idx >= 0) unmountAnchored(idx);
        }
      }
    }

    /** Cerrar el menú si el visitante se alejó del lugar donde lo abrió.
     *
     *  Se mide **en el plano**, sin la altura: agacharse o pararse en puntas de
     *  pie mueve la cabeza medio metro y no es irse a ningún lado.
     *
     *  Sólo cuenta mientras el menú se está viendo de verdad. Con una app en
     *  primer plano el menú ya está oculto, y cerrarlo por debajo dejaría el
     *  estado cambiado sin que nadie lo haya pedido.
     *
     *  El ancla se limpia sola cuando el menú no está: reabrirlo pasa otra vez
     *  por `repositionShellAtViewer`, que lo vuelve a plantar donde estés. */
    function vigilarDistancia(ahora) {
      if (!shouldShowBookmarks()) { anclaVisitante = null; return; }
      if (!anclaVisitante) return;
      // Seis veces por segundo alcanza para algo que se mide en pasos, y evita
      // pedirle la pose al motor sesenta veces por segundo para nada.
      if (ahora - ultimaMirada < 160) return;
      ultimaMirada = ahora;

      const pose = viewerPose();
      if (!pose) return;
      const dx = pose.px - anclaVisitante.px;
      const dz = pose.pz - anclaVisitante.pz;
      if (dx * dx + dz * dz < CIERRE_POR_DISTANCIA * CIERRE_POR_DISTANCIA) return;

      anclaVisitante = null;
      shellState.bookmarksVisible = false;
      applyVisibility();
    }

    function pollInbox() {
      const msgs = poll();
      if (Array.isArray(msgs) && msgs.length) {
        for (const m of msgs) handleAppMessage(m);
      }
      vigilarDistancia(Date.now());
      setTimeout(pollInbox, 16);
    }

    // ── Arranque ─────────────────────────────────────────────────────────
    // Suscripción al catálogo del root, sin polling ni storage del controller.
    // Otros consumidores pueden representar todos los campos como prefieran.
    if (cfg.sharedItems && globalThis.component) {
      function readSharedItems() {
        const catalog = component.props && component.props.shellItems;
        if (!catalog || !Array.isArray(catalog.items)) return;
        BOOKMARKS = catalog.items.filter(item => typeof item.name === 'string' && typeof item.url === 'string');
        pushMenuProps();
      }
      component.addEventListener('propschange', readSharedItems);
      component.addEventListener('connect', readSharedItems);
      readSharedItems();
    }
    wireMenuEvents();
    pollInbox();
    applyVisibility();

    return {
      state: shellState,
      toggle: toggleShell,
      reposition: repositionShellAtViewer,
      apply: applyVisibility,
      applyVisibility: applyVisibility,
      openBookmark: openBookmark,
      open: openEmbeddedApp,
      message: handleAppMessage,
      anchor: onTapFocusAnchor,
      close: onTapFocusClose,
      minimize: onTapFocusMinimize,
      restore: onTapBarFocus,
    };
  }

  globalThis.ShellApp = { mount: mount };
  console.log('[shell_app] listo');
})();
