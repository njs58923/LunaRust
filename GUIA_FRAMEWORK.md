# Guía del framework Luna (HSML + JS)

Referencia práctica para **escribir documentos espaciales** que corran en Luna.
Todo lo de acá está verificado contra el código, no contra el README.

Fuentes de verdad:

| Tema | Archivo |
|---|---|
| Tags válidos | `crates/virtual_dom/src/dom/tags.rs` |
| Atributos y render de cada tag | `crates/luna/src/dom.rs` |
| API JS completa | `crates/js_runtime/runtime.js` |
| Permisos / bundles | `crates/luna/src/permissions.rs` |
| Eventos (raycast) | `crates/luna/src/touch.rs` |
| Páginas internas de ejemplo | `crates/luna/src/routes.rs` |

---

## 0. Advertencias — leer esto primero

Verificadas contra el motor construyendo tres páginas remotas completas
(exposición de assets, backrooms procedurales, banco de pruebas de carga). Las
tres primeras estaban **mal documentadas acá**; el resto faltaba.

**Los `resources` van separados por COMA.** `parse_resource_tokens` hace
`split(',')` a secas. Con espacios queda un único token que no matchea ningún
bundle, el documento se queda **sin capacidades y nadie avisa**. Esta guía los
mostraba separados por espacios; ya está corregido abajo, pero si copiaste un
ejemplo viejo, revisalo. Un día entero de creer que `readViewerPose` estaba roto.

**`<meta type="position">` no hace nada.** Aparece en todos los documentos,
incluidos los internos, pero ningún código del motor lo lee. Para mover un
documento al montarlo, envolvé el contenido en un `<group>` con transform.

**`fetch` no le llega a una página remota.** La capacidad `fetch_text` existe en
`permissions.rs`, pero el shell le concede a una página `spatial` sólo
`navigate_self`, `read_pose_stream`, `read_camera_pose` y `skybox`
(`web/internal/root_api.js`), y las capacidades efectivas se intersecan con eso.
Pedirla en `resources` no alcanza. Para traer datos de tu propio server, hoy el
único camino es `<include>`, que lo resuelve el motor.

**`<include>` no emite `load` ni `error`.** No hay forma de saber desde el
documento si un sub-documento terminó de montarse o falló. Si tu código marca
algo como "montado" al crear el nodo, un fallo lo deja registrado como presente
para siempre.

**`touchable` no tiene default permisivo.** `node_touchable()` devuelve `false`
cuando el atributo falta: sin `touchable="true"` explícito, un nodo con `id` y
listener no recibe nada. Ver `home.hsml`, que lo pone en todos sus botones.

**El `<text>` se centra en `x` y también en `y`.** La guía sólo habla del ancho;
la coordenada que le des es el centro de la caja, no su esquina ni su línea base.

---

## 1. Modelo mental

Un **documento HSML** describe una escena 3D. Sus `<script>` la hacen interactiva.
El documento vive dentro de un **espacio** (`<space>`), que es la unidad de
navegación y de permisos — el equivalente a una pestaña.

Cada frame el host hace, en orden estricto:

1. Drena la cola de mutaciones DOM que encoló el JS (crear, mover, borrar, atributos).
2. Corre el tick de JS (timers, `requestAnimationFrame`, callbacks de eventos).
3. Sincroniza el DOM con el ECS de Bevy y renderiza.

JS nunca bloquea al render y viceversa: la comunicación es por colas + snapshots.
Consecuencia práctica: `createElement()` devuelve un elemento **pendiente**
(`nodeId < 0`) que se resuelve unos ticks después. No hace falta esperar — las
llamadas a `setAttribute`, `position`, etc. sobre un pendiente se encolan y se
aplican al resolverse. Sólo los *getters* devuelven valores cacheados hasta entonces.

---

## 2. Anatomía de un documento

```xml
<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Mi espacio</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="rotation" x="0" y="0" z="0"/>
    <meta type="scale"    x="1" y="1" z="1"/>
  </head>

  <space resources="navigate_self,skybox">
    <!-- contenido -->
    <script src="app.js"/>
  </space>
</hsml>
```

