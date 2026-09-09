// ─────────────────────────────────────────────────────────────────────────────
// shell_draw.js — el vocabulario visual del shell.
//
// Arco, glifos, piezas tocables y la malla de los fondos. Nada de esto sabe qué
// se está dibujando: son las primitivas con las que el menú y las ventanas se
// construyen.
//
// Lo cargan **los dos isolates** —el del controller y el del include del menú—
// porque cada uno dibuja su parte y no hay forma de compartir objetos entre
// ellos. Cada isolate tiene su copia; lo que se comparte es el código, no el
// estado.
// ─────────────────────────────────────────────────────────────────────────────
(function () {
  // Diez tonos que se repiten. En un menú de verdad el color lo trae cada
  // app; acá sólo tienen que distinguirse entre vecinos.
  const PALETTE = [
    '#3B82F6', '#10B981', '#F59E0B', '#EF4444',
    '#8B5CF6', '#EC4899', '#14B8A6', '#F97316',
    '#22C55E', '#EAB308',
  ];

  function init(root) {

      // ── Medidas ────────────────────────────────────────────────────────
      // Metros, y las alturas son **relativas a los ojos**: el shell se planta
      // a SHELL_SPAWN_DISTANCE al frente del visitante, así que y=0 es la
      // altura de la mirada y lo que cuelga por debajo va en negativo.
      const R_PANEL = 1.5;    // = SHELL_SPAWN_DISTANCE
      const R_BAR   = 1.15;   // la barra va más cerca: está más abajo y al alcance

      // Grilla
      const GRID_COLS   = 5;
      const GRID_STEP_X = 0.24;
      const GRID_STEP_Y = 0.30;
      const SIZE_APP    = 0.17;
      // FRACCIÓN del lado, no metros. En un `plane` el radio se pasa crudo a
      // `create_rounded_plane`, que trabaja sobre un cuadrado unitario; en un
      // `box` el mismo atributo son metros del mundo divididos por la escala.
      // Por eso el icono es un plane: con una caja de 0,17 y radio 0,2 el radio
      // satura los tres ejes y el icono sale esfera.
      const CORNER      = 0.22;
      const LABEL_DY    = -0.122;
      const LABEL_SIZE  = 0.028;
      const GRID_Y      = -0.20;
      const HEADER_DY   = 0.235;

      // Capas en z dentro del grupo de cada pieza. Sin esto el motor decide el
      // orden solo y las etiquetas parpadean contra su propio fondo.
      const Z_HIT   = -0.0012;
      const Z_RING  = -0.0008;
      const Z_FACE  = 0;
      const Z_GLYPH = 0.0025;
      const Z_LABEL = 0.005;
      const RING    = 0.0025;   // grosor del anillo de hover

      // Barra
      const BAR_Y        = -0.80;
      const BAR_ITEM     = 0.072;   // lado de un botón redondo de la barra
      const BAR_H        = 0.125;   // alto de una pastilla
      const BAR_GAP      = 0.026;   // aire entre botones de un grupo
      const BAR_PAD      = 0.070;   // aire entre el grupo y su pastilla
      const BAR_GROUP_GAP = 0.090;  // aire entre pastillas
      // Sale de la geometría y no del gusto: atan((ojos - yBarra) / radio) es
      // el ángulo con que se mira la barra, y la barra tiene que devolverlo
      // para quedar de frente. Con 0 se le ve el canto.
      const BAR_TILT     = -Math.atan2(-BAR_Y, R_BAR);

      // Colores
      const BLANCO    = '#F2F5FA';
      const PLACA     = '#171A21';
      const PLACA_ALTA = '#252A34';
      const TENUE     = '#8A94A6';

      // ── El arco ────────────────────────────────────────────────────────
      // La UI vive en un cilindro alrededor del visitante, no en un plano. Un
      // panel plano de 1,3 m a 1,5 m de distancia se lee torcido en los bordes:
      // las columnas de las puntas quedan de costado y más lejos que las del
      // centro.
      //
      // El grupo del shell ya está a `radio` al frente y girado por el yaw, así
      // que en su sistema local el visitante está en (0, 0, radio) y el punto a
      // `u` metros de arco es
      //
      //     t = u / R    x = R sen t    z = R (1 - cos t)
      //
      // con **ry = -t** para que la pieza lo mire. De ahí sale que el +z local
      // de cada pieza apunta a los ojos: acercar un icono es sumarle z.
      function onArc(u, radius) {
        const t = u / radius;
        return { x: Math.sin(t) * radius, z: radius * (1 - Math.cos(t)), ry: -t };
      }

      function surface(parent, u, y, radius, tilt) {
        const group = root.createElement('group');
        const a = onArc(u, radius || SHELL_SPAWN_DISTANCE);
        group.position = { x: a.x, y, z: a.z };
        group.rotation = { x: tilt || 0, y: a.ry, z: 0 };
        parent.appendChild(group);
        return group;
      }

      function plane(parent, opts) {
        const node = root.createElement('plane');
        node.setAttribute('color', opts.color);
        node.setAttribute('border-radius', String(opts.corner));
        node.setAttribute('touchable', opts.touchable ? 'true' : 'false');
        node.position = { x: opts.x || 0, y: opts.y || 0, z: opts.z || 0 };
        node.scale = { x: opts.sx, y: opts.sy, z: 1 };
        parent.appendChild(node);
        return node;
      }

      function caption(parent, value, y, size, color, x) {
        const node = root.createElement('text');
        node.setAttribute('value', String(value));
        node.setAttribute('size', String(size));
        node.setAttribute('color', color || BLANCO);
        node.setAttribute('touchable', 'false');
        node.position = { x: x || 0, y, z: Z_LABEL };
        parent.appendChild(node);
        return node;
      }

      // ── Los glifos ─────────────────────────────────────────────────────
      // Un atlas de 8x4 servido por el propio binario. El slot de cada icono es
      // su índice en `tools/build_icon_atlas.py`, y esta tabla es la otra mitad
      // de ese contrato: agregar un icono es agregarlo **al final** allá y acá.
      // Antes el slot salía del índice del bookmark, que coincidía de casualidad
      // con el orden del generador.
      const ATLAS = 'luna://icons/menu.png';
      const GLYPHS = ['home', 'settings', 'demos', 'app', 'about', 'scale',
                      'fire', 'target', 'range', 'user', 'wifi', 'battery',
                      'bell', 'capture', 'close', 'minimize', 'anchor'];

      // El glifo va sobre el **mismo nodo** que la placa: con
      // `texture-face="front"` el motor entra en modo overlay y compone el PNG
      // sobre el color usando su alfa. Un nodo por icono en vez de dos, y sin
      // capa extra en z que pueda parpadear. De ahí que los PNG del atlas sean
      // blancos: en overlay el color del glifo lo pone la textura.
      function glyph(node, name, padding) {
        const i = GLYPHS.indexOf(name);
        if (i < 0) return;
        node.setAttribute('texture', ATLAS);
        node.setAttribute('texture-region', [(i % 8) / 8, Math.floor(i / 8) / 4, 1 / 8, 1 / 4].join(','));
        node.setAttribute('texture-face', 'front');
        node.setAttribute('texture-fit', 'contain');
        node.setAttribute('texture-padding', String(padding));
      }

      // ── Piezas tocables ────────────────────────────────────────────────
      // Una pieza son tres planos: el **hitbox**, el **anillo** y la **cara**.
      // Los usan por igual los iconos de la grilla, los botones de la barra y
      // los de la barra de título de una ventana, y por eso el hover se siente
      // igual en los tres lados.
      //
      // **El blanco del hover no puede ser lo que el hover mueve.** Apuntando
      // cerca del filo, la cara se adelanta, su silueta en pantalla cambia, el
      // puntero queda afuera, sale el `pointerleave`, la cara vuelve — y queda
      // otra vez debajo del puntero. Un ciclo cerrado a la frecuencia del
      // cuadro: parpadeo. Por eso el hitbox no se mueve nunca y la cara y el
      // anillo son decorado.
      //
      // Que eso funcione depende de un detalle del motor, verificado en
      // `touch.rs`: el raycast recorre `Query<(&GlobalTransform, &Toqueable,
      // ...)>`, o sea que **sólo los touchable son candidatos**. Un nodo
      // no-touchable por delante no tapa el rayo aunque sea opaco.
      const LEVANTE   = 0.028;   // cuánto se adelanta una pieza al apuntarla
      const K_Z       = 16;      // rapidez del acercamiento, en 1/s
      const HUNDIDO   = 0.013;   // cuánto se mete al tocarla
      const K_GOLPE   = 13;      // rapidez del hundimiento y del retorno
      const MS_APRETADO = 105;   // cuánto se queda abajo antes de volver

      const enVuelo = [];
      let hoverPiece = null;
      let previoTs = 0;

      // Una pastilla: un fondo más ancho que alto con esquinas redondeadas.
      // Va como `box` y no como `plane` porque en un plane el `border-radius`
      // es una **fracción del lado** y al escalar se estira con él —una
      // pastilla queda convertida en un óvalo—, mientras que en una caja
      // `rounded_box_radii` divide el radio **en metros** por la escala de cada
      // eje y conserva el radio del mundo.
      //
      // Una caja crece hacia los dos lados, así que se la corre media
      // profundidad para que su cara quede en la z pedida.
      const PILL_DEPTH = 0.020;
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

      function piece(parent, opts) {
        const lado = opts.size;
        const ladoRing = lado + 2 * RING;
        // El radio del anillo NO es la misma fracción que el de la cara: un
        // contorno concéntrico tiene radio exterior = interior + grosor, y como
        // acá el radio es fracción del lado hay que rehacer la cuenta con el
        // lado nuevo. Repitiendo la fracción, el anillo sale más cuadrado que la
        // cara y asoma por las esquinas.
        const corner = opts.corner;
        const cornerRing = Math.min(0.499, (corner * lado + RING) / ladoRing);
        const color = opts.color;
        // Todas las capas de esta pieza se corren juntas en z. Sirve para las
        // que se apoyan **encima** de otra —el punto de cerrar sobre el botón
        // de una ventana—: sin esto quedan a la misma profundidad que aquello
        // sobre lo que están y las dos caras pelean por el mismo píxel.
        const dz = opts.zOffset || 0;

        // El hitbox lleva la forma y el tamaño del anillo, medio milímetro más
        // atrás: una silueta idéntica y más lejos no asoma por ningún lado.
        const hit = plane(parent, { x: opts.x, y: opts.y, z: Z_HIT + dz, sx: ladoRing,
                                    sy: ladoRing, color, corner: cornerRing, touchable: true });
        const ring = plane(parent, { x: opts.x, y: opts.y, z: Z_RING + dz, sx: ladoRing,
                                     sy: ladoRing, color, corner: cornerRing });
        const face = plane(parent, { x: opts.x, y: opts.y, z: Z_FACE + dz, sx: lado,
                                     sy: lado, color, corner });
        if (opts.glyph) glyph(face, opts.glyph, opts.padding || 0.24);
        if (opts.name) hit.setAttribute('name', opts.name);

        const it = { hit, ring, face, color, x: opts.x || 0, y: opts.y || 0,
                     dz,
                     z: 0, zTarget: 0, golpe: 0, hasta: 0, pendiente: false,
                     onTap: opts.onTap };

        hit.addEventListener('pointerenter', () => {
          hoverPiece = it;
          it.pendiente = false;
          applyPiece(it);
          if (typeof opts.onHover === 'function') opts.onHover(true);
        });
        hit.addEventListener('pointerleave', () => {
          // Con dos rayos en VR, el leave de uno no significa que nadie apunte:
          // el otro puede seguir adentro, y eso lo contesta `matches(':hover')`.
          if (hit.matches(':hover')) return;
          // Una pieza siempre apaga lo suyo, mande o no en `hoverPiece`: el
          // enter del vecino llega ANTES que este leave, y si se saliera por no
          // ser el que manda, el anillo quedaría blanco para siempre.
          if (hoverPiece === it) hoverPiece = null;
          // La salida espera a que el click termine de animarse: apretar el
          // gatillo mueve el rayo, y un temblor de un cuadro sacaba el puntero
          // justo mientras la pieza se hundía.
          if (enGolpe(it, previoTs)) it.pendiente = true;
          else applyPiece(it);
          if (typeof opts.onHover === 'function') opts.onHover(false);
        });
        hit.addEventListener('toque', () => {
          golpear(it);
          if (typeof it.onTap === 'function') it.onTap();
        });

        enVuelo.push(it);   // una pasada para dejar la z inicial escrita
        return it;
      }

      // El anillo se enciende con el puntero y con nada más. Compartirlo con un
      // segundo estado —el de "tocado", por ejemplo— hace que al salir el hover
      // la pieza baje pero el anillo se quede blanco: la mitad de la salida
      // ocurre y la otra no, y se lee como trabado.
      function applyPiece(it) {
        it.ring.setAttribute('color', it === hoverPiece ? BLANCO : it.color);
        it.zTarget = it === hoverPiece ? LEVANTE : 0;
        if (enVuelo.indexOf(it) === -1) enVuelo.push(it);
      }

      function recolorPiece(it, color) {
        if (!it || it.color === color) return;
        it.color = color;
        it.face.setAttribute('color', color);
        it.hit.setAttribute('color', color);
        // El anillo sólo si no lo están apuntando: pisarlo ahora le sacaría el
        // blanco a una pieza que sigue bajo el puntero.
        if (it !== hoverPiece) it.ring.setAttribute('color', color);
      }

      function enGolpe(it, ahora) {
        return ahora < it.hasta || it.golpe > 0.0005;
      }

      // El golpe no se escribe: se persigue. Fijar la profundidad de una y
      // dejarla decaer da un salto —no hay bajada, ya está abajo— y tocar de
      // nuevo la manda otra vez al fondo. Lo que fija el toque es una **ventana
      // de tiempo**, y `golpe` corre detrás de ese cuadrado con el mismo
      // suavizado que todo lo demás, así que tocar de nuevo mientras se recupera
      // extiende la ventana desde donde esté.
      function golpear(it) {
        it.hasta = previoTs + MS_APRETADO;
        if (enVuelo.indexOf(it) === -1) enVuelo.push(it);
      }

      // El suavizado es `1 - exp(-k dt)` y no `k dt` porque el segundo depende
      // del cuadro: a 90 fps y a 500 la pieza tiene que tardar lo mismo.
      function tickPieces(ts) {
        requestAnimationFrame(tickPieces);
        const dt = Math.min(0.05, previoTs ? (ts - previoTs) / 1000 : 0.016);
        previoTs = ts;
        const f = 1 - Math.exp(-K_Z * dt);
        const g = 1 - Math.exp(-K_GOLPE * dt);

        for (let i = enVuelo.length - 1; i >= 0; i--) {
          const it = enVuelo[i];
          if (it.dead) { enVuelo.splice(i, 1); continue; }

          // La salida que esperaba a que terminara el golpe. Se aplica antes de
          // mover nada, para que el objetivo nuevo valga ya en este cuadro.
          if (it.pendiente && !enGolpe(it, ts)) {
            it.pendiente = false;
            applyPiece(it);
          }

          it.z += (it.zTarget - it.z) * f;
          it.golpe += ((ts < it.hasta ? HUNDIDO : 0) - it.golpe) * g;

          // Un exponencial no llega nunca: sin este corte la pieza seguiría
          // escribiendo su z para siempre, por milésimas de milímetro. Sale de
          // la lista cuando los tres se quedaron quietos: acercamiento, golpe y
          // la ventana, que todavía puede tenerla abajo esperando.
          const quieto = Math.abs(it.zTarget - it.z) < 0.0002
                      && it.golpe < 0.0002 && ts >= it.hasta && !it.pendiente;
          if (quieto) { it.z = it.zTarget; it.golpe = 0; enVuelo.splice(i, 1); }

          const z = it.z - it.golpe + it.dz;
          it.face.position = { x: it.x, y: it.y, z: Z_FACE + z };
          it.ring.position = { x: it.x, y: it.y, z: Z_RING + z };
        }
      }
      requestAnimationFrame(tickPieces);

      // Soltar una pieza: se saca del bucle y se van sus nodos.
      function dropPiece(it) {
        if (!it) return;
        it.dead = true;
        if (hoverPiece === it) hoverPiece = null;
        try { it.hit.remove(); } catch (_) {}
        try { it.ring.remove(); } catch (_) {}
        try { it.face.remove(); } catch (_) {}
      }

      // ── La malla de los fondos ─────────────────────────────────────────
      // Todos los fondos de un grupo en UNA malla que sigue el arco. Un
      // `<plane>` es plano y las piezas van sobre un cilindro: la flecha del
      // arco sobre el ancho del panel —1,3 m sobre radio 1,5— da **14 cm**, así
      // que la placa se hunde justo donde los iconos se adelantan.
      //
      // Para cada muestra se calcula el punto del arco y el **arriba local**,
      // que no es (0,1,0) sino el (0,1,0) girado `phi` alrededor de la tangente
      // — que es lo que hace que la barra mire hacia arriba:
      //
      //     n(t)   = (-sen t, 0, cos t)              hacia el visitante
      //     up(t)  = (sen t · sen φ, cos φ, -cos t · sen φ)
      //     normal = n · cos φ + (0,1,0) · sen φ
      //
      // El orden de los triángulos importa —el motor saca la cara de ahí— y sale
      // de que (tangente, up, normal) es una terna a derechas.
      const MESH_STEPS = 40;   // fijo: `update` manda siempre la misma cantidad

      function rgba(hex) {
        const v = parseInt(String(hex).replace('#', ''), 16);
        // El motor rechaza la malla entera si un canal se va de [0,1].
        return [((v >> 16) & 255) / 255, ((v >> 8) & 255) / 255, (v & 255) / 255, 1];
      }

      // Cuánto mide de alto una pastilla a `x` metros de su centro: recto en el
      // medio, arco de radio `r` en las puntas.
      function halfHeight(x, width, height, r) {
        const d = width / 2 - Math.abs(x);
        if (d >= r) return height / 2;
        const c = r - d;
        return height / 2 - r + Math.sqrt(Math.max(0, r * r - c * c));
      }

      function buildMesh(pills) {
        const positions = [];
        const normals = [];
        const colors = [];
        const indices = [];
        for (const P of pills) {
          const r = Math.min(0.05, P.height / 2 - 0.001);
          const col = rgba(P.color);
          const base = positions.length / 3;   // en vértices, no en flotantes
          const phi = P.phi || 0;
          const sf = Math.sin(phi), cf = Math.cos(phi);
          // **El centro de curvatura es el del panel, no el de la pastilla.** El
          // fondo va `depth` metros más lejos que las piezas, y eso es un radio
          // mayor **alrededor del mismo centro** —que es el visitante, en
          // (0, 0, radio)—. Calculando `cz = R(1 - cos t)` con el radio propio,
          // el centro se corre con él: en t=0 da z=0, o sea coplanar con los
          // iconos, y hacia los costados se separa para el lado equivocado. Los
          // iconos del medio quedaban tragados por su propio fondo y sólo se
          // salvaban los de las puntas.
          const R = P.radius + P.depth;
          for (let i = 0; i <= MESH_STEPS; i++) {
            const x = -P.width / 2 + (P.width * i) / MESH_STEPS;
            // El ángulo sale del radio de las piezas: así el fondo cubre el mismo
            // tramo angular que ellas y no se corre al alejarse.
            const t = (P.u + x) / P.radius;
            const cx = Math.sin(t) * R;
            const cz = P.radius - R * Math.cos(t);
            const ux = Math.sin(t) * sf, uy = cf, uz = -Math.cos(t) * sf;
            const nx = -Math.sin(t) * cf, ny = sf, nz = Math.cos(t) * cf;
            const h = halfHeight(x, P.width, P.height, r);
            positions.push(cx - ux * h, P.y - uy * h, cz - uz * h);
            positions.push(cx + ux * h, P.y + uy * h, cz + uz * h);
            normals.push(nx, ny, nz, nx, ny, nz);
            colors.push(col[0], col[1], col[2], col[3], col[0], col[1], col[2], col[3]);
          }
          for (let i = 0; i < MESH_STEPS; i++) {
            const a = base + i * 2, b = a + 1, c = a + 2, d = a + 3;
            indices.push(a, c, b, c, d, b);
          }
        }
        // Arrays tipados y planos: la API los quiere así. Pasarle arrays de
        // ternas no da un error de tipo sino «Mesh attributes must be finite and
        // bounded», que manda a buscar un NaN que no existe.
        return { positions: new Float32Array(positions),
                 indices: new Uint32Array(indices),
                 normals: new Float32Array(normals),
                 colors: new Float32Array(colors) };
      }

      // Se crea una vez y se **actualiza**: el número de vértices no cambia
      // —MESH_STEPS es fijo y la cantidad de pastillas también— así que alcanza
      // con reescribir lo que se movió. Crear una malla nueva por cada cambio
      // gastaría del tope de 128 recursos, que además no se liberan al cambiar
      // de espacio.
      function paintMesh(slot, node, pills) {
        if (!node) return;
        const data = buildMesh(pills);
        if (!slot.mesh) {
          slot.mesh = MeshResource.create(data);
          node.src = slot.mesh.src;
        } else {
          // Los cuatro buffers, siempre: los que uno no pasa viajan vacíos, no
          // "sin cambios", y con `indices` vacío el motor los regenera
          // implícitos y la malla se vuelve tiras de triángulos.
          slot.mesh.update(data);
        }
      }
      const panelMesh = {};
      const barMesh = {};

      // ── Animación enter/exit ───────────────────────────────────────────
      const ANIM_MS    = 160;
      const STAGGER_MS = 12;
      function easeOutBack(t) {
        const c1 = 1.70158, c3 = c1 + 1;
        return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
      }
      function easeInBack(t) {
        const c1 = 1.70158, c3 = c1 + 1;
        return c3 * t * t * t - c1 * t * t;
      }


    return {
      onArc, surface, plane, pill, caption, glyph, piece, applyPiece, recolorPiece,
      enGolpe, golpear, dropPiece, buildMesh, paintMesh, rgba, halfHeight,
      easeOutBack, easeInBack,
      // La tabla de glifos y la paleta salen enteras: quien dibuje encima
      // tiene que poder elegir un icono por nombre sin duplicar el índice.
      GLYPHS, PALETTE,
      // Las medidas salen como objeto para que quien dibuje encima no las
      // vuelva a escribir con otro número.
      M: {
        R_PANEL, R_BAR, GRID_COLS, GRID_STEP_X, GRID_STEP_Y, SIZE_APP, CORNER,
        LABEL_DY, LABEL_SIZE, GRID_Y, HEADER_DY,
        Z_HIT, Z_RING, Z_FACE, Z_GLYPH, Z_LABEL, RING,
        BAR_Y, BAR_ITEM, BAR_H, BAR_GAP, BAR_PAD, BAR_GROUP_GAP, BAR_TILT,
        BLANCO, PLACA, PLACA_ALTA, TENUE,
      },
    };
  }

  globalThis.ShellDraw = { init: init };
})();
