// ─────────────────────────────────────────────────────────────────────────────
// shell_section_apps.js — la sección de aplicaciones del menú.
//
// Corre dentro de su propio `<include>`, un nivel más adentro que el menú: el
// controller incluye al menú, y el menú incluye a la sección que esté activa.
// Cambiar de sección es cambiarle el `src` a ese include, así que **sólo está
// cargada la que se ve** — que es todo el punto de haberla separado.
//
// props:
//   { visible, bookmarks: [{ name }] }
// eventos:
//   open   { index }   — tocaron un marcador
//   hidden             — terminó la cascada de salida; el menú lo reenvía al
//                        shell, que lo espera para revelar la ventana de una app
// ─────────────────────────────────────────────────────────────────────────────
(function arrancar(intentos) {
  if (!globalThis.ShellUI || typeof globalThis.component === 'undefined') {
    if (intentos > 200) { console.warn('[apps] no llegó ShellUI o el canal'); return; }
    setTimeout(function () { arrancar(intentos + 1); }, 16);
    return;
  }
  armar();
})(0);

function armar() {
  const UI = globalThis.ShellUI;
  const root = hiperspace.dimention;
  const panel = root.getElementById('seccion');

  function emit(type, detail) {
    try { component.emit(type, detail || {}); }
    catch (e) { console.warn('[apps] no pude emitir ' + type + ': ' + e); }
  }

  let board = null;
  let dibujadas = null;
  let visible = null;

  function buildBoard(bookmarks) {
    // Se rehace de verdad cuando cambian los marcadores. La primera tanda de
    // props llega vacía —el include monta antes de que le manden el estado— así
    // que una guarda de «ya está armado» dejaría el panel vacío para siempre.
    if (board) { board.destroy(); board = null; }
    if (!bookmarks.length) return;
    board = UI.dashboard(panel, {
      title: 'Aplicaciones',
      apps: bookmarks,
      rail: [
        { glyph: 'search', onTap: function () {} },
        { glyph: 'add',    onTap: function () {} },
        { glyph: 'edit',   onTap: function () {} },
      ],
      // Lo único que sale de acá es «tocaron el marcador i». Qué URL es eso, y
      // si abre una pestaña o una app embebida, lo sabe el controller — dos
      // isolates más arriba.
      onOpen: function (app) { emit('open', { index: app.__idx }); },
    });
  }

  // ── La cascada ───────────────────────────────────────────────────────────
  // Vive acá porque acá están los iconos: es lo único que separa «el menú
  // apareció» de «el menú estaba y no lo veías». Y al terminar la salida hay que
  // avisar, porque el shell espera ese aviso para revelar la ventana que venga
  // detrás.
  const ANIM_MS = 160;
  const STAGGER_MS = 12;
  let animStartTs = 0;
  let animActive = false;
  let animMode = null;

  function applyScale(tile, factor) {
    const f = Math.max(0.001, factor);
    tile.group.scale = { x: f, y: f, z: 1 };
    // Escalar mueve el hitbox, así que mientras dura la cascada no puede recibir
    // nada: el objetivo estaría en otro lado que el dibujo.
    tile.piece.hit.setAttribute('touchable', factor > 0.95 ? 'true' : 'false');
  }

  function startEnter() {
    if (!board) return;
    for (const t of board.tiles) applyScale(t, 0);
    animStartTs = -1; animMode = 'enter'; animActive = true;
    requestAnimationFrame(tick);
  }

  function startExit() {
    // Sin tablero no hay cascada, pero el aviso tiene que salir igual: hay
    // alguien esperándolo.
    if (!board) { if (panel) panel.setAttribute('visible', 'false'); emit('hidden', {}); return; }
    animStartTs = -1; animMode = 'exit'; animActive = true;
    requestAnimationFrame(tick);
  }

  function tick(ts) {
    if (!animActive || !board) return;
    if (animStartTs < 0) animStartTs = ts;
    const elapsed = ts - animStartTs;
    const n = board.tiles.length;
    let running = false;
    for (let i = 0; i < n; i++) {
      const tile = board.tiles[i];
      if (!tile.group) continue;
      // Al salir la cascada va al revés: entra por el primero y se va por el
      // último, que es como se lee un barrido.
      const order = animMode === 'exit' ? (n - 1 - i) : i;
      const own = elapsed - order * STAGGER_MS;
      if (own < 0) { applyScale(tile, animMode === 'enter' ? 0 : 1); running = true; continue; }
      if (own >= ANIM_MS) { applyScale(tile, animMode === 'enter' ? 1 : 0); continue; }
      const t = own / ANIM_MS;
      applyScale(tile, animMode === 'enter' ? easeOutBack(t) : (1 - easeInBack(t)));
      running = true;
    }
    if (running) { requestAnimationFrame(tick); return; }
    animActive = false;
    if (animMode === 'exit') {
      // Recién al terminar se apaga el panel: apagarlo antes se comería la
      // animación que justo se está mirando.
      if (panel) panel.setAttribute('visible', 'false');
      emit('hidden', {});
    }
    animMode = null;
  }

  function easeOutBack(t) {
    const c1 = 1.70158, c3 = c1 + 1;
    return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
  }
  function easeInBack(t) {
    const c1 = 1.70158, c3 = c1 + 1;
    return c3 * t * t * t - c1 * t * t;
  }

  function render(props) {
    const bookmarks = (props.bookmarks || []).map(function (b, i) {
      // El índice viaja con el marcador: es lo que devuelve `open`, y el
      // paginador reordena las celdas sin que deje de valer. El glifo sale del
      // índice si el marcador no trae uno.
      return { name: b.name, glyph: b.glyph || UI.GLYPHS[i], __idx: i };
    });

    const firma = JSON.stringify(bookmarks.map(function (b) { return b.name; }));
    if (firma !== dibujadas) {
      dibujadas = firma;
      buildBoard(bookmarks);
    }

    const quiere = !!props.visible;
    if (quiere !== visible) {
      const primera = visible === null;
      visible = quiere;
      if (quiere) {
        if (panel) panel.setAttribute('visible', 'inherit');
        if (!primera) startEnter();
      } else if (primera) {
        if (panel) panel.setAttribute('visible', 'false');
      } else {
        startExit();   // el panel se apaga al terminar, no ahora
      }
    }
  }

  component.addEventListener('propschange', function (e) { render(e.detail.props); });
  component.addEventListener('connect', function () { render(component.props); });
  render(component.props);

  console.log('[apps] listo');
}
