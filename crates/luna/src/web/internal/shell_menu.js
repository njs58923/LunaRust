// ─────────────────────────────────────────────────────────────────────────────
// shell_menu.js — el menú del shell, como componente.
//
// Corre adentro de un `<include>`, en su propio isolate. **No sabe nada del
// host**: no abre pestañas, no lee la pose del visitante, no le habla a las
// apps. Recibe qué dibujar por `component.props` y avisa qué se tocó con
// `component.emit`. El controller que lo incluye hace el resto.
//
// El reparto no es de gusto: el motor exige que el nodo ancla de una ventana
// embebida pertenezca al space que manda el slot (`bind_embedded_slot` en
// `js.rs`), y ese space es el del shell, no el del include. Por eso acá vive el
// menú —grilla y barra, que son puro dibujo y punteros— y los marcos de las
// ventanas se quedan del otro lado.
//
// props:
//   { visible, bookmarks: [{ name }], windows: [{ dim }] }
// eventos:
//   open   { index }   — un marcador de la grilla
//   menu               — el botón de inicio de la barra
//   window { index }   — una ventana abierta de la barra
// ─────────────────────────────────────────────────────────────────────────────
// Los dos scripts llegan asincrónicos y el motor no garantiza el orden, así que
// se espera a que el vocabulario de dibujo exista antes de tocar nada.
(function arrancar(intentos) {
  if (!globalThis.ShellDraw || typeof globalThis.component === 'undefined') {
    if (intentos > 200) { console.warn('[shell_menu] no llegó ShellDraw o el canal'); return; }
    setTimeout(function () { arrancar(intentos + 1); }, 16);
    return;
  }
  armar();
})(0);

