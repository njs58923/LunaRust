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

**`fetch` requiere permiso y respeta el mismo origen.** Pedí `fetch_text` para
GET/HEAD o `fetch_http` para métodos de escritura. El shell los ofrece a páginas
espaciales; los includes necesitan delegación explícita. Ver [Fetch HTTP](FETCH_HTTP.md).

**`<include>` no emite `load` ni `error`.** No hay forma de saber desde el
documento si un sub-documento terminó de montarse o falló. Si tu código marca
algo como "montado" al crear el nodo, un fallo lo deja registrado como presente
para siempre.

**`touchable` no tiene default permisivo.** `node_touchable()` devuelve `false`
cuando el atributo falta: sin `touchable="true"` explícito, un nodo con `id` y
listener no recibe nada. Ver `home.hsml`, que lo pone en todos sus botones.

**El `<text>` se centra en `x` y también en `y`.** La guía sólo habla del ancho;
la coordenada que le des es el centro de la caja, no su esquina ni su línea base.

**El `<text>` es de una sola cara**, al revés que los `plane`, que sí son de
doble cara. Se ve desde su **+z local**: girado 180° desaparece, sin error y sin
warning. Si el cartel está dentro de un `<group>` rotado —un pedestal encarado
al visitante, por ejemplo— hay que orientar el grupo pensando en el +z del
texto, no en el -z de "hacia adelante".

**Los triángulos de una malla dinámica también son de una sola cara**, y el
orden de los vértices decide cuál. Una bóveda de estrellas armada con la normal
hacia afuera se ve vacía desde adentro: hay que emitir los vértices en orden
inverso. No hay aviso; simplemente no se dibuja nada.

**Hay 128 mallas dinámicas por sesión, y no se devuelven solas.** `MAX_RESOURCES`
en `crates/js_runtime/src/mesh.rs` es 128, y el contador **no baja al cambiar de
espacio**: los `MeshResource` de una página que ya no está siguen contando. Una
sesión que navegue veinte veces a escenas con mallas llega al tope, y a partir de
ahí **todo** `MeshResource.create` falla con `Mesh resource/queue limit reached`
—incluso en una página recién abierta— hasta reiniciar Luna. Hay `dispose()` en
la API, pero no hay evento de descarga desde el que llamarlo. Ver
`docs/deuda_tecnica/deudas_de_plataforma.md`.

**Los colores de una malla dinámica tienen que estar en [0, 1].** El motor
valida el buffer y `MeshResource.create` tira `Vertex colors must be in [0, 1]`.
No hay HDR ni sobreexposición: pasarse de 1 para "aclarar" no aclara, falla, y
si eso pasa dentro de un `requestAnimationFrame` que reintenta, el log se llena
de miles de líneas iguales. El contraste hay que ponerlo del otro lado — el
color del fondo.

**Un `cylinder` visto desde adentro no se dibuja.** Las caras miran para afuera,
como en cualquier malla cerrada. Sirve para una isla o un tronco, no para una
banda de horizonte alrededor del visitante. Lo único que se encara hacia adentro
sin trabajo es un `plane`, que sí es de doble cara.

**Pero sus tapas sí se ven, y son enormes.** El corolario del punto anterior, que
cuesta caro encontrar porque no da error: un `cylinder` chato y ancho puesto de
techo —un riel, un anillo, un aro— tapa todo lo que haya arriba con su **cara de
abajo**, que es un disco del diámetro completo. Un anillo hay que armarlo con
piezas, no achatando un cilindro. (Salió en `/observatorio.hsml`: un riel de 23 m
de diámetro y 22 cm de alto borraba la cúpula entera y el cielo con ella.)

**Un error de sintaxis en el script del cliente no sale por ningún lado.** No hay
excepción, no hay warning, no hay entrada de log. El espacio carga, el HSML se
monta, y el script sencillamente no corre — ni siquiera el `console.log` de
arranque. Desde afuera es indistinguible de un script que corre y no encuentra sus
nodos. La verificación barata, antes de abrir nada:

```
bun build public/<escena>.js --outfile /dev/null
```