- `<head>` no se renderiza. `<name>` es el título. Los `<meta type="...">` son
  **decorativos**: ningún código del motor los lee, pese a lo que decía esta guía.
  Para mover el documento, usá un `<group>` con transform alrededor del contenido.
- `<space>` es la raíz visible y **donde se declaran los permisos** (`resources`).
- `<script src="...">` se resuelve relativo a la URL del documento; también acepta
  `luna://internal/*.js`. Corre en el isolate del espacio.

### Tags válidos

De `tags.rs`, la lista completa:

| Tag | Qué es |
|---|---|
| `hsml`, `head`, `name`, `meta`, `state` | estructura / metadatos |
| `script` | carga y ejecuta JS |
| `space` | raíz del espacio, portadora de `resources` |
| `include` | embebe otro documento HSML (sub-documento, permisos propios) |
| `group` (alias `div`) | nodo de agrupación / transform |
| `box`, `sphere`, `cylinder`, `plane` | primitivas |
| `text` | texto renderizado a textura sobre un plano |
| `model` | malla glTF/GLB |
| `skybox` | cubemap de fondo (requiere permiso) |
| `posezone` | volumen invisible que emite eventos `posemove` de manos/mandos |
| `image` | **parsea pero hoy no renderiza** — cae al caso estructural |

> `runtime.js` define además `HSMLButtonElement` y `HSMLVideoElement`, pero los tags
> `button` y `video` no existen en `tags.rs` ni en `dom.rs`: son vestigios de la capa
> de compat con Unity. No los uses.

---

## 3. Atributos

### Transformación (cualquier nodo)

| Atributo | Significado |
|---|---|
| `x`, `y`, `z` | posición local (metros) |
| `rx`, `ry`, `rz` | rotación local (radianes) |
| `s` | escala uniforme |
| `sx`, `sy`, `sz` | escala por eje |
| `visible` | `"false"` oculta el subárbol |

Las unidades son metros y el suelo está en `y = 0`. Altura de ojos ≈ `1.6`.
`-z` es "hacia adelante" desde el origen del espacio.

### Por tag

| Tag | Atributos |
|---|---|
| `box`, `sphere`, `cylinder`, `plane` | `color="#RRGGBB"`, `touchable`, `border-radius` (sólo `box` y `plane`) |
| `text` | `value` (default `"Text"`), `size` (default `0.1`), `color` (default blanco) |
| `model` | `src` (glTF/GLB, relativo o absoluto), `rigidbody`, `collider` |
| `skybox` | `src` = **patrón con `$1`** (ver abajo) |
| `include` | `src`, `resources` |
| `space` | `resources`, `system-space` |

`touchable` acepta `true`, `1`, `yes`, `on`. Sin él, el nodo no recibe `toque`.
La forma de colisión sale del tag: `box`→caja, `plane`→plano, `sphere`/`cylinder`→esfera.

El ancho del `text` se calcula como `size * nChars * 0.6`; es una aproximación
monoespaciada, no layout real. Para centrar, colocá el texto en el mismo `x` que su fondo.

### Skybox

`src` es un patrón donde `$1` se sustituye por cada cara, en este orden:

```
pz, nz, nx, px, py, ny
```

```xml
<skybox src="http://localhost:2052/cielo/$1.png"/>
```

Carga las 6: `cielo/pz.png`, `cielo/nz.png`, … Requiere `resources="skybox"`;
sin el permiso el nodo se monta vacío y se loguea un warning.

---

## 4. Permisos (`resources`)

`<space resources="a,b,c">` declara qué capacidades pide el documento.
**Separadas por coma, sin espacios**: ver la advertencia 1 arriba. Bundles
reales (`permissions.rs`), los que un documento normal puede pedir:

| Bundle | Habilita |
|---|---|
| `navigate_self` | cambiar la URL del propio espacio (`location.href`) |
| `navigate_global` | navegar el shell entero |
| `fetch_text` | `fetch()` de texto |
| `skybox` | montar `<skybox>` |
| `read_pose_stream` | eventos `posemove` de `<posezone>` |
| `read_hmd_pose` | `dimention.readViewerPose()` con pose completa (posición + hacia dónde mira) |
| `read_camera_pose` | `dimention.readViewerPose()` **sólo con posición**; la orientación viene en cero. Anda en escritorio y en VR |
| `capture_frame` | `dimention.captureFrame(nombre)` → PNG del frame. Elevado |
| `desktop_camera_control` | control de cámara en escritorio |
| `vr_locomotion` | locomoción con mandos |

> **Ojo con esta tabla**: lista lo que un documento *puede pedir*, no lo que va a
> *recibir*. Lo efectivo es la intersección con lo que el shell concede según el
> tipo de página. Una `spatial` remota hoy recibe `navigate_self`,
> `read_pose_stream`, `read_camera_pose` y `skybox`; `fetch_text` no está en esa
> lista por más que se pida.

Elevados — **sólo** para páginas nativas o apps montadas por el shell trusted; un
origen remoto que los pida dispara UX de consentimiento o simplemente no los recibe:

`root`, `manage_tabs`, `ux_embed`, `read_system_input`,
`mount_root_space`, `unmount_root_space`, `update_root_space`, `list_root_spaces`.

Algunos bundles auto-inyectan su script:

| Bundle | Script inyectado | API que aparece |
|---|---|---|
| `root` | `luna://internal/root_api.js` | `dimension.luna.*` |
| `read_camera_pose` | `luna://internal/viewer_pose_api.js` | `dimention.readViewerPose()` (sin orientación) |
| `manage_tabs` | `luna://internal/tabs_api.js` | `dimention.tabs.*` |
| `ux_embed` | `luna://internal/embedded_api.js` | `dimention.embedded.*` |
| `read_hmd_pose` | `luna://internal/viewer_pose_api.js` | `dimention.readViewerPose()` |

### Herencia por `include`

Un `<include>` **no propaga permisos** por defecto: el contenido hijo arranca con
capacidades vacías. Si el `include` declara `resources`, eso genera un *entry grant*
para el documento hijo, siempre intersecado con lo que el padre ya tenía
(no se puede escalar privilegios desde un include).

---

## 5. La API JavaScript

### Raíz

```js
const root = hiperspace.dimention;   // sí, "dimention" — capa de compat Unity
```

`hiperspace.dimention` es un `HSMLRootElement` (nodo 0 del espacio).
`location` global apunta a `hiperspace.location`.

### Crear y componer

```js
const box = root.createElement('box');   // pendiente hasta que el host lo resuelva
box.setAttribute('color', '#405E55');
box.setAttribute('touchable', 'true');
box.id = 'mi-caja';
box.className = 'boton';
box.classList.add('activo');
root.appendChild(box);

box.removeChild(hijo);
box.remove();
```

### Transformaciones

`position`, `rotation`, `scale` y `globalPosition` devuelven un **proxy vivo**:
mutar `.x/.y/.z` escribe al host inmediatamente.

```js
box.position.y = 1.5;
box.rotation.y += 0.01;
box.scale = { x: 2, y: 2, z: 2 };     // asignación completa también sirve
const p = box.globalPosition;          // world space
```

Ojo: asignar un objeto captura los valores en ese momento, así que podés reusar un
vector temporal en un bucle sin aliasing.

Para mover muchos nodos por frame, usá el batch (una sola op en vez de N):

```js
// layout plano, múltiplos de 7: [nodeId, px,py,pz, rx,ry,rz, ...]
root.setTransformBatch([
  a.nodeId, 0,1,-2, 0,0,0,
  b.nodeId, 1,1,-2, 0,0.5,0,
]);
```

### Consultas

```js
root.getElementById('btn');
root.getElementByClass('boton');
root.getElementByName('avatar');
root.getElementsByClass('boton');   // array
root.getElementsByName('enemigo');  // array
el.parent;    // HSMLElement | null
el.children;  // array
el.tagName;
```

