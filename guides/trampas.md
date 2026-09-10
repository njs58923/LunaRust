# Trampas y patrones

Todo lo que cuesta una corrida averiguar. Verificado contra el motor, no contra
el README — y cuando algo cambió, dice qué cambió.

**Si vas a leer una sola página de estas guías, que sea ésta.**

---

## Advertencias

Verificadas contra el motor construyendo tres páginas remotas completas
(exposición de assets, backrooms procedurales, banco de pruebas de carga). Las
tres primeras estaban **mal documentadas acá**; el resto faltaba.

**Los `resources` van separados por COMA.** `parse_resource_tokens` hace
`split(',')` a secas. Con espacios queda un único token que no matchea ningún
bundle, el documento se queda **sin capacidades y nadie avisa**. Las guías los
mostraban separados por espacios; ya está corregido en
[permisos.md](permisos.md), pero si copiaste un ejemplo viejo, revisalo. Un día entero de creer que `readViewerPose` estaba roto.

**`<meta type="position">` no hace nada.** Aparece en todos los documentos,
incluidos los internos, pero ningún código del motor lo lee. Para mover un
documento al montarlo, envolvé el contenido en un `<group>` con transform.

**`fetch` requiere permiso y respeta el mismo origen.** Pedí `fetch_text` para
GET/HEAD o `fetch_http` para métodos de escritura. El shell los ofrece a páginas
espaciales; los includes necesitan delegación explícita. Ver [Fetch HTTP](red.md).

**`<include>` no emite `load` ni `error`.** No hay forma de saber desde el
documento si un sub-documento terminó de montarse o falló. Si tu código marca
algo como "montado" al crear el nodo, un fallo lo deja registrado como presente
para siempre.

**`touchable` no tiene default permisivo.** `node_touchable()` devuelve `false`
cuando el atributo falta: sin `touchable="true"` explícito, un nodo con `id` y
listener no recibe nada. Ver `home.hsml`, que lo pone en todos sus botones.

**El `<text>` se centra en `x` y también en `y`.** La referencia sólo habla del ancho;
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

**Los colores de una malla dinámica se entregan en espacio lineal.** No en sRGB.
Pasar el 0,11 de un `#1C1C26` tal cual da un gris lavanda: el pipeline lo
convierte a sRGB al escribir y lo sube a 0,37. Con el material iluminado el error
se disimula —la luz ambiente lo baja de vuelta, por casualidad— y sólo se nota al
prender `material-unlit`, que es justo lo que quiere una interfaz. La conversión
es `c ≤ 0.04045 ? c/12.92 : ((c+0.055)/1.055)^2.4`.

**`MeshResource.update` manda los cinco buffers, siempre.** Los que uno no pasa
viajan **vacíos, no "sin cambios"**, y del lado de Rust un `indices` vacío
significa *índices implícitos* —0,1,2, 3,4,5…—, así que la malla se rearma como
tiras de triángulos entre vértices consecutivos: en pantalla, un erizo de púas.
Mandar sólo lo que cambió es la optimización obvia y está mal.

**Los arrays de una malla tienen que ser tipados y planos.** `mesh.js` acepta
arrays comunes y hace `new Float32Array(valor)`, que sobre un array de ternas da
`NaN` en cada posición. El mensaje que sale es
`Mesh attributes must be finite and bounded`, y es literal: el `NaN` lo fabricó
la conversión.

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
[`disenos/navegacion-de-mundo.md`](disenos/navegacion-de-mundo.md).

Vale notar también que **el mundo mismo es un include**: `mountSpace` monta el
documento raíz dentro de un `<include>`, y por eso `navigate_self` en una escena
se siente como "navegar el mundo". Una puerta anidada tiene su propio include más
cerca, y `find_nearest_ancestor_include` encuentra ése.

**Un `<include>` no puede navegar el mundo: se navega a sí mismo.** Es la
consecuencia práctica de "en una puerta anidada, `navigate_self` sigue navegando
esa puerta" (`ubicacion.md`), y desde afuera se ve así: al tocar el vano, la escena
de destino **se carga adentro del marco** y el mundo de alrededor sigue estando.
`navigate_world` todavía no existe.

Pero un componente sí es posible, porque las dos piezas que hacen falta funcionan
—las dos verificadas en Luna, no deducidas:

| | |
|---|---|
| dos `<include>` del mismo archivo son **dos isolates con dos URLs** | sí |
| el hijo lee sus parámetros con `location.search` | sí (hoy conviene `props`) |
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