**`navigate_global` existe en el motor pero no se concede.** El planificador
(`plan_navigation_for_space`, `js.rs`) es explícito:

```rust
if caps.contains(NAVIGATE_SELF) {
    if let Some(inc) = find_nearest_ancestor_include(space) { return SelfNav { inc, url }; }
}
if caps.contains(NAVIGATE_GLOBAL) { return GlobalNav { url }; }
Blocked
```

O sea que un espacio con `navigate_global` **y sin** `navigate_self` navegaría el
shell entero aunque esté dentro de un include. Lo que lo hace inalcanzable hoy es
la lista de grants por defecto en `web/internal/root_api.js`:

```js
// kind === 'spatial'
grants = ['navigate_self', 'read_pose_stream', 'read_camera_pose', 'skybox',
          'fetch_text', 'fetch_http', 'spawn'];
```

`navigate_global` no está, y los grants de un `<include>` se intersecan con los
del padre, así que ningún hijo puede tenerlo si el mundo no lo tiene. Agregarlo a
esa lista es una línea, pero es una decisión de seguridad: le da a cualquier
documento remoto la capacidad de navegar el shell. La propuesta de un tercer
destino más angosto —reemplazar el mundo sin tocar el shell— está en
[`NAVIGATE_WORLD.md`](NAVIGATE_WORLD.md).

Vale notar también que **el mundo mismo es un include**: `mountSpace` monta el
documento raíz dentro de un `<include>`, y por eso `navigate_self` en una escena
se siente como "navegar el mundo". Una puerta anidada tiene su propio include más
cerca, y `find_nearest_ancestor_include` encuentra ése.

**Un `<include>` no puede navegar el mundo: se navega a sí mismo.** Es la
consecuencia práctica de "en una puerta anidada, `navigate_self` sigue navegando
esa puerta" (`LOCATION.md`), y desde afuera se ve así: al tocar el vano, la escena
de destino **se carga adentro del marco** y el mundo de alrededor sigue estando.
`navigate_world` todavía no existe.

Pero un componente sí es posible, porque las dos piezas que hacen falta funcionan
—las dos verificadas en Luna, no deducidas:

| | |
|---|---|
| dos `<include>` del mismo archivo son **dos isolates con dos URLs** | sí |
| el hijo lee sus parámetros con `location.search` | sí |
| el documento **padre** ve los nodos del include por `getElementById` | sí |
| el padre puede **escribirles atributos** | sí |
| el hijo puede cambiarse su propio `id` con `setAttribute` | sí |
| el padre puede **escuchar un `toque`** en un nodo del include | **no** |

La última es la que manda, y no es obvia. `dispatch_toque_events_to_js`
(`touch.rs`) resuelve el destinatario con `find_owner_space_id`, que sube hasta el
`<space>` **más cercano** — el del propio include. El evento se despacha
únicamente a ese isolate, así que un listener puesto desde el padre sobre un nodo
del hijo **no se entera nunca**. Se puede pintar a través del borde; escuchar, no.

**`getElementsByClass` anda**, y es la forma de pintar muchos nodos de un
componente sin ponerle un id a cada uno: se les pone `class` en el HSML y el
script los recorre. Verificado con 24 nodos por documento en
`server_noche/public/puerta.js`.

El reparto que sí funciona: **el componente dibuja, y el nodo tocable lo declara
el padre**, encima del marco del include. Está implementado así en
`server_noche/public/puerta.hsml` + `src/atrio.ts`: un único documento para las
veintiocho puertas del atrio, y veintiocho vanos tocables en el documento del
atrio.

Lo que **no** funciona, probado: la escala de un `<group>` **no** se aplica al
contenido de un `<include>`. La idea de meter el mundo de destino encogido adentro
del marco, como maqueta viva, no sale por ese camino — el sub-mundo se dibuja a su
propia escala.

**El `ry` de un `<spawn>` gira al revés que el de un objeto.** Medido con
`luna_status`: el forward del visitante es **`(-sin ry, 0, -cos ry)`**. Para que
mire a un punto `P` parado en `S`:

```
ry = atan2(Sx - Px, Sz - Pz)      // mirar a P desde S
ry = atan2(Sx, Sz)                // mirar al origen
```

No es el `atan2(-x, -z)` que se usa en todo `server_noche` para orientar arcos y
paneles: ése apunta el **contenido** de un grupo (que mira a su +Z local) hacia el
centro, y da justo lo contrario. Las dos fórmulas conviven en el mismo archivo y
se confunden solas; conviene escribir cuál es cuál al lado de cada una.

**Un `<spawn>` no inclina el visor**, sólo lo gira en horizontal. Consecuencia
práctica al componer la llegada: lo que esté por debajo del nivel de los ojos
—una mesa, un yunque, un tablero— hay que **mirarlo de cerca o desde bastante
lejos**, porque a media distancia queda debajo del encuadre. Con los ojos a 1,70 m
y una superficie a 0,95 m, la caída es de 23° a dos metros y de 13° a cuatro.

**Y la trampa boba, que picó dos veces:** no poner el spawn detrás del propio
cartel de la escena. Un panel de 3,7 m de ancho a medio metro de la cara es una
pantalla negra, y desde afuera parece que el espacio no cargó.

**Resolver miles de nodos por `getElementById` en un solo frame mata el script.**
Sin error, sin log y sin nada: el espacio monta, la escena se ve, y el script
simplemente no arranca — el mismo síntoma que un error de sintaxis, con otra
causa. Medido en este motor, buscando ids de un pozo grande:

| ids en una tanda | resultado |
|---:|---|
| 2 016 | anda |
| 2 560 | **el script no arranca** |
| 3 072, repartidos de a 400 por frame | anda |
| 6 400, repartidos de a 400 por frame | no arranca (otra pared, más arriba) |

La cura es repartir la resolución en varios frames: el `preparar()` que ya
devuelve `false` hasta estar listo lleva un cursor y resuelve un puñado por
vuelta. Cuesta medio segundo de arranque que nadie ve. Está hecho así en
`server_noche/public/circuito.js` y `telar.js`; el resto de las escenas resuelve
de una porque tiene pocos nodos, y está bien.

**Pero la cura de fondo es no buscar.** Este techo sólo aparece si uno declara los
nodos en el HSML y después los busca por id. Con `createElement` uno se queda con
la referencia y no llama a `getElementById` ni una vez, y el problema desaparece
en vez de mitigarse. Medido: 800 `createElement` + `appendChild` cuestan **0 ms**
de script y quedan resueltos **15 ms** después. Declarar un "pozo" de nodos
escondidos para irlos sacando es un patrón que **no hace falta** — y que, además,
es el que crea este techo.

**Un `<model>` con animación no la reproduce solo.** Hace falta declarar
`animation-clip`: `select_clip` (`model_animation.rs`) devuelve `None` cuando el
atributo está vacío, así que el modelo se queda quieto por más que
`animation-state` valga `"playing"`, que es el default. No hay error ni warning
— el .glb trae sus clips y no pasa nada. Ver la tabla de atributos de `model`.

**Las texturas del `.glb` no se aplican.** Del material sólo entra el color
base: un modelo texturizado se dibuja **gris liso**, sin error y sin warning.
Para un asset low-poly que usa la textura como paleta, la salida es hornearla a
materiales planos antes de exportar —una muestra por cara, cuantizada, un
material por color—; para una textura de verdad no hay salida hoy.

**Pero el `COLOR_0` del glTF sí funciona.** Un `.glb` con colores por vértice y
un material blanco se dibuja con esos colores, y eso cambia el precio de las
cosas: hornear una textura a materiales planos cuesta **una primitiva por
color** —una pieza de 67 colores son 67 primitivas por copia—, y hornearla a
color por vértice cuesta **una**. Es además más fiel, porque el color no se
cuantiza a una paleta: se muestrea, y la resolución que se pierde es la de la
malla. En Blender hay que enchufar un nodo *Color Attribute* al *Base Color*
del material: si el árbol de nodos no lo usa, el exportador **no escribe el
atributo** y sólo avisa con un warning en consola.