No hay `querySelector` con selectores CSS: sólo estas búsquedas por id / class / name.

### Eventos

`addEventListener(type, fn)` / `removeEventListener` / `dispatchEvent(evt)`.
Los tipos se normalizan a minúsculas. También hay handlers `onX` por propiedad.

Eventos que emite el host:

| Evento | Cuándo | Campos extra |
|---|---|---|
| `toque` | click de mouse (escritorio) o trigger derecho (VR) sobre un nodo `touchable` | `x`, `y`, `z` = punto de impacto en world space |
| `posemove` | mano/mando dentro de un `<posezone>` (requiere `read_pose_stream`) | `hand`, `px py pz`, `dx dy dz`, `trigger`, `grip`, `qx qy qz qw` |

`toque` es *el* click en Luna. No existe `click` nativo: `el.click()` sólo despacha
un evento sintético `click` a tus propios listeners.

```js
const btn = root.getElementById('btn');
btn.addEventListener('toque', (e) => {
  console.log('impacto en', e.x, e.y, e.z);
  location.href = 'luna://home';
});
```

### Web APIs disponibles

```js
setTimeout / setInterval / clearTimeout / clearInterval
requestAnimationFrame / cancelAnimationFrame
console.log / warn / error
await fetch(url)                 // requiere fetch_text, que una página spatial remota NO recibe
localStorage                     // persistente por origen, sin permiso
new WebSocket('ws://...')        // onopen/onmessage/onerror/onclose, polling cada 16 ms
location.href = '...'            // navegar (requiere navigate_self / navigate_global)
```

`localStorage` se separa por **protocolo + host + puerto**. Todas las páginas
`luna://` comparten un almacén interno. `data:` y `file:` reciben `SecurityError`.
Detalle completo en [`../LOCAL_STORAGE.md`](../LOCAL_STORAGE.md).

### APIs condicionadas a permisos

```js
// read_hmd_pose
const pose = hiperspace.dimention.readViewerPose();
// { mode:'vr'|'desktop', px,py,pz, forwardX/Y/Z, yaw,pitch, qx,qy,qz,qw, aspect,fovY }

// manage_tabs
dimention.tabs.open(url, { kind: 'spatial' | 'app' | 'app-embedded' });
dimention.tabs.close(tabId);

// root (sólo páginas nativas)
dimension.luna.mountSpace(url, { tabId, kind });
dimension.luna.unmountSpace(id);
dimension.luna.listMountedSpaces();
dimension.luna.switchMode('vr' | 'desktop');
```

Tipos de espacio:

- `spatial` — "irte a otro lugar"; al abrir uno, el shell cierra los otros spatial.
- `app` — aditivo, sobrevive a cambios de espacio spatial.
- `app-embedded` — aditivo, además negocia un slot con el shell vía `dimention.embedded`.

---

## 6. Ejemplo completo

Dos archivos servidos desde `http://localhost:2052/`.

### `demo.hsml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Demo Luna</name>
  </head>

  <!-- Coma, no espacio. Y sólo lo que una página remota recibe de verdad:
       fetch_text no se concede, y de la pose llega read_camera_pose. -->
  <space resources="navigate_self,read_camera_pose">
    <group id="panel" z="-3">
      <text y="2.0" value="Demo Luna" size="0.28" color="#344C49"/>
      <text id="contador" y="1.72" value="toques: 0" size="0.09" color="#4E6560"/>

      <plane y="1.2" sx="2.2" sy="0.9" sz="1" color="#E9E4D2" border-radius="0.08"/>

      <box id="btn_girar" x="-0.6" y="1.2" z="0.05"
           sx="0.8" sy="0.3" sz="0.08"
           color="#405E55" touchable="true" border-radius="0.04"/>
      <text x="-0.6" y="1.2" z="0.1" value="Girar" size="0.08" color="#F4F1E5"/>

      <box id="btn_home" x="0.6" y="1.2" z="0.05"
           sx="0.8" sy="0.3" sz="0.08"
           color="#7A4A3A" touchable="true" border-radius="0.04"/>
      <text x="0.6" y="1.2" z="0.1" value="Inicio" size="0.08" color="#F4F1E5"/>
    </group>

    <sphere id="orbe" y="0.6" z="-2" s="0.25" color="#C9A227" touchable="true"/>

    <script src="demo.js"/>
  </space>
