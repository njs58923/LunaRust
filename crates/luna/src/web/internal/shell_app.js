// ─────────────────────────────────────────────────────────────────────────────
// shell_app.js — el shell de Luna: su interfaz y su máquina de estados.
//
// Es, tal cual, lo que vivía adentro de `ux_vr.hsml`. Lo único que cambió es de
// dónde saca las cosas: **no toca el host**. Todo lo que sale de este módulo
// hacia el motor —abrir una pestaña, cerrarla, mandarle un mensaje a una app,
// leer la pose del visitante— entra por `cfg`. El controller que lo monta decide
// cómo se cumplen esas operaciones, y por eso el mismo shell puede montarse
// desde más de un controller aunque lleguen a él con capacidades distintas.
//
// Va como script y no como `<include>`: un include es otro documento, con su
// propio isolate y sus propios permisos, y desde ahí no habría forma de hablarle
// a las pestañas ni de mandarle el slot a una app.
//
// `cfg`:
//   root, panel, bottomBar, focusZone, anchoredZone, panelBg, barBg  — nodos
//   bookmarks   — [{ name, url, kind }]
//   tabs        — { open(url, opts), close(id), setVisible(id, visible) }
//   send(tabId, payload) / poll()   — mensajería con las apps
//   viewerPose()                    — { px, py, pz, yaw } o null
//   navigate(url)                   — salida de emergencia sin pestañas
//
// **Regla de oro, heredada y sin cambiar: todo cambio en `shellState` va seguido
// de `applyVisibility()`.** Nada dibuja por su cuenta.
// ─────────────────────────────────────────────────────────────────────────────
(function () {
  function mount(cfg) {
      // ── Puentes al host ────────────────────────────────────────────────
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
      // ────────────────────────────────────────────────────────────────────
      // ux_vr shell — orquesta bookmarks + bottom bar + (futuro) focus/anchored.
      // Implementación del state machine definido en docs/embedded_apps.md.
      // ────────────────────────────────────────────────────────────────────

      const root = cfg.root;

      // ── Bookmarks ──────────────────────────────────────────────────────
      // TODO(persistence): mover a storage cuando exista API.
      // kind:  'spatial' (default) | 'app' | 'app-embedded' (futuro)
      const BOOKMARKS = cfg.bookmarks || [];

      const PALETTE = [
        '#3B82F6', '#10B981', '#F59E0B', '#EF4444',
        '#8B5CF6', '#EC4899', '#14B8A6', '#F97316',
        '#22C55E', '#EAB308',
      ];

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

      // ── Focus zone (frame del focus app) ───────────────────────────────
      // Dimensiones del slot: el shell decide el tamaño, las apps reciben
      // un slot.size. Por ahora fijo.
      const FOCUS_SLOT_W  = 1.0;
      const FOCUS_SLOT_H  = 0.7;
      const FOCUS_SLOT_D  = 0.1;
      const FOCUS_FRAME_THICK = 0.015;
      const FOCUS_TITLEBAR_H  = 0.10;
      const FOCUS_BTN_SIZE   = 0.08;
      const FOCUS_BTN_GAP    = 0.012;

      const SHELL_SPAWN_DISTANCE = 1.5;

      // ── Anchored ───────────────────────────────────────────────────────
      // Offsets laterales para apartar anchored del frente del shell. Stack
      // a la derecha primero, después izquierda; pares para diferenciar idx.
      // local-X (right) en frame del shell — se rota por yaw al world.
      const ANCHORED_BASE_X = 1.15;
      const ANCHORED_STACK_DX = 0.10;
      // Frame anchored = mismo tamaño que focus (la app se renderiza adentro).
      // Si lo achicamos, hay que enviar size distinto en el slot, y la app
      // no escala el contenido auto — fuera del scope de este commit.
      const ANCHORED_SLOT_W = FOCUS_SLOT_W;
      const ANCHORED_SLOT_H = FOCUS_SLOT_H;
      const ANCHORED_SLOT_D = FOCUS_SLOT_D;

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

        // El hitbox lleva la forma y el tamaño del anillo, medio milímetro más
        // atrás: una silueta idéntica y más lejos no asoma por ningún lado.
        const hit = plane(parent, { x: opts.x, y: opts.y, z: Z_HIT, sx: ladoRing,
                                    sy: ladoRing, color, corner: cornerRing, touchable: true });
        const ring = plane(parent, { x: opts.x, y: opts.y, z: Z_RING, sx: ladoRing,
                                     sy: ladoRing, color, corner: cornerRing });
        const face = plane(parent, { x: opts.x, y: opts.y, z: Z_FACE, sx: lado,
                                     sy: lado, color, corner });
        if (opts.glyph) glyph(face, opts.glyph, opts.padding || 0.24);
        if (opts.name) hit.setAttribute('name', opts.name);

        const it = { hit, ring, face, color, x: opts.x || 0, y: opts.y || 0,
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

          const z = it.z - it.golpe;
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

      // ── ShellState ─────────────────────────────────────────────────────
      // State machine global del shell. Cualquier cambio acá DEBE ir seguido
      // de applyVisibility() para refrescar UI.
      const shellState = {
        bookmarksVisible: false,
        // Commits siguientes:
        focusApp: null,    // { tabId, minimized, slot:{position,rotation,size} }
        anchored: [],      // [{ tabId, position, rotation, minimized, alwaysOn }]
      };

      function shouldShowBookmarks() {
        // Bookmarks visibles si shell visible y no hay focus tomando la pantalla.
        if (!shellState.bookmarksVisible) return false;
        if (shellState.focusApp && !shellState.focusApp.minimized && shellState.focusApp.ready) return false;
        return true;
      }

      // Regla literal del doc:
      //   bottom_bar.visible = bookmarks_visible && (focus.is_some() || anchored.len > 0)
      // bookmarksVisible se mantiene `true` durante toda la sesión del shell
      // (incluyendo cuando se abre una app embedded — el panel se oculta solo
      // por la regla de shouldShowBookmarks, pero el flag sigue true). Eso
      // hace que la bar quede visible para volver a marcadores desde una app.
      // La regla del doc era `bookmarks_visible && (focus || anchored)`: la barra
      // existía sólo para volver desde una app. Ahora lleva **estado** —usuario,
      // wifi, batería, avisos, capturas— y eso tiene que estar mientras el shell
      // esté, haya ventanas o no. Con la regla vieja, abrir el menú sin ninguna
      // app mostraba una grilla flotando sin nada abajo.
      function shouldShowBottomBar() {
        return shellState.bookmarksVisible;
      }

      // ── DOM refs ───────────────────────────────────────────────────────
      const panel = cfg.panel;
      const bottomBar = cfg.bottomBar;
      const focusZone = cfg.focusZone;
      const panelBgNode = cfg.panelBg;
      const barBgNode = cfg.barBg;

      // ── Bookmark items refs (para anim cascada) ────────────────────────
      let itemRefs = []; // { box, text, idx }
      let animStartTs = 0;
      let animActive = false;
      let animMode = null;

      let dashboardRefs = [];
      function clearBookmarks() {
        for (const ref of itemRefs) dropPiece(ref.piece);
        for (const node of dashboardRefs) { try { node.remove(); } catch (_) {} }
        dashboardRefs = [];
        itemRefs = [];
      }

      // La grilla: columnas y filas de paso fijo, centradas. **El ancho del
      // panel sale de la grilla y no al revés**; hacerlo al revés obliga a
      // retocar dos números cada vez que se agrega un marcador.
      function gridSlot(i, count) {
        const rows = Math.max(1, Math.ceil(count / GRID_COLS));
        const row = Math.floor(i / GRID_COLS);
        const inRow = Math.min(GRID_COLS, count - row * GRID_COLS);
        return {
          u: (i % GRID_COLS - (inRow - 1) / 2) * GRID_STEP_X,
          y: GRID_Y + ((rows - 1) / 2 - row) * GRID_STEP_Y,
          rows,
        };
      }

      function buildBookmarks() {
        clearBookmarks();
        if (!panel) return;

        const n = BOOKMARKS.length;
        const rows = Math.max(1, Math.ceil(n / GRID_COLS));
        const half = (rows - 1) / 2 * GRID_STEP_Y;
        const gridTop = GRID_Y + half + SIZE_APP / 2;
        const gridBottom = GRID_Y - half + LABEL_DY - LABEL_SIZE;
        const headerY = gridTop + HEADER_DY;

        const header = surface(panel, 0, headerY, R_PANEL);
        dashboardRefs.push(header);
        caption(header, 'Explorar', 0.020, 0.040);
        caption(header, 'Mundos y aplicaciones', -0.036, 0.023, TENUE);

        for (let i = 0; i < n; i++) {
          const bm = BOOKMARKS[i];
          const slot = gridSlot(i, n);
          const tile = surface(panel, slot.u, slot.y, R_PANEL);
          dashboardRefs.push(tile);
          const it = piece(tile, {
            x: 0, y: 0, size: SIZE_APP, corner: CORNER,
            color: PALETTE[i % PALETTE.length],
            glyph: GLYPHS[i] || 'app', padding: 0.24,
            name: 'open-' + bm.name,
            onTap: () => openBookmark(bm),
          });
          const text = caption(tile, bm.name, LABEL_DY, LABEL_SIZE);
          itemRefs.push({ group: tile, piece: it, text, idx: i });
        }

        // El fondo: una sola pastilla curva que cubre el encabezado y la
        // grilla entera.
        const top = headerY + 0.075;
        const bottom = gridBottom - 0.045;
        paintMesh(panelMesh, panelBgNode, [{
          u: 0, y: (top + bottom) / 2, radius: R_PANEL, depth: 0.012,
          width: (GRID_COLS - 1) * GRID_STEP_X + SIZE_APP + 0.16,
          height: top - bottom, color: PLACA, phi: 0,
        }]);
      }

      function applyScale(ref, factor) {
        const f = Math.max(0.001, factor);
        // El grupo entero: icono y etiqueta entran juntos.
        ref.group.scale = { x: f, y: f, z: 1 };
        // Escalar mueve el hitbox, así que mientras dura la cascada no puede
        // recibir nada: el objetivo estaría en otro lado que el dibujo.
        ref.piece.hit.setAttribute('touchable', factor > 0.95 ? 'true' : 'false');
      }

      function startEnterAnimation() {
        for (const ref of itemRefs) applyScale(ref, 0);
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
        const n = itemRefs.length;
        let stillRunning = false;
        for (const ref of itemRefs) {
          if (!ref.group) continue;
          const orderIdx = (animMode === 'exit') ? (n - 1 - ref.idx) : ref.idx;
          const itemElapsed = elapsed - orderIdx * STAGGER_MS;
          if (itemElapsed < 0) {
            applyScale(ref, animMode === 'enter' ? 0 : 1);
            stillRunning = true;
            continue;
          }
          if (itemElapsed >= ANIM_MS) {
            applyScale(ref, animMode === 'enter' ? 1 : 0);
            continue;
          }
          const t = itemElapsed / ANIM_MS;
          const progress = animMode === 'enter' ? easeOutBack(t) : (1 - easeInBack(t));
          applyScale(ref, progress);
          stillRunning = true;
        }
        if (stillRunning) {
          requestAnimationFrame(tickAnim);
        } else {
          animActive = false;
          if (animMode === 'exit' && panel) {
            panel.setAttribute('visible', 'false');
          }
          animMode = null;
        }
      }

      // ── La barra ───────────────────────────────────────────────────────
      // Tres pastillas, no una sola de punta a punta: una pastilla de 1,2 m
      // sobre un cilindro habría que segmentarla, y partida en tres la
      // curvatura no se nota y además queda dicho qué cosa va con qué.
      //
      //   [ usuario · wifi · batería ]  [ avisos · capturas ]  [ inicio · apps ]
      //
      // Los dos primeros grupos son fijos; el tercero crece con las ventanas
      // abiertas. Todos los botones son redondos y del mismo lado: un ícono se
      // reconoce de un vistazo, y las etiquetas de texto que había antes no se
      // leían a esta distancia y obligaban a hacer los slots anchos.
      let barPieces = [];      // piezas tocables vivas
      let barNodes = [];       // grupos y textos, para poder barrer
      let barSignature = null; // qué composición está dibujada
      let barPage = 0;
      const BAR_APPS_PER_PAGE = 4;

      function clearBar() {
        for (const it of barPieces) dropPiece(it);
        for (const node of barNodes) { try { node.remove(); } catch (_) {} }
        barPieces = [];
        barNodes = [];
      }

      // Centros de una fila de anchos iguales, centrada en cero.
      function spread(count, step) {
        const out = [];
        for (let i = 0; i < count; i++) out.push((i - (count - 1) / 2) * step);
        return out;
      }

      function barButton(parent, u, opts) {
        const group = surface(parent, u, BAR_Y, R_BAR, BAR_TILT);
        barNodes.push(group);
        if (opts.glyph && !opts.onTap) {
          // Indicador de estado: no se toca, así que no es una pieza. Un anillo
          // que se enciende en algo que no responde miente.
          const node = plane(group, { x: 0, y: 0, z: Z_FACE, sx: opts.size || BAR_ITEM,
                                      sy: opts.size || BAR_ITEM,
                                      color: opts.color || PLACA_ALTA, corner: 0.5 });
          glyph(node, opts.glyph, 0.28);
          return null;
        }
        const it = piece(group, {
          x: 0, y: 0, size: opts.size || BAR_ITEM, corner: 0.5,
          color: opts.color || PLACA_ALTA,
          glyph: opts.glyph, padding: 0.26,
          name: opts.name, onTap: opts.onTap,
        });
        barPieces.push(it);
        return it;
      }

      function rebuildBottomBar() {
        if (!bottomBar) return;

        // Qué ventanas hay. El grupo tres es inicio + una por ventana.
        const apps = [];
        if (shellState.focusApp) {
          apps.push({
            key: 'focus', dim: !!shellState.focusApp.minimized,
            title: shellState.focusApp.ready ? (shellState.focusApp.title || 'App') : 'Cargando',
            onTap: onTapBarFocus, onClose: onTapFocusClose,
          });
        }
        for (const a of shellState.anchored) {
          apps.push({
            key: 'anc:' + a.tabId, dim: !!a.minimized, title: a.title || 'App',
            onTap: () => onTapBarAnchored(shellState.anchored.indexOf(a)),
            onClose: () => onTapBarAnchoredClose(shellState.anchored.indexOf(a)),
          });
        }

        const pages = Math.max(1, Math.ceil(apps.length / BAR_APPS_PER_PAGE));
        barPage = Math.max(0, Math.min(barPage, pages - 1));
        const shown = apps.slice(barPage * BAR_APPS_PER_PAGE, (barPage + 1) * BAR_APPS_PER_PAGE);

        // **Sólo se redibuja si la composición cambió.** `applyVisibility()` se
        // llama en cada transición de estado, y rehacer los nodos en cada una
        // hacía parpadear la barra entera.
        const signature = [barPage, pages].concat(shown.map(a => a.key + (a.dim ? ':m' : ''))).join('|');
        if (signature === barSignature) return;
        barSignature = signature;
        clearBar();

        // Grupo 1: estado. Grupo 2: avisos. Grupo 3: inicio + ventanas.
        const g1 = ['user', 'wifi', 'battery'];
        const g2 = ['bell', 'capture'];
        const g3n = 1 + shown.length + (pages > 1 ? 1 : 0);
        const counts = [g1.length, g2.length, g3n];
        const widths = counts.map(n => n * BAR_ITEM + (n - 1) * BAR_GAP + BAR_PAD);

        // Centro de cada pastilla, centradas entre sí.
        const totalW = widths.reduce((a, b) => a + b, 0) + (widths.length - 1) * BAR_GROUP_GAP;
        const centers = [];
        let cursor = -totalW / 2;
        for (const w of widths) { centers.push(cursor + w / 2); cursor += w + BAR_GROUP_GAP; }

        // Los fondos, en una malla: la barra se inclina hacia arriba, y `phi` es
        // el ángulo con que la malla la levanta —el opuesto del rx del nodo.
        paintMesh(barMesh, barBgNode, widths.map((w, i) => ({
          u: centers[i], y: BAR_Y, radius: R_BAR, depth: 0.012, width: w,
          height: BAR_H, color: PLACA, phi: -BAR_TILT,
        })));

        const place = (gi, j, n) => centers[gi] + spread(n, BAR_ITEM + BAR_GAP)[j];

        for (let j = 0; j < g1.length; j++) {
          barButton(bottomBar, place(0, j, g1.length), { glyph: g1[j], color: PLACA_ALTA });
        }
        barButton(bottomBar, place(1, 0, g2.length), { glyph: 'bell', color: PLACA_ALTA, onTap: () => {} });
        barButton(bottomBar, place(1, 1, g2.length), { glyph: 'capture', color: PLACA_ALTA, onTap: () => {} });

        let j = 0;
        barButton(bottomBar, place(2, j++, g3n), {
          glyph: 'home', color: '#30435F', name: 'bar-menu', onTap: onTapBarMenu,
        });
        for (const app of shown) {
          const u = place(2, j++, g3n);
          barButton(bottomBar, u, {
            glyph: 'app', color: app.dim ? PLACA_ALTA : '#304F68',
            name: 'bar-' + app.key, onTap: app.onTap,
          });
          // Cerrar: un punto chico arriba a la derecha del botón, no un cuadro
          // rojo del ancho de un slot.
          const closeGroup = surface(bottomBar, u + BAR_ITEM * 0.42, BAR_Y + BAR_ITEM * 0.42, R_BAR, BAR_TILT);
          barNodes.push(closeGroup);
          const closeIt = piece(closeGroup, {
            x: 0, y: 0, z: 0, size: 0.030, corner: 0.5, color: '#B0413E',
            name: 'close-' + app.key, onTap: app.onClose,
          });
          barPieces.push(closeIt);
        }
        if (pages > 1) {
          barButton(bottomBar, place(2, j++, g3n), {
            glyph: 'range', color: PLACA_ALTA, name: 'bar-page',
            onTap: () => { barPage = (barPage + 1) % pages; barSignature = null; rebuildBottomBar(); },
          });
        }
      }

      // ── Bar handlers ───────────────────────────────────────────────────
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
        // bar Menu tap sin focus → no-op (ya estamos en bookmarks).
      }

      function onTapBarFocus() {
        if (!shellState.focusApp || !shellState.focusApp.ready) return;
        shellState.focusApp.minimized = false;
        // bookmarksVisible queda true (sesión sigue activa). Las reglas de
        // visibilidad ocultan panel automáticamente porque focus !minimized.
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
        // Suspend/resume al app — esperan el render de visibilidad para
        // que la app pueda pausar loops antes de que el frame desaparezca.
        sendMessageToApp(a.tabId, a.minimized ? 'suspend' : 'resume');
        applyVisibility();
      }

      function onTapBarAnchoredClose(idx) {
        const a = shellState.anchored[idx];
        if (!a) return;
        unmountAnchored(idx);
      }

      // ── applyVisibility — derive UI from ShellState ────────────────────
      function applyVisibility() {
        if (panel)     panel.setAttribute('visible', shouldShowBookmarks() ? 'true' : 'false');
        if (bottomBar) bottomBar.setAttribute('visible', shouldShowBottomBar() ? 'true' : 'false');
        rebuildBottomBar();

        // Focus zone (frame): visible si hay focus no-minimizado Y el frame
        // ya está construido (focusApp.tabId > 0). Esto evita el parpadeo de
        // un focusZone vacío durante los ~100ms entre openEmbeddedApp() y la
        // llegada del primer mensaje de la app que dispara buildFocusFrame.
        const showFocus = shellState.focusApp
          && !shellState.focusApp.minimized
          && shellState.focusApp.tabId > 0 && shellState.focusApp.ready;
        if (focusZone) focusZone.setAttribute('visible', showFocus ? 'true' : 'false');
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

      // Regla del doc:
      //   anchored_i.visible = !minimized && (alwaysOn || bookmarks_visible)
      // Frame sub-group + tab setVisible siguen la misma regla. Mantenemos
      // cache `lastVisible` por entry para emitir suspend/resume sólo en
      // transiciones, no en cada applyVisibility.
      function applyAnchoredVisibility() {
        for (let i = 0; i < shellState.anchored.length; i++) {
          const a = shellState.anchored[i];
          // Build frame on demand (sólo la primera vez tras anchor).
          ensureAnchoredFrame(a, i);
          updateAnchoredFramePose(a);

          const visible = !a.minimized && (a.alwaysOn || shellState.bookmarksVisible);
          if (a.frame && a.frame.group) {
            a.frame.group.setAttribute('visible', visible ? 'true' : 'false');
          }
          if (a.tabId > 0) tabVisible(a.tabId, visible);
          if (a._lastVisible !== visible) {
            // Suspend/resume sólo en transición; sin esto, cada applyVisibility
            // bombardearía al worker con el mismo mensaje.
            if (a.tabId > 0) {
              sendMessageToApp(a.tabId, visible ? 'resume' : 'suspend');
            }
            a._lastVisible = visible;
          }
        }
      }

      // ── Posicionar shell al frente del usuario ─────────────────────────
      function repositionShellAtViewer() {
        const pose = viewerPose();
        if (!pose) return;

        const fwdX = -Math.sin(pose.yaw);
        const fwdZ = -Math.cos(pose.yaw);

        // Mover ambos grupos juntos. Como son hijos del mismo space, podríamos
        // mover el space entero, pero por ahora movemos cada group para que el
        // commit siguiente pueda darle a focus/anchored una pose distinta.
        const px = pose.px + fwdX * SHELL_SPAWN_DISTANCE;
        const py = pose.py;
        const pz = pose.pz + fwdZ * SHELL_SPAWN_DISTANCE;
        const ry = pose.yaw;

        if (panel) {
          panel.position = { x: px, y: py, z: pz };
          panel.rotation = { x: 0, y: ry, z: 0 };
        }
        if (bottomBar) {
          bottomBar.position = { x: px, y: py, z: pz };
          bottomBar.rotation = { x: 0, y: ry, z: 0 };
        }
        if (focusZone) {
          focusZone.position = { x: px, y: py, z: pz };
          focusZone.rotation = { x: 0, y: ry, z: 0 };
        }
        // Si hay focus visible, re-enviar slot a la app (la pose cambió).
        if (shellState.focusApp && !shellState.focusApp.minimized && shellState.focusApp.tabId > 0) {
          sendFocusSlotToApp();
        }

        // Anchored !alwaysOn arrastran con el shell — recalc world pose desde
        // offset local y re-send slot. alwaysOn quedan fijos en world.
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

      // ── Toggle shell (menu press del controller) ───────────────────────
      // Transiciones del doc (docs/embedded_apps.md sección "Transiciones"):
      //   focus visible (!minimized)     → focus.minimized=true, bookmarks visible.
      //   shell hidden (algo visible)    → bookmarks visible, bar visible, anchored restaurados.
      //   bookmarks visible (sin focus)  → bookmarks hidden, bar hidden, anchored !always_on ocultos.
      //
      // `bookmarksVisible` es el flag de sesión: true mientras cualquier
      // pieza del shell sea visible o esté minimizada-pero-vigente. Volverá
      // a false sólo cuando el user oculta todo via menu press.
      function toggleShell() {
        // Branch: focus visible → minimizar focus, mostrar bookmarks. (= 🏠)
        if (shellState.focusApp && !shellState.focusApp.minimized) {
          shellState.focusApp.minimized = true;
          if (shellState.focusApp.tabId > 0) {
            tabVisible(shellState.focusApp.tabId, false);
            sendMessageToApp(shellState.focusApp.tabId, 'blur');
          }
          // bookmarksVisible ya era true (sesión activa con focus). Idempotent.
          applyVisibility();
          startEnterAnimation();
          return;
        }

        // Branch: simple toggle on/off.
        const wasActive = shellState.bookmarksVisible;
        shellState.bookmarksVisible = !wasActive;

        if (!wasActive) {
          // off → on: reposicionar al frente del user (sesión nueva).
          repositionShellAtViewer();
          applyVisibility();
          startEnterAnimation();
        } else {
          // on → off: cerrar sesión. Exit anim.
          applyVisibility();
          startExitAnimation();
        }
      }

      // ── Frame del focus (titlebar + border) ────────────────────────────

      // ── El marco de una ventana ────────────────────────────────────────
      // Lo usan por igual la ventana con foco y las fijadas: son la misma cosa
      // en distinto lugar, y tenerlo dos veces escrito fue lo que las dejó
      // divergir. El marco es sólo el cuadro; el contenido lo dibuja la app en
      // su propio espacio, ubicado por el slot que le manda el shell.
      //
      // Un marco NO va sobre el arco: es una superficie plana que se mira de
      // frente. La curva es para el menú, que rodea al visitante.
      const WIN_TITLEBAR = 0.075;
      const WIN_EDGE     = 0.010;   // cuánto asoma el borde alrededor
      const WIN_BTN      = 0.046;
      const WIN_BTN_GAP  = 0.013;
      const WIN_MARGIN   = 0.030;   // del último botón al filo
      const WIN_CORNER   = 0.026;   // fracción del lado

      function buildWindowFrame(parent, opts) {
        const W = opts.width, H = opts.height, T = WIN_TITLEBAR;
        const nodes = [];
        const pieces = [];
        const add = (node) => { nodes.push(node); return node; };

        // Borde: un plano detrás y apenas más grande, igual que el anillo de un
        // icono. Es lo único que separa la ventana del mundo cuando el fondo es
        // oscuro, y por eso no puede ser del color de la ventana.
        add(plane(parent, { x: 0, y: 0, z: -0.006,
                            sx: W + WIN_EDGE * 2, sy: H + T + WIN_EDGE * 2,
                            color: '#39445A', corner: WIN_CORNER }));
        add(plane(parent, { x: 0, y: 0, z: -0.004, sx: W + WIN_EDGE * 0.6,
                            sy: H + T + WIN_EDGE * 0.6, color: PLACA, corner: WIN_CORNER }));

        // Barra de título y hueco del contenido. La barra NO lleva placa propia:
        // dibujar un rectángulo más claro encima del fondo daba dos barras
        // apiladas, una dentro de la otra, y el borde se leía dos veces. Lo que
        // la separa del contenido es el hueco negro de abajo, nada más.
        const titleY = H / 2 + T / 2;
        add(plane(parent, { x: 0, y: 0, z: -0.003, sx: W, sy: H,
                            color: '#0E1116', corner: 0.02 }));

        // Los botones ocupan la derecha de la barra; el título se centra en lo
        // que sobra, no en la barra entera, porque si no queda debajo de ellos.
        const btnBlock = 3 * WIN_BTN + 2 * WIN_BTN_GAP;
        const btnLeft = W / 2 - WIN_MARGIN - btnBlock;

        const title = root.createElement('text');
        title.setAttribute('value', windowTitle(opts.title));
        title.setAttribute('size', '0.028');
        title.setAttribute('color', TENUE);
        title.setAttribute('touchable', 'false');
        title.position = { x: (-W / 2 + btnLeft) / 2, y: titleY, z: 0.006 };
        parent.appendChild(title);
        nodes.push(title);

        // Redondos y con glifo: los cuadrados de color con una letra adentro no
        // se leían a esta distancia y ocupaban media barra.
        let bx = W / 2 - WIN_MARGIN - WIN_BTN / 2;
        const button = (glyphName, color, onTap) => {
          const it = piece(parent, {
            x: bx, y: titleY, size: WIN_BTN, corner: 0.5,
            color, glyph: glyphName, padding: 0.28, onTap,
          });
          bx -= WIN_BTN + WIN_BTN_GAP;
          pieces.push(it);
          return it;
        };

        button('close', '#B0413E', opts.onClose);
        button('minimize', PLACA_ALTA, opts.onMinimize);
        const anchorBtn = opts.onAnchor ? button('anchor', opts.anchorColor || PLACA_ALTA, opts.onAnchor) : null;

        return { nodes, pieces, title, anchorBtn };
      }

      function clearWindowFrame(frame) {
        if (!frame) return;
        for (const it of frame.pieces) dropPiece(it);
        for (const node of frame.nodes) { try { node.remove(); } catch (_) {} }
        frame.pieces = [];
        frame.nodes = [];
      }

      // ── Marco del focus ────────────────────────────────────────────────
      let focusFrame = null;

      function clearFocusFrame() {
        clearWindowFrame(focusFrame);
        focusFrame = null;
      }

      function windowTitle(title) {
        const value = title || 'App';
        return value.length > 24 ? value.slice(0, 23) + '…' : value;
      }

      function buildFocusFrame(title) {
        clearFocusFrame();
        if (!focusZone) return;
        focusFrame = buildWindowFrame(focusZone, {
          width: FOCUS_SLOT_W, height: FOCUS_SLOT_H, title,
          onClose: onTapFocusClose,
          onMinimize: onTapFocusMinimize,
          onAnchor: onTapFocusAnchor,
          anchorColor: '#30435F',
        });
      }

      // ── Slot pose computation ──────────────────────────────────────────
      // Posición world del focus slot = pose del shell + offset al frente.
      // El frame visual (focusZone) está local al space ux_vr, así que el
      // shell mueve focusZone en local coords (mismo origen del panel).
      // La pose del slot del focus es la MISMA pose que el shell.
      // Ambos comparten posición/rotación porque el frame se renderiza junto
      // a los bookmarks. Leemos del panel, que es el source-of-truth de la
      // pose del shell — se actualiza solo en `repositionShellAtViewer` cuando
      // el menú aparece. Si abriéramos una app y leyéramos del viewer en
      // tiempo real, el shell se reposicionaría según la cabeza del user en
      // ese instante → la app + frame saltarían respecto al menú visible.
      function computeFocusSlotPose() {
        const fallback = {
          position: { x: 0, y: 1.5, z: -1.5 },
          rotation: { x: 0, y: 0, z: 0 },
          size: { x: FOCUS_SLOT_W, y: FOCUS_SLOT_H, z: FOCUS_SLOT_D },
        };
        if (!panel) return fallback;
        let p, r;
        try { p = panel.position; } catch (_) { return fallback; }
        try { r = panel.rotation; } catch (_) { return fallback; }
        return {
          position: { x: Number(p.x || 0), y: Number(p.y || 0), z: Number(p.z || 0) },
          rotation: { x: Number(r.x || 0), y: Number(r.y || 0), z: Number(r.z || 0) },
          size: { x: FOCUS_SLOT_W, y: FOCUS_SLOT_H, z: FOCUS_SLOT_D },
        };
      }

      // The host follows the actual frame entity every frame. Apps receive
      // local coordinates; a busy app worker cannot leave content behind.
      let nextSlotRevision = 0;
      function sendWindowSlot(entry, frame, size) {
        if (!entry || entry.tabId <= 0 || !frame) return;
        const revision = ++nextSlotRevision;
        entry.slotRevision = revision;
        frame._onResolved((anchorNodeId) => {
          if (entry.slotRevision !== revision ||
              (shellState.focusApp !== entry && !shellState.anchored.includes(entry))) return;
          sendMessageToApp(entry.tabId, 'slot', {
            coordinateSpace: 'window-local', anchorNodeId, revision,
            position: { x: 0, y: 0, z: 0 }, rotation: { x: 0, y: 0, z: 0 }, size,
          });
        });
      }

      function sendFocusSlotToApp() {
        sendWindowSlot(shellState.focusApp, focusZone,
          { x: FOCUS_SLOT_W, y: FOCUS_SLOT_H, z: FOCUS_SLOT_D });
      }

      function sendMessageToApp(tabId, type, extra) {
        const payload = JSON.stringify(Object.assign({ type }, extra || {}));
        send(tabId, payload);
      }

      // ── Handlers titlebar focus ────────────────────────────────────────
      function onTapFocusClose() {
        if (!shellState.focusApp) return;
        const tabId = shellState.focusApp.tabId;
        if (tabId <= 0) {
          shellState.focusApp.cancelled = true;
          shellState.focusApp.supersededBy = null;
          shellState.focusApp.minimized = true;
          shellState.bookmarksVisible = true;
          applyVisibility();
          return;
        }
        // Notificar a la app antes de unmount (oportunidad de cleanup).
        sendMessageToApp(tabId, 'close');
        // Unmount inmediato; la app pierde su isolate (worker shutdown).
        // TODO(commit watchdog): esperar ack o timeout 500ms.
        tabClose(tabId);
        shellState.focusApp = null;
        shellState.bookmarksVisible = true;
        applyVisibility();
      }

      function onTapFocusMinimize() {
        if (!shellState.focusApp) return;
        shellState.focusApp.minimized = true;
        shellState.bookmarksVisible = true;
        // Ocultar el tab outer wrapper — los hijos (Inherited) heredan.
        tabVisible(shellState.focusApp.tabId, false);
        sendMessageToApp(shellState.focusApp.tabId, 'blur');
        applyVisibility();
      }

      function onTapFocusAnchor() {
        if (!shellState.focusApp) return;
        const tabId = shellState.focusApp.tabId;
        if (tabId <= 0) return; // app aún sin tabId — esperar 'ready'.

        // Offset lateral derecho con stack para que dos anchors consecutivos
        // no se monten uno sobre otro. Coords locales al frame del shell;
        // se rotan por yaw para world. El doc no especifica layout, esto es
        // pragmático para commit 4 — drag (commit 5) le dará pose final.
        const idx = shellState.anchored.length;
        const offsetLocal = {
          x: ANCHORED_BASE_X + idx * ANCHORED_STACK_DX,
          y: 0,
          z: 0,
        };
        const worldPose = computeAnchoredWorldPose(offsetLocal);

        const entry = {
          tabId,
          title: shellState.focusApp.title || 'App',
          offset: offsetLocal,  // local → recalculado cuando shell se mueve.
          position: worldPose.position,
          rotation: worldPose.rotation,
          minimized: false,
          alwaysOn: false,
          frame: null,          // refs DOM, populadas por ensureAnchoredFrame.
        };
        shellState.anchored.push(entry);

        // App perdió focus (pasa a anchored). El doc enumera 'blur' cuando
        // pasa a anchored o minimized. Acá es el caso anchored.
        sendMessageToApp(tabId, 'blur');

        // Limpiar focus state ANTES de re-enviar slot — sendAnchoredSlot lee
        // del entry recién pusheado, no del focus.
        shellState.focusApp = null;
        clearFocusFrame();

        // Nueva pose para la app — ya no está en el slot focus.
        sendAnchoredSlotToApp(shellState.anchored.length - 1);

        applyVisibility();
      }

      // ── Anchored frame + pose ──────────────────────────────────────────
      // Cada anchored vive en su propio sub-group dentro de ux_vr_anchored_zone.
      // El sub-group lleva la pose (position/rotation world); sus hijos
      // (borde, titlebar, botones) son locales al sub-group.
      const anchoredZone = cfg.anchoredZone;

      function computeAnchoredWorldPose(offsetLocal) {
        // Rota offset por yaw del shell y suma a la pose del shell (panel).
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

      // Una ventana fijada es el mismo marco que la del foco, en su propia pose
      // world. Antes eran dos constructores copiados, y por eso divergieron: el
      // botón de fijar decía «Siempre» acá y «Fijar» allá, con dos tamaños de
      // letra distintos.
      function ensureAnchoredFrame(anc, idx) {
        if (anc.frame) return anc.frame;
        if (!anchoredZone) return null;

        const group = root.createElement('group');
        group.position = anc.position;
        group.rotation = anc.rotation;
        anchoredZone.appendChild(group);

        const win = buildWindowFrame(group, {
          width: ANCHORED_SLOT_W, height: ANCHORED_SLOT_H, title: anc.title,
          onClose: () => onTapAnchoredClose(shellState.anchored.indexOf(anc)),
          onMinimize: () => onTapAnchoredMinimize(shellState.anchored.indexOf(anc)),
          onAnchor: () => onTapAnchoredAlwaysOn(shellState.anchored.indexOf(anc)),
          anchorColor: anc.alwaysOn ? '#2F7D5B' : PLACA_ALTA,
        });

        anc.frame = { group, win, titleText: win.title, aoPiece: win.anchorBtn };
        return anc.frame;
      }

      function clearAnchoredFrame(anc) {
        if (!anc || !anc.frame) return;
        clearWindowFrame(anc.frame.win);
        // Quitar el group entero arrastra lo que quede colgando.
        try { if (anc.frame.group) anc.frame.group.remove(); } catch (_) {}
        anc.frame = null;
      }

      function updateAnchoredFramePose(anc) {
        if (!anc || !anc.frame || !anc.frame.group) return;
        anc.frame.group.position = anc.position;
        anc.frame.group.rotation = anc.rotation;
      }

      // Verde cuando la ventana se queda quieta en el mundo aunque el menú se
      // vaya; el color de fondo de la barra cuando sigue al shell.
      function updateAnchoredFrameAOColor(anc) {
        if (!anc || !anc.frame || !anc.frame.aoPiece) return;
        recolorPiece(anc.frame.aoPiece, anc.alwaysOn ? '#2F7D5B' : PLACA_ALTA);
      }

      function sendAnchoredSlotToApp(idx) {
        const a = shellState.anchored[idx];
        if (!a || a.tabId <= 0) return;
        ensureAnchoredFrame(a, idx);
        sendWindowSlot(a, a.frame && a.frame.group,
          { x: ANCHORED_SLOT_W, y: ANCHORED_SLOT_H, z: ANCHORED_SLOT_D });
      }

      // ── Anchored titlebar handlers ────────────────────────────────────
      function onTapAnchoredClose(idx) {
        unmountAnchored(idx);
      }

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
        // alwaysOn=true: la pose queda FIJA en world. Las recomputaciones
        // por shell move dejan de tocarla.
        // alwaysOn=false: vuelve a seguir al shell — recalcular offset
        // relativo a la pose CURRENT del shell para que no salte.
        if (!a.alwaysOn) {
          const localOff = worldOffsetFromShell(a.position);
          if (localOff) a.offset = localOff;
        }
        updateAnchoredFrameAOColor(a);
        applyVisibility();
      }

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
        // Inversa de la rotación (yaw -> -yaw).
        return {
          x:  dx * cos - dz * sin,
          y:  dy,
          z:  dx * sin + dz * cos,
        };
      }

      function unmountAnchored(idx) {
        const a = shellState.anchored[idx];
        if (!a) return;
        const tabId = a.tabId;
        sendMessageToApp(tabId, 'close');
        tabClose(tabId);
        clearAnchoredFrame(a);
        shellState.anchored.splice(idx, 1);
        applyVisibility();
      }

      // ── Open embedded app ──────────────────────────────────────────────
      // Diferencia de openBookmark (apps spatial/app normales):
      //   - No cierra bookmarks de inmediato.
      //   - Setea focusApp en ShellState.
      //   - Cuando la app pida 'requestSlot', shell le manda el slot y oculta bookmarks.
      function openEmbeddedApp(bm) {
        shellState.bookmarksVisible = true;
        // El shell mantiene su pose actual (la que tenía cuando aparecieron
        // los bookmarks). NO re-leemos viewer pose acá: si el user movió la
        // cabeza entre abrir el menú y clickear la app, el shell se saltaría
        // visualmente — todo el menú parpadearía al moverse. El frame del
        // focus aparece exactamente sobre la posición del shell visible.
        if (shellState.focusApp && shellState.focusApp.tabId === 0) {
          shellState.focusApp.cancelled = true;
          shellState.focusApp.supersededBy = bm;
          return;
        }
        if (shellState.focusApp) {
          sendMessageToApp(shellState.focusApp.tabId, 'close');
          tabClose(shellState.focusApp.tabId);
          shellState.focusApp = null;
        }

        // Identity arrives from the host's tabopened acknowledgement.
        // App messages cannot identify or reveal a pending window.
        shellState.focusApp = {
          tabId: 0,        // pending until host acknowledgement
          minimized: false,
          ready: false,
          title: bm.name,
          pendingUrl: bm.url,
        };
        tabOpen(bm.url, { kind: 'app-embedded' });

        // Mantenemos bookmarksVisible=true: la sesión del shell sigue activa.
        // Las reglas derivadas (shouldShowBookmarks / shouldShowBottomBar) hacen
        // el resto — panel se oculta porque focus está visible, bar queda
        // visible para volver a marcadores. Modelo literal del doc:
        //   panel = bookmarks_visible && (no focus || focus.minimized)
        //   bar   = bookmarks_visible && (focus.is_some() || anchored.len > 0)
        applyVisibility();
      }

      // ── Bookmark click ─────────────────────────────────────────────────
      function openBookmark(bm) {
        if (typeof tabs.open !== 'function') {
          console.warn('[shell_app] sin pestañas — navegando directo');
          navigate(bm.url);
          return;
        }

        if (bm.kind === 'app-embedded') {
          openEmbeddedApp(bm);
          return;
        }

        // Spatial / app normal — comportamiento previo.
        tabOpen(bm.url, { kind: bm.kind || 'spatial' });
        shellState.bookmarksVisible = false;
        applyVisibility();
        startExitAnimation();
      }

      // ── Mensaje recibido de una app embedded ───────────────────────────
      function handleAppMessage(msg) {
        const fromTabId = Number(msg.fromTabId || 0);
        let parsed = null;
        try { parsed = JSON.parse(msg.payload); } catch (_) {}
        if (!parsed || typeof parsed !== 'object') return;
        const type = String(parsed.type || '').toLowerCase();

        // Only the host assigns identity. Never let an unrelated/late app
        // claim a pending focus window just by sending its first message.
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
          // Reply only to the requesting app, preserving its focus/anchor slot.
          if (shellState.focusApp && shellState.focusApp.tabId === fromTabId) {
            sendFocusSlotToApp();
          } else {
            const idx = shellState.anchored.findIndex(a => a.tabId === fromTabId);
            if (idx >= 0) sendAnchoredSlotToApp(idx);
          }
        } else if (type === 'ready') {
          // Readiness is handled above, after matching the host-assigned ID.
        } else if (type === 'closeself') {
          if (shellState.focusApp && shellState.focusApp.tabId === fromTabId) {
            onTapFocusClose();
          } else {
            const idx = shellState.anchored.findIndex(a => a.tabId === fromTabId);
            if (idx >= 0) unmountAnchored(idx);
          }
        }
      }

      // Poll loop: drena inbox del shell cada 16ms.
      function pollShellInbox() {
        const msgs = poll();
        if (Array.isArray(msgs) && msgs.length) {
          for (const m of msgs) handleAppMessage(m);
        }
        setTimeout(pollShellInbox, 16);
      }
      pollShellInbox();


      // ── Arranque ───────────────────────────────────────────────────────
      buildBookmarks();
      applyVisibility();

      // Lo que el controller necesita para cablear su botón de menú y, si
      // hace falta, empujar el shell desde afuera.
      // La superficie pública del módulo. Es más ancha que lo que un controller
      // necesita a propósito: las pruebas manejan el shell desde afuera —abrir
      // una app, fingir el reconocimiento del host, fijarla, restaurarla— y sin
      // esto tendrían que recortar el archivo por marcadores de texto para
      // alcanzar sus funciones, que es exactamente lo que las rompía cada vez
      // que algo se movía de lugar.
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
        barVisible: shouldShowBottomBar,
        rebuild: rebuildBottomBar,
        items: () => itemRefs,
        bar: () => barPieces,
        focusSlotPose: computeFocusSlotPose,
      };

      }

  globalThis.ShellApp = { mount: mount };
  console.log('[shell_app] listo');
})();