**Pero ya hay un canal.** Todo lo de la tabla sigue siendo cierto —el toque
nunca cruza el borde— y sin embargo el reparto que obligaba cambió, porque un
include con `props` o `events` habla con el espacio que lo contiene:

```xml
<include id="puerta_cueva" src="./puerta.hsml" events="abrir"
         props='{"titulo":"La cueva","roca":"#4A515C"}'/>
```

El hijo lee `component.props` —disponibles **antes de su primer script**— y
avisa con `component.emit('abrir', {...})`; el padre escucha `component:abrir`
**en el nodo include** y contesta reemplazando `include.props`. Así el nodo
tocable puede vivir dentro del componente: recibe su toque en su propio isolate
y lo que cruza el borde es el evento, no el nodo. Contrato completo y límites en
[componentes.md](componentes.md).

Está implementado así en `server_noche/public/puerta.hsml` + `src/atrio.ts`: un
único documento **y una única URL** para las veintiocho puertas del atrio, cada
una con su propio vano adentro. Antes eran veintiocho URLs —los datos viajaban
en la query— y veintiocho vanos tocables declarados por el atrio.

Lo que el canal **no** arregla: **un include sigue sin poder navegar** el
documento que lo contiene. Por eso la puerta avisa y el padre navega.

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

**~~No hay canal alfa.~~ Ahora hay `material-alpha`.** Esta advertencia se
escribió antes de `add superficies` y ya no vale como estaba. El atributo acepta
`opaque`, `mask` y `blend`, y en un `<model>` **alcanza a todo el glTF**: el
sistema recorre los descendientes de la instancia y les reemplaza el material
conservando el original como base (`surface.rs:531`). O sea que un pack con
follaje recortado por alfa debería resolverse con
`material-alpha="mask"` —umbral 0,5— en vez de tallando la silueta.

Verificado leyendo el código, **no medido en pantalla**: si alguien lo prueba con
un pack de verdad, que anote acá el resultado. Ver
[superficies.md](superficies.md).

El truco viejo —grilla sobre la textura, tirar la celda cuyo alfa no llega al
umbral, quedarse con el resto— sigue sirviendo para bajar el costo de relleno,
que es otro problema: geometría a cambio de píxeles dibujados.

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

**~~No hay `visible` ni `display`.~~ Sí hay `visible`, y hace lo correcto.**
Esta advertencia era falsa —y se contradecía con la tabla de atributos de esta
misma guía—. `node_visibility` (`dom.rs:327`) acepta
`false`, `0`, `no`, `off`, `hidden` para ocultar, cualquier otra cosa para
forzar visible, y **sin el atributo hereda del padre**, que es lo que hace que
ocultar un grupo oculte su subárbol.

Lo que sigue valiendo es el truco viejo por si hace falta esconder algo sin tocar
la jerarquía: achicarlo con `scale` a un valor chico pero **distinto de cero**
(0.0001 sirve), porque una matriz de escala nula es singular y no todas las
etapas del pipeline la tratan igual.

**`el.scale` reemplaza el tamaño del nodo, no lo multiplica.** Asignarle `0.94`
a una caja declarada `sx="0.19" sy="0.24" sz="0.19"` la convierte en un cubo de
94 cm, no la achica un 6%. Si querés un pulso sobre el tamaño declarado, tenés
que guardarte las dimensiones originales en el script y multiplicarlas vos.

---

**El audio no avisa cuando no suena.** `new Audio(...)` y `new AudioStream(...)`
se construyen igual sin el permiso `audio`; el rechazo aparece recién cuando el
comando llega al mixer, o sea en el `play()`. Y un `AudioStream` que se queda sin
datos emite silencio sin emitir ningún evento: quedarse corto de muestras se
parece exactamente a una pausa querida. Ver [audio.md](audio.md).

**Las voces mueren con su worker.** El audio pertenece al isolate que lo creó: se
descarta al descargar el espacio, al reiniciar el runtime y al revocarse el
permiso. No hay sonido que sobreviva a una navegación.

**`URL.createObjectURL` no se limpia solo.** 128 URLs vivas y 64 MiB de cuota, y
el `RangeError` aparece en la creación número 129, lejos de la que sobra. Un
`revokeObjectURL` por cada `createObjectURL`.