</hsml>
```

### `demo.js`

```js
const root = hiperspace.dimention;

const contador = root.getElementById('contador');
const orbe     = root.getElementById('orbe');
const btnGirar = root.getElementById('btn_girar');
const btnHome  = root.getElementById('btn_home');

let toques = 0;
let girando = false;

// --- Eventos -------------------------------------------------------------
orbe.addEventListener('toque', (e) => {
  toques += 1;
  contador.setAttribute('value', `toques: ${toques}`);
  console.log('impacto en', e.x.toFixed(2), e.y.toFixed(2), e.z.toFixed(2));

  // Persistimos entre visitas (mismo origen).
  localStorage.setItem('toques', String(toques));
});

btnGirar.addEventListener('toque', () => {
  girando = !girando;
});

btnHome.addEventListener('toque', () => {
  location.href = 'luna://home';   // necesita navigate_self
});

// --- Estado previo -------------------------------------------------------
toques = Number(localStorage.getItem('toques') || 0);
contador.setAttribute('value', `toques: ${toques}`);

// --- Nodos creados desde JS ---------------------------------------------
const satelites = [];
for (let i = 0; i < 6; i++) {
  const s = root.createElement('box');
  s.setAttribute('color', '#405E55');
  s.scale = { x: 0.06, y: 0.06, z: 0.06 };
  root.appendChild(s);
  satelites.push(s);
}