**No hay canal alfa.** Un material con transparencia se dibuja opaco. Todo pack
que resuelva follaje, rejas o vidrios con cards recortadas por alfa se ve como
rectángulos. Lo único que se puede hacer es tallar la silueta en la geometría:
grilla sobre la textura, tirar la celda cuyo alfa no llega al umbral, y quedarse
con el resto. Geometría a cambio de transparencia.

**Ojo con lo que el exportador de glTF escribe en el nodo.** Si el objeto tiene
padre, transformaciones delta o animación importada del FBX, esa transformación
viaja en el nodo del `.glb` y el modelo aparece corrido —metros— respecto de la
posición que le da el documento. La animación es la peor de las tres: el
depsgraph la evalúa y pisa la transformación que uno le asigna al objeto. La
comprobación que sirve es sobre el archivo, no sobre la escena de origen: todo
nodo con traslación cero y escala uno.

**`setAttribute` funciona en caliente, y `getAttribute` no lo ve.** Se puede
cambiar en vivo el `value` y el `size` de un `<text>`, el `color` de cualquier
primitiva y hasta el `sx` de una caja: el cambio llega al motor y se dibuja. Lo
que no se actualiza es la lectura — `getAttribute` sigue devolviendo lo que
decía el HSML, así que el estado hay que guardarlo en el script. Verificado con
un banco chico: `value:ok color:ok size:ok sx:ok`, `getAttribute` devolvió el
valor viejo.

**`setTransformBatch` escribe la transformación local completa.** Cada entrada es
`nodeId, x, y, z, rx, ry, rz` y **las siete cosas se aplican**: mandar cero en la
posición porque «esa no cambia» no la deja como estaba, la manda al origen. Si
sólo se quiere rotar, hay que repetir la posición en cada entrada.

**No hay `visible` ni `display`.** Para esconder un nodo desde el script hay que
achicarlo con `scale`, y conviene un valor chico pero **distinto de cero** (0.0001
sirve): una matriz de escala nula es singular y no todas las etapas del pipeline
la tratan igual.

**`el.scale` reemplaza el tamaño del nodo, no lo multiplica.** Asignarle `0.94`
a una caja declarada `sx="0.19" sy="0.24" sz="0.19"` la convierte en un cubo de
94 cm, no la achica un 6%. Si querés un pulso sobre el tamaño declarado, tenés
que guardarte las dimensiones originales en el script y multiplicarlas vos.

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
| `spawn` | punto invisible de aparición del visitante; requiere `resources="spawn"` |
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
| `model` | `src` (glTF/GLB, relativo o absoluto), `rigidbody`, `collider`, y los de animación (abajo) |
| `skybox` | `src` = **patrón con `$1`** (ver abajo) |
| `include` | `src`, `resources` |
| `space` | `resources`, `system-space` |

`touchable` acepta `true`, `1`, `yes`, `on`. Sin él, el nodo no recibe `toque`.
La forma de colisión sale del tag: `box`→caja, `plane`→plano, `sphere`/`cylinder`→esfera.

El ancho del `text` se calcula como `size * nChars * 0.6`; es una aproximación
monoespaciada, no layout real. Para centrar, colocá el texto en el mismo `x` que su fondo.

### Animación de un `<model>`

Los clips que trae el `.glb` se controlan por atributos (`model_animation.rs`).
**Sin `animation-clip` no se reproduce nada**: es el único que no tiene default
útil.

| Atributo | Default | Qué hace |
|---|---|---|
| `animation-clip` | *(vacío = no anima)* | nombre del clip, o su índice (`"0"` es el primero) |
| `animation-state` | `playing` | `playing`, `paused` o `stopped` |
| `animation-loop` | `true` | `true`/`1` o `false`/`0` |
| `animation-speed` | `1` | multiplicador, no negativo |
| `animation-time` | — | busca un instante del clip, en segundos |
| `animation-restart` | — | cambiar su valor reinicia el clip desde cero |

El motor **escribe de vuelta** tres atributos sobre el nodo, que se pueden leer
desde JS con `getAttribute`:

| Atributo | Qué trae |
|---|---|
| `animation-clips` | JSON con el catálogo de clips del modelo: nombres y duraciones |
| `animation-status` | `idle`, `playing`, `paused`, `stopped` o `error` |
| `animation-error` | el motivo, cuando `animation-status` es `error` |

`animation-clips` es la forma de averiguar cómo se llaman los clips de un
modelo ajeno sin abrirlo en Blender.

```xml
<model src="bicho.glb" animation-clip="0" animation-loop="true"/>
```

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
| `fetch_text` | `fetch()` GET/HEAD con respuestas HTTP reales |
| `fetch_http` | `fetch()` con métodos, cabeceras y cuerpo de texto/JSON/formulario |
| `skybox` | montar `<skybox>` |
| `spawn` | aplicar una aparición por carga del documento espacial principal |
| `read_pose_stream` | eventos `posemove` de `<posezone>` |
| `read_hmd_pose` | `dimention.readViewerPose()` con pose completa (posición + hacia dónde mira) |
| `read_camera_pose` | `dimention.readViewerPose()` **sólo con posición**; la orientación viene en cero. Anda en escritorio y en VR |
| `capture_frame` | `dimention.captureFrame(nombre)` → PNG del frame. Elevado |
| `desktop_camera_control` | control de cámara en escritorio |
| `vr_locomotion` | locomoción con mandos |

> **Ojo con esta tabla**: lista lo que un documento *puede pedir*, no lo que va a
> *recibir*. Lo efectivo es la intersección con lo que el shell concede según el
> tipo de página. Una `spatial` remota hoy recibe `navigate_self`,
> `read_pose_stream`, `read_camera_pose`, `skybox`, `fetch_text`, `fetch_http` y `spawn` como
> concesiones predeterminadas. Los dos permisos fetch y `spawn` requieren además una
> solicitud explícita del documento. El shell puede limitar esas concesiones.

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

`spawn` tiene una restricción adicional: sólo se usan marcadores del documento
principal de un montaje `spatial` visible. Los includes anidados, los espacios
anidados y las apps no pueden definir la aparición, aunque tengan el permiso
o compartan origen. Ver [Puntos de aparición](SPAWN.md).

Un `<include>` **no propaga permisos** por defecto: el contenido hijo arranca con
capacidades vacías. Si el `include` declara `resources`, eso genera un *entry grant*
para el documento hijo, siempre intersecado con lo que el padre ya tenía
(no se puede escalar privilegios desde un include).

---

## 5. La API JavaScript

`location` expone la URL del documento de cada isolate, con `search`, `hash`,
`pathname`, `origin` y navegación. Para leer parámetros:
`new URLSearchParams(location.search).get('uuid')`.
También está disponible `new URL(ruta, location.href)`.
Ver [Location y parámetros](LOCATION.md) para includes, permisos y límites.

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
| `pointerenter`, `pointerleave` | entrada/salida del cursor o rayos VR; no burbujean | `target`, `currentTarget`, `relatedTarget`, `pointerId`, `pointerType`, `hand` |
| `pointerover`, `pointerout` | cambio de objetivo; burbujean dentro del isolate | mismos campos |
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

Hover admite también `mouseenter`, `mouseleave`, `mouseover` y `mouseout` para
mouse y mando derecho, propiedades `onpointerenter`, etc., y `matches(':hover')`.
Para ambos mandos usar `pointer*`. Requiere `touchable="true"` en el objetivo.
Ver [semántica y ejemplo de hover](HOVER.md). No incorpora un motor CSS.

### Web APIs disponibles

```js
setTimeout / setInterval / clearTimeout / clearInterval
requestAnimationFrame / cancelAnimationFrame
console.log / warn / error
await fetch(url)                 // requiere fetch_text (lectura) o fetch_http, mismo origen
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

  <!-- Coma, no espacio. fetch_text habilita lectura del propio origen. -->
  <space resources="navigate_self,read_camera_pose,fetch_text">
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
// Lectura del propio servidor; requiere fetch_text en el space.
fetch('./data.json')
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
