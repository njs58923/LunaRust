// ─────────────────────────────────────────────────────────────────────────────
// shell_menu_flat.js — el menú plano del shell, como componente.
//
// El gemelo de `shell_menu.js`: **el mismo contrato, otro dibujo**. Uno apoya
// las piezas en un cilindro alrededor del visitante y arma sus fondos con una
// malla que sigue el arco; éste es plano y usa cajas con esquinas redondeadas.
// Curvo se lee bien con la cabeza adentro; plano se lee bien en una pantalla.
//
// Que compartan contrato es lo que hace que `shell_app.js` no sepa cuál de los
// dos está montado. El controller elige el include y nada más cambia.
//
// props:
//   { visible, bookmarks: [{ name }], windows: [{ dim }] }
// eventos:
//   open   { index }   — un marcador de la grilla
//   menu               — el botón de aplicaciones de la barra
//   window { index }   — una ventana abierta de la barra
//   hidden             — terminó de irse; el shell espera esto para revelar
//                        la ventana que venga detrás
// ─────────────────────────────────────────────────────────────────────────────
(function arrancar(intentos) {
  if (!globalThis.ShellUI || typeof globalThis.component === 'undefined') {
    if (intentos > 200) { console.warn('[shell_menu_flat] no llegó ShellUI o el canal'); return; }
    setTimeout(function () { arrancar(intentos + 1); }, 16);
    return;
  }
  armar();
})(0);

function armar() {
  const UI = globalThis.ShellUI;
  const root = hiperspace.dimention;
  const panel = root.getElementById('menu_panel');
  const barZone = root.getElementById('menu_bar');

  // ── El canal con el controller ───────────────────────────────────────────
  function emit(type, detail) {
    try { component.emit(type, detail || {}); }
    catch (e) { console.warn('[shell_menu_flat] no pude emitir ' + type + ': ' + e); }
  }

  // ── Lo dibujado ──────────────────────────────────────────────────────────
  let board = null;
  let bar = null;
  let dibujadas = null;    // firma de los marcadores que están puestos
  let barSignature = null; // firma de la barra que está puesta
  let visible = null;

  function buildBoard(bookmarks) {
    // Se rehace sólo cuando cambian los marcadores, no en cada cambio de
    // ventanas: son tres nodos por icono y lo que más se repite es abrir o
    // cerrar algo.
    //
    // Y **se rehace de verdad**. Acá había un `if (board) return` que parecía
    // una optimización y era un bug: la primera tanda de props llega vacía
    // —el include monta antes de que el shell le mande el estado— así que el
    // tablero se construía con cero marcadores y esa guarda impedía volver a
    // armarlo cuando llegaban los nueve. El panel quedaba vacío para siempre.
    if (board) { board.destroy(); board = null; }
    if (bar) { bar.destroy(); bar = null; }
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
      // si abre una pestaña o una app embebida, lo sabe el controller.
      onOpen: function (app) { emit('open', { index: app.__idx }); },
    });

    // La barra cuelga del borde de abajo del tablero, no de una altura fija:
    // así sigue al panel cuando cambia la escala o la cantidad de filas.
    const barY = board.centerY - board.height / 2 - UI.M.barH / 2 - 0.06 * (UI.M.escala || 1);
    bar = UI.bar(barZone, { y: barY });
  }

  function rebuildBar(windows) {
    if (!bar) return;
    // Sólo se redibuja si la composición cambió: las props llegan en cada
    // transición de estado, y rehacer los nodos en cada una hacía parpadear la
    // barra entera.
    const signature = windows.map(function (w) { return w.dim ? 'm' : 'v'; }).join('|');
    if (signature === barSignature) return;
    barSignature = signature;

    const estado = [
      { glyph: 'user', color: UI.C.blue, onTap: function () {} },
      { glyph: 'home', color: UI.C.blue, onTap: function () {} },
      { glyph: 'bell', color: UI.C.blue, onTap: function () {} },
    ];
    // El botón de apps es la puerta al menú; los que le siguen, las ventanas
    // vivas. Por eso comparten pastilla y el estado del sistema no.
    const apps = [{
      glyph: 'apps', color: UI.C.blue, selected: true,
      onTap: function () { emit('menu', {}); },
    }];
    for (let i = 0; i < windows.length; i++) {
      apps.push({
        glyph: 'app',
        color: windows[i].dim ? UI.C.pill : UI.C.blue,
        name: 'bar-win-' + i,
        onTap: (function (idx) { return function () { emit('window', { index: idx }); }; })(i),
      });
    }
    bar.render([estado, apps]);
  }

  // ── La cascada de entrada y de salida ────────────────────────────────────
  // Misma lógica que la del menú curvo, y por la misma razón: es lo único que
  // separa «el menú apareció» de «el menú estaba y no lo veías». Y al terminar
  // la salida hay que avisar, porque el shell espera ese aviso para revelar la
  // ventana que venga detrás.
  const ANIM_MS = 160;
  const STAGGER_MS = 12;
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
    if (!board) return;
    for (const t of board.tiles) applyScale(t, 0);
    animStartTs = -1; animMode = 'enter'; animActive = true;
    requestAnimationFrame(tickAnim);
  }

  function startExitAnimation() {
    // Sin tablero no hay cascada, pero el aviso tiene que salir igual: el
    // shell lo está esperando para revelar la ventana.
    if (!board) { if (panel) panel.setAttribute('visible', 'false'); emit('hidden', {}); return; }
    animStartTs = -1; animMode = 'exit'; animActive = true;
    requestAnimationFrame(tickAnim);
  }

  function tickAnim(ts) {
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
    if (running) { requestAnimationFrame(tickAnim); return; }
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

  // ── Render ───────────────────────────────────────────────────────────────
  function render(props) {
    const bookmarks = (props.bookmarks || []).map(function (b, i) {
      // El índice viaja con el marcador: es lo que el evento `open` devuelve, y
      // el paginador reordena las celdas sin que el índice deje de valer.
      //
      // El glifo sale del índice si el marcador no trae uno, igual que en el
      // menú curvo: las props llevan sólo el nombre, y sin este respaldo los
      // nueve iconos salían con el mismo dibujo.
      return { name: b.name, glyph: b.glyph || UI.GLYPHS[i], __idx: i };
    });
    const windows = props.windows || [];

    const firma = JSON.stringify(bookmarks.map(function (b) { return b.name; }));
    if (firma !== dibujadas) {
      dibujadas = firma;
      buildBoard(bookmarks);
      barSignature = null;   // la barra se apoya en las mismas medidas
    }
    rebuildBar(windows);

    const quiere = !!props.visible;
    if (quiere !== visible) {
      const primera = visible === null;
      visible = quiere;
      if (quiere) {
        if (panel) panel.setAttribute('visible', 'true');
        if (!primera) startEnterAnimation();
      } else if (primera) {
        if (panel) panel.setAttribute('visible', 'false');
      } else {
        startExitAnimation();   // el panel se apaga al terminar, no ahora
      }
    }
    if (barZone) barZone.setAttribute('visible', quiere ? 'true' : 'false');
  }

  component.addEventListener('propschange', function (e) { render(e.detail.props); });
  component.addEventListener('connect', function () { render(component.props); });
  render(component.props);

  console.log('[shell_menu_flat] listo');
}