// --- Loop ----------------------------------------------------------------
let t = 0;
function frame() {
  if (girando) {
    t += 0.02;
    orbe.rotation.y = t;

    // Un solo op para los 6 satélites en vez de 6 escrituras sueltas.
    const batch = [];
    satelites.forEach((s, i) => {
      const a = t + (i * Math.PI * 2) / satelites.length;
      batch.push(
        s.nodeId,
        Math.cos(a) * 0.7,
        0.6 + Math.sin(a * 2) * 0.15,
        -2 + Math.sin(a) * 0.7,
        0, a, 0,
      );
    });
    root.setTransformBatch(batch);
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// --- Pose del usuario (read_camera_pose) ---------------------------------
// Con read_camera_pose llega la posición y la orientación en cero; con
// read_hmd_pose vendría completa, pero una página remota no la recibe.
const pose = root.readViewerPose && root.readViewerPose();
if (pose) console.log('modo:', pose.mode, 'en', pose.px.toFixed(2), pose.pz.toFixed(2));

// --- Red -----------------------------------------------------------------
// OJO: esto falla en una página spatial remota. fetch necesita fetch_text y el
// shell no se lo concede (ver advertencia 3). Queda como referencia para
// páginas nativas o apps montadas por el shell.
fetch('http://localhost:2052/data.json')
  .then(r => r.json())
  .then(d => console.log('datos', d))
  .catch(e => console.warn('fetch falló', e));

// Lo que sí funciona desde un origen remoto: que el motor traiga el documento.
// Los <include> los resuelve el motor, no el JS, así que no piden permiso.
const inc = root.createElement('include');
inc.setAttribute('src', 'http://localhost:2052/pedazo.hsml?q=1');
root.getElementById('panel').appendChild(inc);
```

### Probarlo

```bash
docker compose up -d          # Caddy + Bun sirven el directorio en :2052
cargo run -p luna --profile release-fast
```

Dentro de Luna, escribí `http://localhost:2052/demo.hsml` en la barra de direcciones.
En escritorio: WASD + Q/E para moverse, mouse para mirar, click para `toque`.
En VR: gatillo derecho para `toque`.

---

## 7. Patrones y trampas

**Delegar navegación con un mapa de ids.** Es lo que hacen las páginas internas
(`routes.rs`):

```js
const links = { btn_demos: 'luna://demos', btn_about: 'luna://about' };
for (const [id, url] of Object.entries(links)) {
  const el = root.getElementById(id);
  if (el) el.addEventListener('toque', () => { location.href = url; });
}
```

**Siempre chequear `if (el)`.** `getElementById` devuelve `null` si el nodo todavía
no se sincronizó o el id no existe; el código del repo lo hace sin excepción.

**Los botones son dos nodos.** `box` con `touchable` + `text` encima desplazado en
`z` (típicamente `+0.04` a `+0.1`). El `text` nunca es touchable.

**No hay layout.** Todo es posicionamiento absoluto en metros. El texto se
autoescala por conteo de caracteres.

**`createElement` es asíncrono por dentro.** Podés encadenar `setAttribute` y
`appendChild` de inmediato, pero `el.nodeId` es negativo hasta que se resuelve, y
los getters (`getAttribute`, `children`, `tagName`) devuelven valores vacíos o
cacheados mientras tanto. Si necesitás leer, hacelo dentro de un
`requestAnimationFrame` posterior.

**Preferí `setTransformBatch` en loops.** Cada `.position.x = …` es una op cruzando
el borde JS↔Rust.

**`include` no hereda permisos.** Si tu sub-documento necesita `fetch_text`, el
`<include>` tiene que declararlo, y el padre ya debe tenerlo.

**Cada `<include>` es un space, o sea un isolate de JS.** No es sólo geometría:
son una petición, un parseo, una frontera de permisos y un worker. No hay acceso
al DOM entre spaces, así que padre e hijo no se ven; el único canal es lo que el
padre le inyecte por la URL. Medido, ese costo fijo domina el streaming por
chunks: agrupar varios chunks en un documento fue lo que movió los FPS, no
reducir geometría. Ver [`docs/costo_de_montaje.md`](docs/costo_de_montaje.md).

**`border-radius` existe en `box` y `plane`, y regenera la malla.** Vale
recordarlo porque sin él un panel con esquinas redondeadas hay que armarlo con
dos cajas cruzadas y cuatro cilindros girados, que es lo que uno termina
haciendo si no sabe que está.

**Los `plane` son de doble cara.** No hace falta duplicarlos ni girarlos para
verlos desde atrás.

**`setTransformBatch` toma 7 floats por nodo**: `[nodeId, px,py,pz, rx,ry,rz, …]`,
en coordenadas **locales**. La escala **no** va en el batch: para eso está el
proxy `el.scale`, que sí cruza el borde una vez por acceso.

---

## 8. Dónde seguir

- `crates/luna/src/routes.rs` — todas las páginas `luna://` con su HSML+JS inline.
  Es el mejor catálogo de patrones reales (demos de disparo, targets, ajustes, caché).
- `crates/luna/src/web/home.hsml` + `web/internal/home_navigation.js` — documento
  mínimo completo.
- `crates/js_runtime/runtime.js` — verdad absoluta de la API JS.
- [`../LOCAL_STORAGE.md`](../LOCAL_STORAGE.md) — cuotas y aislamiento por origen.
- [`../NATIVE_ENVIRONMENT.md`](../NATIVE_ENVIRONMENT.md) — el islote compartido de las páginas nativas.
- [`embedded_apps.md`](embedded_apps.md) — protocolo de apps `app-embedded`.
- [`CACHE_CSP_IMPLEMENTATION.md`](CACHE_CSP_IMPLEMENTATION.md) — caché HTTP y CSP.
- [`docs/costo_de_montaje.md`](docs/costo_de_montaje.md) — qué cuesta montar
  documentos, medido en el motor, y el banco de pruebas que lo mide.
- [`docs/deuda_tecnica/deudas_de_plataforma.md`](docs/deuda_tecnica/deudas_de_plataforma.md)
  — lo que falta en la plataforma, visto desde una página remota.
