// ─────────────────────────────────────────────────────────────────────────────
// shell_section_blank.js — una sección vacía: el fondo y su título.
//
// Lo comparten las secciones que todavía no tienen contenido —notificaciones,
// mundo, cuenta—. Cada una es su propio documento para poder crecer por
// separado; lo único que las distingue hoy es el atributo `title` de su
// `<space>`, que este script lee.
//
// El fondo **no se declara en el HSML** aunque sería más corto: sus medidas
// salen de `ShellUI.M`, las mismas que usa la grilla. Escritas a mano en cada
// archivo, la primera vez que alguien toque la escala del menú estas tres
// secciones se quedarían del tamaño viejo y nadie se enteraría hasta verlas.
//
// props:
//   { visible }
// eventos:
//   hidden   — se emite al ocultarse. Acá es inmediato, sin cascada, pero el
//              aviso tiene que salir igual: el shell lo espera para revelar la
//              ventana de una app.
// ─────────────────────────────────────────────────────────────────────────────
(function arrancar(intentos) {
  if (!globalThis.ShellUI || typeof globalThis.component === 'undefined') {
    if (intentos > 200) { console.warn('[seccion] no llegó ShellUI o el canal'); return; }
    setTimeout(function () { arrancar(intentos + 1); }, 16);
    return;
  }
  armar();
})(0);

function armar() {
  const UI = globalThis.ShellUI;
  const M = UI.M;
  const root = hiperspace.dimention;
  const panel = root.getElementById('seccion');
  const titulo = root.getAttribute('title') || '';

  function emit(type, detail) {
    try { component.emit(type, detail || {}); }
    catch (e) { console.warn('[seccion] no pude emitir ' + type + ': ' + e); }
  }

  // Las mismas medidas que la grilla, para que el fondo no salte de tamaño al
  // cambiar de sección: cinco columnas de ancho y dos filas de alto.
  const cols = M.cols;
  const rows = 2;
  const half = (rows - 1) / 2 * M.stepY;
  const top = M.stepY * (rows - 1) / 2 + M.tile / 2 + M.panelPad;
  const bottom = -half + M.labelDy - M.labelSize - M.panelPad;
  const w = (cols - 1) * M.stepX + M.tile + 2 * M.panelPad;
  const cy = (top + bottom) / 2;

  UI.plane(panel, {
    x: 0, y: cy, z: M.zBack, w: w, h: top - bottom,
    color: UI.C.panel, corner: M.panelCorner, blocking: true,
  });

  const hy = top + M.headerGap + M.headerH / 2;
  UI.pill(panel, { x: 0, y: hy, z: M.zBack, w: M.headerW, h: M.headerH,
                   r: M.headerRadius, color: UI.C.pill, blocking: true });
  UI.text(panel, { x: 0, y: hy, value: titulo, size: M.headerSize });

  let visible = null;
  function render(props) {
    const quiere = !!props.visible;
    if (quiere === visible) return;
    visible = quiere;
    if (panel) panel.setAttribute('visible', quiere ? 'inherit' : 'false');
    if (!quiere) emit('hidden', {});
  }

  component.addEventListener('propschange', function (e) { render(e.detail.props); });
  component.addEventListener('connect', function () { render(component.props); });
  render(component.props);

  console.log('[seccion] ' + titulo + ' lista');
}
