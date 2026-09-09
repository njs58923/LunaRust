// ─────────────────────────────────────────────────────────────────────────────
// shell_ui.js — la interfaz del shell de Luna, una sola vez.
//
// La cargan **los dos controllers**, `ux_vr` y `ux_desktop`, con
// `<script src="luna://internal/shell_ui.js"/>`. Va como script y no como
// `<include>` a propósito: un include es otro documento, con su propio isolate
// y sus propios permisos, y el menú no podría hablarle a `dimention.tabs` ni
// mandarle el slot a una app. Un script cargado en el espacio comparte isolate,
// así que el controller le pasa callbacks y listo.
//
// **Todo es plano.** La versión anterior apoyaba cada pieza en un cilindro
// alrededor del visitante y armaba los fondos con una malla que seguía el arco.
// Se leía bien en VR y no se podía reusar en escritorio, que es una pantalla.
// Plano se ve igual en los dos, y de paso se va la malla dinámica —con su tope
// de 128 recursos por sesión que no se liberan al cambiar de espacio.
//
// Nada de acá toca el host: se dibuja, escucha punteros y avisa por callback.
// Quién abre una pestaña, quién manda un slot y quién decide qué se ve es el
// controller.
// ─────────────────────────────────────────────────────────────────────────────
(function () {
  const root = hiperspace.dimention;

  // ── Medidas ────────────────────────────────────────────────────────────────
  // Metros. El origen de cada pieza es el centro de su grupo, y el eje +z apunta
  // al visitante: acercar algo es sumarle z.
  //
  // **Un solo número gobierna el tamaño de todo.** Los iconos viven en tres
  // escalas —la grilla, la barra, el riel— y cada una arrastra sus pasos, sus
  // huecos y sus etiquetas: agrandar un icono sin agrandar el paso los hace
  // pisarse, y agrandarlo sin la etiqueta deja un texto de juguete al lado. Por
  // eso la tabla de abajo son las medidas **base** y `ESCALA` las multiplica a
  // todas de una vez, en el bucle que sigue al objeto.
  //
  // Lo que NO se escala son las fracciones —los `border-radius` de un `plane`
  // son proporción del lado y ya siguen a la pieza— ni las capas en z, que no
  // son tamaño sino orden de dibujo.
  const ESCALA = 2;

  const M = {
    tile: 0.17,
    tileCorner: 0.16,      // FRACCIÓN del lado, no metros (ver `plane`)
    stepX: 0.23,
    stepY: 0.26,
    cols: 5,
    rows: 3,
    labelDy: -0.115,
    labelSize: 0.026,

    panelPad: 0.055,
    panelCorner: 0.035,

    headerH: 0.062,
    headerW: 0.30,
    headerGap: 0.030,
    headerSize: 0.029,
    headerRadius: 0.021,   // METROS

    railSize: 0.052,
    railGap: 0.014,
    railOffset: 0.042,     // del filo del panel al riel
    railCorner: 0.28,

    dotR: 0.009,
    dotGap: 0.026,         // centro a centro entre puntos
    navGap: 0.022,         // entre un botón de página y los puntos

    barY: -0.62,
    barH: 0.105,
    barItem: 0.078,
    barItemCorner: 0.30,
    barGap: 0.014,
    barPad: 0.020,
    barPillGap: 0.018,
    barPillRadius: 0.036,  // METROS: una pastilla no es cuadrada (ver `pill`)

    winCorner: 0.030,
    winTitleH: 0.044,
    winTitleRadius: 0.019, // METROS
    winTitleGap: 0.026,
    winBtn: 0.032,
    winBtnGap: 0.010,
    winTitleSize: 0.024,

    // Capas en z dentro de un grupo. Sin esto el motor decide el orden solo y
    // las etiquetas parpadean contra su propio fondo.
    zBack: -0.004,
    zHit: -0.0012,
    zRing: -0.0008,
    zFace: 0,
    zGlyph: 0.0025,
    zText: 0.005,
    ring: 0.0022,          // grosor del anillo de hover

    // Aire entre el glifo y el borde de su placa, como fracción del lado.
    // **No pasa por ESCALA**: es una proporción, no una longitud, y ahí estaba
    // el malentendido — al duplicar las placas los iconos se veían igual porque
    // crecían junto con su marco. Para agrandar el dibujo hay que agrandar su
    // rect adentro, o sea bajar esto.
    //
    // El atlas ya trae su propio aire: cada icono se ajusta a 72 px dentro de
    // una celda de 96, así que el glifo visible ocupa el 75% de lo que se le
    // reserve. Con 0,10 termina cerca del 60% del lado de la placa.
    padTile: 0.10,
    padBar: 0.13,
    padRail: 0.14,
    padWin: 0.18,
  };

  // Las claves que son una longitud. Enumerarlas en vez de escalar el objeto
  // entero es a propósito: así una medida nueva no se escala sola por olvido, y
  // agregar una obliga a decidir de qué lado está.
  const ESCALABLES = [
    'tile', 'stepX', 'stepY', 'labelDy', 'labelSize',
    'panelPad',
    'headerH', 'headerW', 'headerGap', 'headerSize', 'headerRadius',
    'railSize', 'railGap', 'railOffset',
    'dotR', 'dotGap', 'navGap',
    'barY', 'barH', 'barItem', 'barGap', 'barPad', 'barPillGap', 'barPillRadius',
    'winTitleH', 'winTitleRadius', 'winTitleGap', 'winBtn', 'winBtnGap',
    'winTitleSize', 'ring',
  ];
  for (const k of ESCALABLES) M[k] *= ESCALA;
  M.escala = ESCALA;

  const C = {
    panel: '#2A2C31',
    pill: '#232529',
    surface: '#2F3238',
    blue: '#4A80E8',
    blueOn: '#5C8DEB',
    red: '#E5484D',
    text: '#FFFFFF',
    label: '#E6E8EC',
    dim: '#6B7280',
    white: '#FFFFFF',
    viewport: '#3A3C42',
  };

  // ── Glifos ─────────────────────────────────────────────────────────────────
  // Atlas de 8x4 embebido en el binario. El slot de cada icono es su índice en
  // `tools/build_icon_atlas.py`; esta tabla es la otra mitad del contrato, así
  // que **los nombres nuevos van al final en los dos lados**. Intercalar uno le
  // cambia el dibujo a todo lo que venga después.
  const ATLAS = 'luna://icons/menu.png';
  const GLYPHS = ['home', 'settings', 'demos', 'app', 'about', 'scale', 'fire',
                  'target', 'range', 'user', 'wifi', 'battery', 'bell',
                  'capture', 'close', 'minimize', 'anchor', 'search', 'add',
                  'edit', 'apps', 'chevron'];

  // ── Nodos ──────────────────────────────────────────────────────────────────
  function group(parent, x, y, z) {
    const g = root.createElement('group');
    g.position = { x: x || 0, y: y || 0, z: z || 0 };
    parent.appendChild(g);
    return g;
  }

  // `border-radius` en un `plane` es una **fracción del lado**: el motor se lo
  // pasa crudo a `create_rounded_plane`, que arma el contorno sobre un cuadrado
  // unitario. En un `box`, en cambio, son metros del mundo divididos por la
  // escala de cada eje, y se topean en 0,499 — por eso un icono es un plane y
  // no una caja: con lado 0,17 y radio 0,2 la caja satura los tres ejes y sale
  // esfera. Media fracción, 0,5, es un círculo.
  function plane(parent, o) {
    const n = root.createElement('plane');
    n.setAttribute('color', o.color);
    n.setAttribute('border-radius', String(o.corner === undefined ? 0 : o.corner));
    n.setAttribute('touchable', o.touchable ? 'true' : 'false');
    n.position = { x: o.x || 0, y: o.y || 0, z: o.z || 0 };
    n.scale = { x: o.w, y: o.h, z: 1 };
    parent.appendChild(n);
    return n;
  }

  // Una pastilla: un fondo **más ancho que alto** con esquinas redondeadas.
  //
  // Va como `box` y no como `plane` por una diferencia del motor que se ve a
  // simple vista: en un `plane` el `border-radius` se pasa crudo a
  // `create_rounded_plane`, que trabaja sobre un cuadrado unitario, así que es
  // una **fracción del lado** y al escalar 0,37 x 0,10 las esquinas se estiran
  // con él — una pastilla queda convertida en un óvalo. En un `box`,
  // `rounded_box_radii` divide el radio **en metros** por la escala de cada eje
  // (y lo topea en 0,499), o sea que conserva el radio del mundo: esquinas
  // circulares de verdad en un rectángulo.
  //
  // El espesor es el precio. Con una caja fina el radio satura en z y el canto
  // se redondea entero, pero eso no toca la silueta de frente, que es lo único
  // que se mira. Lo que sí hay que corregir es la profundidad: una caja crece
  // hacia los dos lados, así que se la corre media altura para que su **cara**
  // quede en la z pedida y no se adelante sobre lo que tiene encima.
  const PILL_DEPTH = 0.020 * M.escala;
  function pill(parent, o) {
    const n = root.createElement('box');
    n.setAttribute('color', o.color);
    n.setAttribute('border-radius', String(o.r));
    n.setAttribute('touchable', 'false');
    const d = o.d || PILL_DEPTH;
    n.position = { x: o.x || 0, y: o.y || 0, z: (o.z || 0) - d / 2 };
    n.scale = { x: o.w, y: o.h, z: d };
    parent.appendChild(n);
    return n;
  }

  function text(parent, o) {
    const n = root.createElement('text');
    n.setAttribute('value', String(o.value));
    n.setAttribute('size', String(o.size));
    n.setAttribute('color', o.color || C.text);
    n.setAttribute('touchable', 'false');
    n.position = { x: o.x || 0, y: o.y || 0, z: o.z === undefined ? M.zText : o.z };
    parent.appendChild(n);
    return n;
  }

  // El glifo va sobre el **mismo nodo** que la placa: con `texture-face="front"`
  // el motor entra en modo overlay y el shader hace
  //
  //     base_color.rgb = mix(base_color.rgb, texel.rgb, texel.a)
  //
  // o sea compone el PNG sobre el color usando su alfa. Un nodo por botón en vez
  // de dos, y sin capa extra en z que pueda parpadear. De ahí que los PNG del
  // atlas tengan que ser **blancos**: en overlay el color del glifo lo pone la
  // textura y el atributo `color` es el del fondo.
  function glyph(node, name, padding) {
    const i = GLYPHS.indexOf(name);
    if (i < 0) return;
    node.setAttribute('texture', ATLAS);
    node.setAttribute('texture-region',
      [(i % 8) / 8, Math.floor(i / 8) / 4, 1 / 8, 1 / 4].join(','));
    node.setAttribute('texture-face', 'front');
    node.setAttribute('texture-fit', 'contain');
    // `texture-padding` mete la textura hacia adentro: con p ocupa (1 - 2p) del
    // lado. Sin esto el glifo llega al borde redondeado y el squircle se pierde.
    node.setAttribute('texture-padding', String(padding === undefined ? 0.26 : padding));
  }

  // ── Piezas tocables ────────────────────────────────────────────────────────
  // Tres planos: **hitbox**, **anillo** y **cara**. Los usan por igual los
  // iconos de la grilla, los botones del riel, los de la barra y los de una
  // ventana, y por eso el hover se siente igual en todos lados.
  //
  // **El blanco del hover no puede ser lo que el hover mueve.** Apuntando cerca
  // del filo, la cara se adelanta, su silueta en pantalla cambia, el puntero
  // queda afuera, sale el `pointerleave`, la cara vuelve — y queda otra vez
  // debajo del puntero. Un ciclo cerrado a la frecuencia del cuadro: parpadeo.
  // Por eso el hitbox no se mueve nunca y la cara y el anillo son decorado.
  //
  // Funciona porque el raycast del motor recorre
  // `Query<(&GlobalTransform, &Toqueable, ...)>` (ver `crates/luna/src/touch.rs`):
  // **sólo los touchable son candidatos**, así que un nodo no-touchable por
  // delante no tapa el rayo aunque sea opaco.
  // Cuánto se adelanta una pieza al apuntarla. **No es uno solo**: un icono
  // de la grilla es grande y se apoya sobre un panel hondo, así que puede
  // salir dos centímetros sin despegarse; un botón de la barra mide un tercio
  // y vive sobre una pastilla fina, y con el mismo levante se ve flotando
  // adelante en vez de hundido en su lugar. El levante tiene que ir con el
  // tamaño de la pieza, no con el gesto.
  // Van por `ESCALA` como todo lo demás: con piezas del doble de tamaño, un
  // levante fijo se vuelve un temblor. Lo que no escala es la **rapidez**, que
  // es un tiempo y no una distancia.
  const LIFT = 0.022 * M.escala;        // grilla de aplicaciones
  const LIFT_SMALL = 0.007 * M.escala;  // barra de abajo: un tercio
  const LIFT_RAIL = 0.011 * M.escala;   // riel y navegación, al costado del panel
  const K_LIFT = 16;                    // rapidez del acercamiento, en 1/s
  const PRESS = 0.011 * M.escala;       // cuánto se hunde al tocarla
  const K_PRESS = 13;
  const MS_PRESS = 105;      // cuánto se queda abajo antes de volver

  const flying = [];
  let hovered = null;
  let lastTs = 0;

  function piece(parent, o) {
    const side = o.size;
    const ringSide = side + 2 * M.ring;
    // El radio del anillo NO es la misma fracción que el de la cara: un contorno
    // concéntrico tiene radio exterior = interior + grosor, y como acá el radio
    // es fracción del lado hay que rehacer la cuenta con el lado nuevo.
    // Repitiendo la fracción, el anillo sale más cuadrado que la cara y asoma
    // por las esquinas.
    const corner = o.corner;
    const ringCorner = Math.min(0.499, (corner * side + M.ring) / ringSide);

    const hit = plane(parent, { x: o.x, y: o.y, z: M.zHit, w: ringSide, h: ringSide,
                                color: o.color, corner: ringCorner, touchable: true });
    const ring = plane(parent, { x: o.x, y: o.y, z: M.zRing, w: ringSide, h: ringSide,
                                 color: o.color, corner: ringCorner });
    const face = plane(parent, { x: o.x, y: o.y, z: M.zFace, w: side, h: side,
                                 color: o.color, corner });
    if (o.glyph) glyph(face, o.glyph, o.padding);
    // Un glifo al revés es el mismo glifo girado: la placa es simétrica, así
    // que alcanza para tener «arriba» y «abajo» con un solo dibujo en el atlas.
    if (o.flip) face.rotation = { x: 0, y: 0, z: Math.PI };
    if (o.name) hit.setAttribute('name', o.name);

    const it = { hit, ring, face, color: o.color, x: o.x || 0, y: o.y || 0,
                 z: 0, zTarget: 0, press: 0, until: 0, pending: false,
                 lift: o.lift === undefined ? LIFT : o.lift,
                 onTap: o.onTap, selected: false };

    hit.addEventListener('pointerenter', function () {
      hovered = it;
      it.pending = false;
      applyPiece(it);
    });
    hit.addEventListener('pointerleave', function () {
      // Con dos rayos en VR, el leave de uno no significa que nadie apunte: el
      // otro puede seguir adentro, y eso lo contesta `matches(':hover')`.
      if (hit.matches(':hover')) return;
      // Una pieza siempre apaga lo suyo, mande o no en `hovered`: el enter del
      // vecino llega ANTES que este leave, y si se saliera por no ser el que
      // manda, el anillo quedaría blanco para siempre.
      if (hovered === it) hovered = null;
      // La salida espera a que el click termine de animarse: apretar el gatillo
      // mueve el rayo, y un temblor de un cuadro sacaba el puntero justo
      // mientras la pieza se hundía.
      if (inPress(it, lastTs)) it.pending = true;
      else applyPiece(it);
    });
    hit.addEventListener('toque', function () {
      strike(it);
      if (typeof it.onTap === 'function') it.onTap();
    });

    flying.push(it);
    return it;
  }

  // El anillo se enciende con el puntero y con nada más. Compartirlo con un
  // segundo estado —«seleccionado», por ejemplo— hace que al salir el hover la
  // pieza baje pero el anillo se quede blanco: la mitad de la salida ocurre y la
  // otra no, y se lee como trabado. Lo seleccionado se marca aparte.
  function applyPiece(it) {
    const on = it === hovered;
    it.ring.setAttribute('color', on ? C.white : (it.selected ? C.white : it.color));
    it.zTarget = on ? it.lift : 0;
    if (flying.indexOf(it) === -1) flying.push(it);
  }

  function inPress(it, now) {
    return now < it.until || it.press > 0.0005;
  }

  // El golpe no se escribe: se persigue. Fijar la profundidad de una y dejarla
  // decaer da un salto —no hay bajada, ya está abajo— y tocar de nuevo la manda
  // otra vez al fondo. Lo que fija el toque es una **ventana de tiempo**, y
  // `press` corre detrás de ese cuadrado con el mismo suavizado que todo lo
  // demás, así que tocar de nuevo mientras se recupera la extiende desde donde
  // esté.
  function strike(it) {
    it.until = lastTs + MS_PRESS;
    if (flying.indexOf(it) === -1) flying.push(it);
  }

  function recolor(it, color) {
    if (!it || it.color === color) return;
    it.color = color;
    it.face.setAttribute('color', color);
    it.hit.setAttribute('color', color);
    if (it !== hovered && !it.selected) it.ring.setAttribute('color', color);
  }

  function select(it, on) {
    if (!it || it.selected === !!on) return;
    it.selected = !!on;
    applyPiece(it);
  }

  function drop(it) {
    if (!it) return;
    it.dead = true;
    if (hovered === it) hovered = null;
    try { it.hit.remove(); } catch (_) {}
    try { it.ring.remove(); } catch (_) {}
    try { it.face.remove(); } catch (_) {}
  }

  // El suavizado es `1 - exp(-k dt)` y no `k dt` porque el segundo depende del
  // cuadro: a 72 fps y a 500 la pieza tiene que tardar lo mismo.
  function tick(ts) {
    requestAnimationFrame(tick);
    const dt = Math.min(0.05, lastTs ? (ts - lastTs) / 1000 : 0.016);
    lastTs = ts;
    const f = 1 - Math.exp(-K_LIFT * dt);
    const g = 1 - Math.exp(-K_PRESS * dt);

    for (let i = flying.length - 1; i >= 0; i--) {
      const it = flying[i];
      if (it.dead) { flying.splice(i, 1); continue; }

      // La salida que esperaba a que terminara el golpe. Se aplica antes de
      // mover nada, para que el objetivo nuevo valga ya en este cuadro.
      if (it.pending && !inPress(it, ts)) {
        it.pending = false;
        applyPiece(it);
      }

      it.z += (it.zTarget - it.z) * f;
      it.press += ((ts < it.until ? PRESS : 0) - it.press) * g;

      // Un exponencial no llega nunca: sin este corte la pieza seguiría
      // escribiendo su z para siempre, por milésimas de milímetro. Sale de la
      // lista cuando los tres se quedaron quietos: acercamiento, golpe y la
      // ventana, que todavía puede tenerla abajo esperando.
      if (Math.abs(it.zTarget - it.z) < 0.0002 && it.press < 0.0002
          && ts >= it.until && !it.pending) {
        it.z = it.zTarget;
        it.press = 0;
        flying.splice(i, 1);
      }

      const z = it.z - it.press;
      it.face.position = { x: it.x, y: it.y, z: M.zFace + z };
      it.ring.position = { x: it.x, y: it.y, z: M.zRing + z };
    }
  }
  requestAnimationFrame(tick);

  // ── Reparto ────────────────────────────────────────────────────────────────
  // n posiciones centradas en cero, con paso fijo.
  function spread(count, step) {
    const out = [];
    for (let i = 0; i < count; i++) out.push((i - (count - 1) / 2) * step);
    return out;
  }

  // ── El tablero ─────────────────────────────────────────────────────────────
  // Encabezado suelto arriba, panel con la grilla, riel a la izquierda y los
  // puntos de página a la derecha. Los rieles van **fuera** del panel y
  // alineados a la grilla, no a la placa: lo que tiene que leerse alineado son
  // los iconos.
  function dashboard(parent, cfg) {
    const apps = cfg.apps || [];
    const cols = M.cols;
    // **Las filas salen del contenido, no de la constante.** Con nueve apps y
    // cinco columnas alcanzan dos, y reservar siempre tres deja un tercio del
    // panel vacío abajo. El tope sigue siendo M.rows: pasado eso se pagina.
    const rows = apps.length > cols * M.rows
      ? M.rows
      : Math.max(1, Math.ceil(apps.length / cols));
    const perPage = cols * rows;
    const pages = Math.max(1, Math.ceil(apps.length / perPage));

    const g = group(parent, 0, cfg.y || 0, 0);
    const xs = spread(cols, M.stepX);
    const ys = spread(rows, M.stepY);

    const w = (cols - 1) * M.stepX + M.tile + 2 * M.panelPad;
    const top = ys[rows - 1] + M.tile / 2 + M.panelPad;
    const bottom = ys[0] + M.labelDy - M.labelSize - M.panelPad;
    const h = top - bottom;
    const cy = (top + bottom) / 2;

    plane(g, { x: 0, y: cy, z: M.zBack, w, h, color: C.panel, corner: M.panelCorner });

    // Encabezado: una pastilla suelta arriba del panel, no una franja adentro.
    const hy = top + M.headerGap + M.headerH / 2;
    pill(g, { x: 0, y: hy, z: M.zBack, w: M.headerW, h: M.headerH,
              r: M.headerRadius, color: C.pill });
    text(g, { x: 0, y: hy, value: cfg.title || 'Aplicaciones', size: M.headerSize });

    // Riel izquierdo.
    const railX = -w / 2 - M.railOffset - M.railSize / 2;
    const rail = cfg.rail || [];
    const railYs = spread(rail.length, M.railSize + M.railGap);
    const railPieces = [];
    for (let i = 0; i < rail.length; i++) {
      railPieces.push(piece(g, {
        x: railX, y: cy + railYs[rail.length - 1 - i],
        size: M.railSize, corner: M.railCorner, color: C.panel,
        glyph: rail[i].glyph, padding: M.padRail, lift: LIFT_RAIL,
        name: 'rail-' + rail[i].glyph, onTap: rail[i].onTap,
      }));
    }

    // La navegación de páginas: subir, los puntos, bajar. **Se centra sola** en
    // el panel: la altura del conjunto sale de cuántas páginas hay, y con los
    // puntos anclados a una altura fija el bloque se descolgaba en cuanto
    // aparecía una página más.
    // Con una sola página no hay nada que navegar: un punto solo no informa
    // nada y las flechas no llevan a ningún lado.
    const dotX = w / 2 + M.railOffset + M.railSize / 2;
    const many = pages > 1;
    const dotsSpan = (pages - 1) * M.dotGap + M.dotR * 2;
    const navH = many ? 2 * (M.railSize + M.navGap) + dotsSpan : dotsSpan;
    const navTop = cy + navH / 2;

    let page = 0;
    let upPiece = null;
    let downPiece = null;

    if (many) {
      upPiece = piece(g, {
        x: dotX, y: navTop - M.railSize / 2, size: M.railSize, corner: M.railCorner,
        color: C.panel, glyph: 'chevron', padding: M.padRail, flip: true,
        lift: LIFT_RAIL, name: 'page-prev', onTap: () => setPage(page - 1),
      });
    }

    const dotTop = many ? navTop - M.railSize - M.navGap - M.dotR : navTop - M.dotR;
    const dots = [];
    for (let i = 0; many && i < pages; i++) {
      dots.push(plane(g, {
        x: dotX, y: dotTop - i * M.dotGap, z: M.zFace,
        w: M.dotR * 2, h: M.dotR * 2, color: i === 0 ? C.white : C.dim, corner: 0.5,
      }));
    }

    if (many) {
      downPiece = piece(g, {
        x: dotX, y: cy - navH / 2 + M.railSize / 2, size: M.railSize,
        corner: M.railCorner, color: C.panel, glyph: 'chevron', padding: M.padRail,
        lift: LIFT_RAIL, name: 'page-next', onTap: () => setPage(page + 1),
      });
    }

    // Los iconos se crean una vez por página visible y se reescriben al pasar
    // de página: recrear quince piezas por cada toque cuesta treinta mutaciones
    // de DOM y una sincronización, para mover cinco etiquetas.
    const tiles = [];
    for (let i = 0; i < perPage; i++) {
      const col = i % cols;
      const row = Math.floor(i / cols);
      const tg = group(g, xs[col], ys[rows - 1 - row], 0);
      const it = piece(tg, {
        x: 0, y: 0, size: M.tile, corner: M.tileCorner, color: C.blue,
        glyph: 'app', padding: M.padTile, name: 'app-' + i,
        onTap: () => { const a = appAt(i); if (a && cfg.onOpen) cfg.onOpen(a); },
      });
      const label = text(tg, { x: 0, y: M.labelDy, value: '', size: M.labelSize, color: C.label });
      tiles.push({ group: tg, piece: it, label });
    }

    function appAt(i) { return apps[page * perPage + i] || null; }

    function setPage(p) {
      // Se recorta en vez de dar la vuelta: con flechas arriba y abajo, saltar
      // de la última a la primera contradice lo que el botón dibuja.
      page = Math.max(0, Math.min(p, pages - 1));
      for (let i = 0; i < dots.length; i++) dots[i].setAttribute('color', i === page ? C.white : C.dim);
      // El botón que no lleva a ningún lado se apaga, pero sigue tocable: que
      // desaparezca movería los otros dos y el bloque dejaría de estar centrado.
      if (upPiece) recolor(upPiece, page > 0 ? C.panel : C.pill);
      if (downPiece) recolor(downPiece, page < pages - 1 ? C.panel : C.pill);
      for (let i = 0; i < tiles.length; i++) {
        const a = appAt(i);
        const t = tiles[i];
        // Una celda sin app no se borra: se apaga. Quitar y recrear nodos por
        // paginar es lo que hacía parpadear la grilla entera.
        t.group.setAttribute('visible', a ? 'true' : 'false');
        if (!a) continue;
        t.label.setAttribute('value', a.name || '');
        recolor(t.piece, a.color || C.blue);
        glyph(t.piece.face, a.glyph || 'app', M.padTile);
      }
    }
    setPage(0);

    // `tiles` sale afuera para que quien lo monte pueda animarlos: la cascada
    // de entrada es del shell, no del tablero, y necesita tocarlos de a uno.
    //
    // Y `destroy` porque el tablero se arma con los marcadores que le pasan, y
    // esos pueden cambiar: quitar el grupo se lleva los nodos, pero las piezas
    // además hay que sacarlas del bucle de animación o seguirían escribiendo
    // la z de un nodo que ya no existe.
    return {
      group: g, width: w, height: h, centerY: cy, setPage, tiles,
      rail: railPieces, upPiece, downPiece,
      destroy() {
        for (const t of tiles) drop(t.piece);
        for (const p of railPieces) drop(p);
        if (upPiece) drop(upPiece);
        if (downPiece) drop(downPiece);
        try { g.remove(); } catch (_) {}
      },
    };
  }

  // ── La barra ───────────────────────────────────────────────────────────────
  // Dos pastillas: estado a la izquierda, ventanas a la derecha. Cada una se
  // dimensiona por su contenido; separarlas en dos deja dicho qué cosa va con
  // qué sin necesidad de un separador dibujado.
  // `cfg.y` la ata al panel: el llamador pasa dónde termina el tablero y la
  // barra se cuelga de ahí. Con una altura fija, cualquier cambio de escala o
  // de cantidad de filas la dejaba flotando lejos o encima del panel.
  function bar(parent, cfg) {
    const g = group(parent, 0, cfg.y === undefined ? M.barY : cfg.y, 0);
    let pills = [];
    let pieces = [];

    function clear() {
      for (const p of pieces) drop(p);
      for (const n of pills) { try { n.remove(); } catch (_) {} }
      pieces = [];
      pills = [];
    }

    function pillWidth(n) {
      return n * M.barItem + (n - 1) * M.barGap + 2 * M.barPad;
    }

    function render(groups) {
      clear();
      const widths = groups.map(items => pillWidth(items.length));
      const total = widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * M.barPillGap;
      let cursor = -total / 2;
      for (let gi = 0; gi < groups.length; gi++) {
        const w = widths[gi];
        const cx = cursor + w / 2;
        cursor += w + M.barPillGap;
        pills.push(pill(g, { x: cx, y: 0, z: M.zBack, w, h: M.barH,
                             r: M.barPillRadius, color: C.pill }));
        const xs = spread(groups[gi].length, M.barItem + M.barGap);
        for (let j = 0; j < groups[gi].length; j++) {
          const item = groups[gi][j];
          const it = piece(g, {
            x: cx + xs[j], y: 0, size: M.barItem, corner: M.barItemCorner,
            color: item.color || C.blue, glyph: item.glyph, padding: M.padBar,
            lift: LIFT_SMALL, name: item.name, onTap: item.onTap,
          });
          if (item.selected) select(it, true);
          pieces.push(it);
          // El punto de aviso: chico, arriba a la derecha del botón, sin
          // hitbox propio — es información, no un blanco.
          if (item.badge) {
            const b = plane(g, {
              x: cx + xs[j] + M.barItem * 0.40, y: M.barItem * 0.40, z: M.zGlyph,
              w: 0.018, h: 0.018, color: C.red, corner: 0.5,
            });
            pills.push(b);
          }
        }
      }
    }

    return {
      group: g, render,
      destroy() { clear(); try { g.remove(); } catch (_) {} },
    };
  }

  // ── Una ventana ────────────────────────────────────────────────────────────
  // El hueco donde dibuja la app, y **debajo** una pastilla con el título y sus
  // tres botones. Va abajo y no arriba porque arriba compite con el contenido:
  // una barra de título encima de la app le come el borde superior y, cuando la
  // ventana está a la altura de los ojos, tapa justo lo que se está mirando.
  function windowFrame(parent, cfg) {
    const w = cfg.width;
    const h = cfg.height;
    const g = group(parent, cfg.x || 0, cfg.y || 0, cfg.z || 0);
    const nodes = [];
    const pieces = [];

    nodes.push(plane(g, { x: 0, y: 0, z: M.zBack, w, h,
                          color: C.viewport, corner: M.winCorner }));

    const ty = -h / 2 - M.winTitleGap - M.winTitleH / 2;
    // El orden es el de **colocación**, que va de derecha a izquierda: el
    // primero queda pegado al filo. Así, leídos de izquierda a derecha, salen
    // fijar · minimizar · cerrar, con el rojo en la punta.
    const btns = [
      { glyph: 'close', color: C.red, onTap: cfg.onClose },
      { glyph: 'minimize', color: C.dim, onTap: cfg.onMinimize },
      { glyph: 'anchor', color: C.blue, onTap: cfg.onAnchor },
    ].filter(b => typeof b.onTap === 'function');

    const btnBlock = btns.length * M.winBtn + (btns.length - 1) * M.winBtnGap;
    const titleW = Math.max(0.26, btnBlock + 0.20);
    nodes.push(pill(g, { x: 0, y: ty, z: M.zBack, w: titleW, h: M.winTitleH,
                         r: M.winTitleRadius, color: C.pill }));

    // El título se centra en lo que sobra a la izquierda de los botones, no en
    // la pastilla entera: centrado en la pastilla queda debajo de ellos.
    const btnLeft = titleW / 2 - 0.012 - btnBlock;
    const title = text(g, {
      x: (-titleW / 2 + btnLeft) / 2, y: ty,
      value: shortTitle(cfg.title), size: M.winTitleSize, color: C.label,
    });
    nodes.push(title);

    let bx = titleW / 2 - 0.012 - M.winBtn / 2;
    for (const b of btns) {
      pieces.push(piece(g, {
        x: bx, y: ty, size: M.winBtn, corner: 0.5, color: b.color,
        glyph: b.glyph, padding: M.padWin, onTap: b.onTap,
      }));
      bx -= M.winBtn + M.winBtnGap;
    }

    function shortTitle(t) {
      const v = t || 'App';
      return v.length > 18 ? v.slice(0, 17) + '…' : v;
    }

    return {
      group: g,
      setTitle: (t) => title.setAttribute('value', shortTitle(t)),
      anchorPiece: pieces[pieces.length - 1] || null,
      destroy() {
        for (const p of pieces) drop(p);
        for (const n of nodes) { try { n.remove(); } catch (_) {} }
        try { g.remove(); } catch (_) {}
      },
    };
  }

  globalThis.ShellUI = {
    M, C, GLYPHS,
    group, plane, pill, text, glyph, piece, recolor, select, drop, spread,
    dashboard, bar, windowFrame,
  };
  console.log('[shell_ui] listo —', GLYPHS.length, 'glifos');
})();