**`IO.Buffer` y `pipe.write` escriben lo que entra, no lo que les diste.**
Devuelven cuántos bytes tomaron; el `Buffer` no crece y el `pipe` está acotado
por su capacidad. Ignorar ese número pierde la cola del mensaje en silencio.
`writeAll` de `AudioStream` es la única que hace el bucle sola.

**`TextDecoder` no decodifica por partes.** `{ stream: true }` tira `TypeError`,
y con razón: un carácter multibyte partido entre dos trozos no se reconstruye
después. Hay que juntar los bytes y decodificar una vez.

---

**El alfa de un color no se ve sin `material-alpha`.** `color` acepta
`#RRGGBBAA` y `transparent` desde
`feat(ui): support shared CSS-style RGBA colors and transparent surfaces`, pero
un `box` o un `plane` sin atributos de superficie se dibuja con un material
**opaco**: el alfa se parsea, se guarda y no pasa nada. La mezcla la fija el
camino de superficies, y a ese camino se entra declarando `material-alpha`
(`surface.rs:472`; el gate está en `SurfaceDesc::parse`, `surface.rs:70`).

    <plane color="transparent" material-alpha="blend" touchable="true"/>

Eso último es lo que más rinde: **un blanco táctil invisible**. Sigue recibiendo
`toque` y `pointerenter` sin dibujar nada, así que las zonas de impacto dejan de
tener que pintarse del color de lo que tapan.

**~~El ancho del texto es `size · nChars · 0.6`.~~ Ya no.** Era una aproximación
monoespaciada y estaba bien medida en su momento; `add real mesh render` la
reemplazó por layout de verdad con FiraSans, así que `build_text_transform` mide
la cadena con la fuente (`text_texture_size`, `render.rs:66`). Un framework que
todavía multiplique por 0,6 va a acomodar mal — y ahora hay a quién preguntarle:
`TextLayout.create(texto, size)` devuelve `{w, h, glyphs}`. Ver
[interfaz.md](interfaz.md).

---

## Patrones

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
reducir geometría. Ver [`docs/costo_de_montaje.md`](../docs/costo_de_montaje.md).

**`border-radius` existe en `box` y `plane`, y regenera la malla.** Vale
recordarlo porque sin él un panel con esquinas redondeadas hay que armarlo con
dos cajas cruzadas y cuatro cilindros girados, que es lo que uno termina
haciendo si no sabe que está.

**Pero significa dos cosas distintas según la etiqueta, y falla en silencio.**

```rust
// box   — dom.rs:229    [ (r/sx).min(0.499), (r/sy).min(0.499), (r/sz).min(0.499) ]
//         → METROS del mundo, divididos por la escala de cada eje
// plane — shapes.rs:741 let r = radius.clamp(0.0, 0.499); let h = 0.5 - r;
//         → FRACCIÓN del lado, sobre un cuadrado unitario, sin mirar la escala
```

Pasarle metros a un `plane` no da error: da esquinas casi rectas. Sólo se nota
al intentar un círculo, que en `plane` es `0.5` y en `box` sería la mitad del
lado en metros.

Y en un `box` **el radio es uno solo para los tres ejes**, así que una tarjeta
fina no puede tener esquinas grandes: el radio choca contra el espesor y la caja
se vuelve una esfera. Es exactamente lo que le pasa hoy al menú de VR, que usa
una caja de 0,22 con radio 0,2 —`rounded_box_radii(0.2, (0.22,0.22,0.025))` da
`[0.499, 0.499, 0.499]`, los tres ejes saturados—. Para una tarjeta con esquinas
grandes, `plane`. Ver [el diseño del menú VR](disenos/menu-vr.md).

**Los `plane` son de doble cara.** No hace falta duplicarlos ni girarlos para
verlos desde atrás.

**El orden de los `<script src>` no está garantizado.** Se cargan en asíncrono y
no hay `defer`, ni `type="module"`, ni promesa de orden. Con una biblioteca y su
usuario como dos etiquetas separadas, el usuario puede evaluarse primero y morir
con `ReferenceError`, mientras la biblioteca se evalúa bien tres milisegundos
después. Está medido. Las dos salidas: concatenar del lado del servidor, o que
cada script espere en un `requestAnimationFrame` a que aparezca lo que necesita.

**`setTransformBatch` toma 7 floats por nodo**: `[nodeId, px,py,pz, rx,ry,rz, …]`,
en coordenadas **locales**. La escala **no** va en el batch: para eso está el
proxy `el.scale`, que sí cruza el borde una vez por acceso.

---

Volver al [índice de guías](index.md).