function armar() {
  const root = hiperspace.dimention;
  const D = globalThis.ShellDraw.init(root);
  const M = D.M;

  const panel = root.getElementById('menu_panel');
  const bottomBar = root.getElementById('menu_bar');
  const panelBgNode = root.getElementById('menu_panel_bg');
  const barBgNode = root.getElementById('menu_bar_bg');

  const panelMesh = {};
  const barMesh = {};

  // ── La grilla ──────────────────────────────────────────────────────────────
  let itemRefs = [];
  let dashboardRefs = [];

  function clearBookmarks() {
    for (const ref of itemRefs) D.dropPiece(ref.piece);
    for (const node of dashboardRefs) { try { node.remove(); } catch (_) {} }
    dashboardRefs = [];
    itemRefs = [];
  }

  // El ancho del panel sale de la grilla y no al revés; al revés obliga a
  // retocar dos números cada vez que se agrega un marcador.
  function gridSlot(i, count) {
    const rows = Math.max(1, Math.ceil(count / M.GRID_COLS));
    const row = Math.floor(i / M.GRID_COLS);
    const inRow = Math.min(M.GRID_COLS, count - row * M.GRID_COLS);
    return {
      u: (i % M.GRID_COLS - (inRow - 1) / 2) * M.GRID_STEP_X,
      y: M.GRID_Y + ((rows - 1) / 2 - row) * M.GRID_STEP_Y,
      rows: rows,
    };
  }

  function buildBookmarks(bookmarks) {
    clearBookmarks();
    if (!panel) return;

    const n = bookmarks.length;
    const rows = Math.max(1, Math.ceil(n / M.GRID_COLS));
    const half = (rows - 1) / 2 * M.GRID_STEP_Y;
    const gridTop = M.GRID_Y + half + M.SIZE_APP / 2;
    const gridBottom = M.GRID_Y - half + M.LABEL_DY - M.LABEL_SIZE;
    const headerY = gridTop + M.HEADER_DY;

    const header = D.surface(panel, 0, headerY, M.R_PANEL);
    dashboardRefs.push(header);
    D.caption(header, 'Explorar', 0.020, 0.040);
    D.caption(header, 'Mundos y aplicaciones', -0.036, 0.023, M.TENUE);

    for (let i = 0; i < n; i++) {
      const bm = bookmarks[i];
      const slot = gridSlot(i, n);
      const tile = D.surface(panel, slot.u, slot.y, M.R_PANEL);
      dashboardRefs.push(tile);
      const it = D.piece(tile, {
        x: 0, y: 0, size: M.SIZE_APP, corner: M.CORNER,
        color: D.PALETTE[i % D.PALETTE.length],
        glyph: D.GLYPHS[i] || 'app', padding: 0.24,
        name: 'open-' + (bm.name || i),
        // Lo único que sale de acá es «tocaron el índice i». Qué URL es eso, y
        // si abre una pestaña o una app embebida, lo sabe el controller.
        onTap: (function (idx) { return function () { emit('open', { index: idx }); }; })(i),
      });
      const text = D.caption(tile, bm.name || '', M.LABEL_DY, M.LABEL_SIZE);
      itemRefs.push({ group: tile, piece: it, text: text, idx: i });
    }

    const top = headerY + 0.075;
    const bottom = gridBottom - 0.045;
    D.paintMesh(panelMesh, panelBgNode, [{
      u: 0, y: (top + bottom) / 2, radius: M.R_PANEL, depth: 0.012,
      width: (M.GRID_COLS - 1) * M.GRID_STEP_X + M.SIZE_APP + 0.16,
      height: top - bottom, color: M.PLACA, phi: 0,
    }]);
  }

  // ── La cascada de entrada y de salida ──────────────────────────────────────
  const ANIM_MS = 160;
  const STAGGER_MS = 12;
  let animStartTs = 0;
  let animActive = false;
  let animMode = null;

  function applyScale(ref, factor) {
    const f = Math.max(0.001, factor);
    ref.group.scale = { x: f, y: f, z: 1 };
    // Escalar mueve el hitbox, así que mientras dura la cascada no puede
    // recibir nada: el objetivo estaría en otro lado que el dibujo.
    ref.piece.hit.setAttribute('touchable', factor > 0.95 ? 'true' : 'false');
  }

  function startEnterAnimation() {
    for (const ref of itemRefs) applyScale(ref, 0);
    animStartTs = -1; animMode = 'enter'; animActive = true;
    requestAnimationFrame(tickAnim);
  }

  function startExitAnimation() {
    animStartTs = -1; animMode = 'exit'; animActive = true;
    requestAnimationFrame(tickAnim);
  }

  function tickAnim(ts) {
    if (!animActive) return;
    if (animStartTs < 0) animStartTs = ts;
    const elapsed = ts - animStartTs;
    const n = itemRefs.length;
    let running = false;
    for (const ref of itemRefs) {
      if (!ref.group) continue;
      const order = animMode === 'exit' ? (n - 1 - ref.idx) : ref.idx;
      const own = elapsed - order * STAGGER_MS;
      if (own < 0) { applyScale(ref, animMode === 'enter' ? 0 : 1); running = true; continue; }
      if (own >= ANIM_MS) { applyScale(ref, animMode === 'enter' ? 1 : 0); continue; }
      const t = own / ANIM_MS;
      applyScale(ref, animMode === 'enter' ? D.easeOutBack(t) : (1 - D.easeInBack(t)));
      running = true;
    }
    if (running) { requestAnimationFrame(tickAnim); return; }
    animActive = false;
    // Recién al terminar la salida se apaga el panel: apagarlo antes se comería
    // la animación que justo se está mirando.
    if (animMode === 'exit') {
      if (panel) panel.setAttribute('visible', 'false');
      // Y recién ahora el shell puede mostrar lo que venga detrás. El menú
      // es el único que sabe cuándo terminó su cascada: son ~250 ms que
      // dependen de cuántos marcadores haya, y hacer que el otro lado los
      // adivine con un número fijo se desincroniza al primer cambio.
      emit('hidden', {});
    }
    animMode = null;
  }

  // ── La barra ───────────────────────────────────────────────────────────────
  // Tres pastillas: estado, avisos, y el inicio con las ventanas abiertas. Las
  // dos primeras son fijas; la tercera crece con lo que haya abierto.
  let barPieces = [];
  let barNodes = [];
  let barSignature = null;

  function clearBar() {
    for (const it of barPieces) D.dropPiece(it);
    for (const node of barNodes) { try { node.remove(); } catch (_) {} }
    barPieces = [];
    barNodes = [];
  }

  function spread(count, step) {
    const out = [];
    for (let i = 0; i < count; i++) out.push((i - (count - 1) / 2) * step);
    return out;
  }

  function barButton(parent, u, opts) {
    const group = D.surface(parent, u, M.BAR_Y, M.R_BAR, M.BAR_TILT);
    barNodes.push(group);
    if (opts.glyph && !opts.onTap) {
      // Indicador de estado: no se toca, así que no es una pieza. Un anillo que
      // se enciende en algo que no responde miente.
      const node = D.plane(group, {
        x: 0, y: 0, z: M.Z_FACE, sx: M.BAR_ITEM, sy: M.BAR_ITEM,
        color: opts.color || M.PLACA_ALTA, corner: 0.5,
      });
      D.glyph(node, opts.glyph, 0.28);
      return null;
    }
    const it = D.piece(group, {
      x: 0, y: 0, size: M.BAR_ITEM, corner: 0.5,
      color: opts.color || M.PLACA_ALTA,
      glyph: opts.glyph, padding: 0.26,
      name: opts.name, onTap: opts.onTap,
    });
    barPieces.push(it);
    return it;
  }

  function rebuildBar(windows) {
    // Sólo se redibuja si la composición cambió: las props llegan en cada
    // transición de estado, y rehacer los nodos en cada una hacía parpadear la
    // barra entera.
    const signature = windows.map(w => (w.dim ? 'm' : 'v')).join('|');
    if (signature === barSignature) return;
    barSignature = signature;
    clearBar();

    const g1 = ['user', 'wifi', 'battery'];
    const g2 = ['bell', 'capture'];
    const g3n = 1 + windows.length;
    const counts = [g1.length, g2.length, g3n];
    const widths = counts.map(n => n * M.BAR_ITEM + (n - 1) * M.BAR_GAP + M.BAR_PAD);

    const totalW = widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * M.BAR_GROUP_GAP;
    const centers = [];
    let cursor = -totalW / 2;
    for (const w of widths) { centers.push(cursor + w / 2); cursor += w + M.BAR_GROUP_GAP; }

    // `phi` es el ángulo con que la malla levanta la pastilla — el opuesto del
    // rx de los nodos, que se inclinan hacia arriba porque están bajo los ojos.
    D.paintMesh(barMesh, barBgNode, widths.map((w, i) => ({
      u: centers[i], y: M.BAR_Y, radius: M.R_BAR, depth: 0.012, width: w,
      height: M.BAR_H, color: M.PLACA, phi: -M.BAR_TILT,
    })));

    const place = (gi, j, n) => centers[gi] + spread(n, M.BAR_ITEM + M.BAR_GAP)[j];

    for (let j = 0; j < g1.length; j++) {
      barButton(bottomBar, place(0, j, g1.length), { glyph: g1[j], color: M.PLACA_ALTA });
    }
    for (let j = 0; j < g2.length; j++) {
      barButton(bottomBar, place(1, j, g2.length), {
        glyph: g2[j], color: M.PLACA_ALTA, onTap: function () {},
      });
    }

    let j = 0;
    barButton(bottomBar, place(2, j++, g3n), {
      glyph: 'home', color: '#30435F', name: 'bar-menu',
      onTap: function () { emit('menu', {}); },
    });
    for (let i = 0; i < windows.length; i++) {
      const u = place(2, j++, g3n);
      barButton(bottomBar, u, {
        glyph: 'app', color: windows[i].dim ? M.PLACA_ALTA : '#304F68',
        name: 'bar-win-' + i,
        onTap: (function (idx) { return function () { emit('window', { index: idx }); }; })(i),
      });
      // Cerrar: un punto chico arriba a la derecha del botón. Tiene que ser
      // su propia pieza —hitbox aparte— o cerrar y traer al frente serían el
      // mismo toque.
      const cierre = D.surface(bottomBar, u + M.BAR_ITEM * 0.42,
                               M.BAR_Y + M.BAR_ITEM * 0.42, M.R_BAR, M.BAR_TILT);
      barNodes.push(cierre);
      barPieces.push(D.piece(cierre, {
        x: 0, y: 0, size: 0.030, corner: 0.5, color: '#B0413E',
        // Adelantado sobre el botón: a la misma profundidad las dos caras
        // pelean por el mismo píxel y el punto parpadea.
        zOffset: 0.006,
        name: 'bar-close-' + i,
        onTap: (function (idx) { return function () { emit('close', { index: idx }); }; })(i),
      }));
    }
  }

  // ── El canal con el controller ─────────────────────────────────────────────
  // `emit` es asincrónico y el evento le llega al padre como `component:<tipo>`
  // sobre el nodo del include. Los tipos tienen que estar declarados en el
  // atributo `events` o el motor los rechaza.
  function emit(type, detail) {
    try { component.emit(type, detail || {}); }
    catch (e) { console.warn('[shell_menu] no pude emitir ' + type + ': ' + e); }
  }

  // Qué hay dibujado, para no rehacerlo cuando las props cambian por otra cosa.
  let dibujadas = null;
  let visible = null;

  function render(props) {
    const bookmarks = props.bookmarks || [];
    const windows = props.windows || [];

    // La grilla se rehace sólo si cambiaron los marcadores: es lo caro —una
    // pieza son tres nodos— y lo que más se repite es un cambio de ventanas.
    const firma = JSON.stringify(bookmarks.map(b => b.name));
    if (firma !== dibujadas) {
      dibujadas = firma;
      buildBookmarks(bookmarks);
      barSignature = null;   // la barra se apoya en las mismas piezas
    }
    rebuildBar(windows);

    // El panel y la barra no siguen la misma regla: el panel se va cuando una
    // app toma el frente, la barra se queda mientras dure la sesión del shell,
    // que es lo único que permite volver al menú desde adentro de una app.
    const quiere = !!props.panelVisible;
    if (quiere !== visible) {
      const primera = visible === null;
      visible = quiere;
      if (quiere) {
        if (panel) panel.setAttribute('visible', 'inherit');
        if (!primera) startEnterAnimation();
      } else if (primera) {
        if (panel) panel.setAttribute('visible', 'false');
      } else {
        // El panel se apaga al terminar la cascada, no ahora.
        startExitAnimation();
      }
    }
    if (bottomBar) bottomBar.setAttribute('visible', props.barVisible ? 'inherit' : 'false');
  }

  component.addEventListener('propschange', function (e) { render(e.detail.props); });
  component.addEventListener('connect', function () { render(component.props); });
  render(component.props);

  console.log('[shell_menu] listo');
}
