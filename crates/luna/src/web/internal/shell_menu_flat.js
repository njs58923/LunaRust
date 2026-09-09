// ─────────────────────────────────────────────────────────────────────────────
// shell_menu_flat.js — el menú plano del shell.
//
// Ya no dibuja el panel: dibuja **la barra** y hospeda una **sección**, que es
// otro `<include>` un nivel más adentro. Cambiar de sección es cambiarle el
// `src` a ese include, así que sólo está cargada la que se ve — la que no se
// mira ni siquiera tiene isolate.
//
// Queda entonces una cadena de tres isolates, y cada eslabón habla sólo con sus
// vecinos:
//
//     controller  ──props──▶  menú  ──props──▶  sección
//                 ◀─eventos──       ◀─eventos──
//
// El menú es el del medio: traduce. Del controller recibe qué marcadores hay y
// si se ve; a la sección le pasa lo que necesite; y lo que la sección avisa
// —tocaron el marcador i, terminé de irme— lo reenvía hacia arriba tal cual. Qué
// URL es ese índice lo sigue sabiendo sólo el controller.
//
// props (del controller):
//   { panelVisible, barVisible, bookmarks: [{ name }], windows: [{ dim }] }
// eventos (al controller):
//   open { index } · menu · window { index } · close { index } · hidden
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
  const M = UI.M;
  const S = M.escala || 1;
  const root = hiperspace.dimention;
  const barZone = root.getElementById('menu_bar');
  const seccion = root.getElementById('menu_seccion');

  // La barra cuelga del borde de abajo del panel. Antes se calculaba con las
  // medidas que devolvía el tablero, pero el tablero ahora vive en otro isolate:
  // sale de las mismas constantes de las que el tablero las saca.
  const FILAS = 2;
  const PANEL_ABAJO = -(FILAS - 1) / 2 * M.stepY + M.labelDy - M.labelSize - M.panelPad;
  const BAR_Y = PANEL_ABAJO - M.barH / 2 - 0.06 * S;

  // ── Las secciones ────────────────────────────────────────────────────────
  // Cada una es un documento aparte. El botón de la barra que la abre está acá
  // al lado a propósito: es la única tabla que relaciona las dos cosas.
  const SECCIONES = [
    { id: 'apps',   url: 'luna://shell_apps',    glyph: 'apps' },
    { id: 'cuenta', url: 'luna://shell_account', glyph: 'user' },
    { id: 'mundo',  url: 'luna://shell_world',   glyph: 'home' },
    { id: 'avisos', url: 'luna://shell_notifs',  glyph: 'bell' },
  ];
  let activa = 'apps';

  // ── Props hacia la sección ───────────────────────────────────────────────
  let ultimasSectionProps = null;
  let quiereVisible = false;
  let ultimosBookmarks = [];
  let ultimasWindows = [];

  function pushSectionProps(forzar) {
    if (!seccion) return;
    const next = {
      visible: quiereVisible,
      // Sólo la sección de aplicaciones los usa; a las demás les llegan y los
      // ignoran. Mandarlos siempre es más simple que decidir acá qué necesita
      // cada una, y son nueve nombres.
      bookmarks: ultimosBookmarks,
    };
    const firma = JSON.stringify(next);
    // Se compara contra lo último enviado: las props del controller llegan en
    // cada transición de estado, y escribir props iguales despierta al isolate
    // de la sección para nada.
    if (!forzar && firma === ultimasSectionProps) return;
    ultimasSectionProps = firma;
    seccion.props = next;
  }

  function irA(id) {
    // Tocar una sección es pedir el menú. Si hay una app en primer plano el
    // panel no se está viendo —ocupa el mismo lugar que la ventana—, así que
    // cambiar de sección sin avisar dejaba la sección nueva escondida detrás de
    // la app: el botón respondía y no pasaba nada visible. `menu` es lo que el
    // shell entiende como «minimizá lo que haya y volvé al menú»; sin foco no
    // hace nada, por eso se manda siempre.
    //
    // Va **antes** del corte por sección repetida: tocar la que ya está activa
    // con una app adelante también tiene que traer el menú de vuelta.
    emit('menu', {});
    if (id === activa) return;
    const s = SECCIONES.find(function (x) { return x.id === id; });
    if (!s || !seccion) return;
    activa = id;
    // Cambiar el `src` reemplaza el documento, y el isolate de la sección
    // anterior se va con él. Por eso la nueva arranca sin saber nada y hay que
    // volver a mandarle las props aunque no hayan cambiado.
    seccion.setAttribute('src', s.url);
    pushSectionProps(true);
    barSignature = null;   // cambió cuál está activa
    rebuildBar(ultimasWindows);
  }

  // ── Lo que la sección avisa, se reenvía ──────────────────────────────────
  function emit(type, detail) {
    try { component.emit(type, detail || {}); }
    catch (e) { console.warn('[shell_menu_flat] no pude emitir ' + type + ': ' + e); }
  }

  if (seccion) {
    seccion.addEventListener('component:open', function (e) {
      emit('open', { index: e.detail && e.detail.index });
    });
    // El shell espera este aviso para revelar la ventana de una app, así que
    // tiene que llegarle desde dos isolates más abajo sin perderse.
    seccion.addEventListener('component:hidden', function () { emit('hidden', {}); });
  }

  // ── La barra ─────────────────────────────────────────────────────────────
  let bar = null;
  let barSignature = null;

  function rebuildBar(windows) {
    if (!barZone) return;
    if (!bar) bar = UI.bar(barZone, { y: BAR_Y });

    // Sólo se redibuja si cambió la composición o la sección activa: las props
    // llegan en cada transición y rehacer los nodos en cada una hacía parpadear
    // la barra entera.
    const signature = activa + '|' + windows.map(function (w) { return w.dim ? 'm' : 'v'; }).join(',');
    if (signature === barSignature) return;
    barSignature = signature;

    const boton = function (s) {
      return {
        glyph: s.glyph, color: UI.C.blue, selected: activa === s.id,
        name: 'bar-' + s.id,
        onTap: (function (id) { return function () { irA(id); }; })(s.id),
      };
    };

    // Izquierda, las secciones del sistema. Derecha, aplicaciones y las ventanas
    // vivas: el botón de apps es la puerta a su sección y los que le siguen son
    // lo que está abierto, por eso comparten pastilla y el resto no.
    const estado = [boton(SECCIONES[1]), boton(SECCIONES[2]), boton(SECCIONES[3])];
    const apps = [boton(SECCIONES[0])];
    for (let i = 0; i < windows.length; i++) {
      apps.push({
        glyph: 'app', color: UI.C.blue, selected: !windows[i].dim,
        name: 'bar-win-' + i,
        onTap: (function (idx) { return function () { emit('window', { index: idx }); }; })(i),
        onClose: (function (idx) { return function () { emit('close', { index: idx }); }; })(i),
      });
    }
    bar.render([estado, apps]);
  }

  // ── Render ───────────────────────────────────────────────────────────────
  function render(props) {
    ultimosBookmarks = (props.bookmarks || []).map(function (b) { return { name: b.name }; });
    ultimasWindows = props.windows || [];
    quiereVisible = !!props.panelVisible;

    pushSectionProps(false);
    rebuildBar(ultimasWindows);
    if (barZone) barZone.setAttribute('visible', props.barVisible ? 'true' : 'false');
  }

  component.addEventListener('propschange', function (e) { render(e.detail.props); });
  component.addEventListener('connect', function () { render(component.props); });
  render(component.props);

  console.log('[shell_menu_flat] listo — ' + SECCIONES.length + ' secciones');
}
