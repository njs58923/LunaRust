# Anclajes

Un anclaje hace que un elemento siga la pose de otro (un mando, otro elemento
o el resultado de un programa) desde el renderer, en el mismo frame, sin
escribir poses desde JavaScript. Para combinar varias poses (un arma a dos
manos, un arco) ver [programas-de-movimiento.md](programas-de-movimiento.md).


El framework permite publicar la transformación de un elemento HSML o una
referencia nativa de Bevy y asociarle elementos con `anchor="key"`. Después
del registro, el seguimiento ocurre en el renderer sin escrituras JS por frame.

## Publicar

El espacio creador debe pedir y recibir el recurso `create_anchor`
(`CapabilityBits::CREATE_ANCHOR`). Si está dentro de un include, el include
debe delegar también ese recurso.

```xml
<space resources="create_anchor">
  <group id="plataforma">...</group>
  <script>
    (async () => {
      const root = hiperspace.dimention;
      const handle = await dimension.anchors.create(
        'plataforma', root.getElementById('plataforma')
      );
    })().catch(console.error);
  </script>
</space>
```

`create()` devuelve una Promise que confirma el registro o rechaza con un error
del host. La confirmación no exige que la fuente tenga tracking disponible.
También existe `hiperspace.dimention.anchors`.

El handle tiene `key` y `dispose()`, que devuelve otra Promise. Un handle viejo
no puede retirar una publicación reemplazada. `dimension.anchors.remove(key)`
retira explícitamente la publicación actual del espacio. Eliminar el wrapper JS
o colectarlo no retira la publicación: su propietario es el espacio.

Las keys admiten letras/números ASCII y `_`, `-`, `.`, `:`, hasta 128 bytes.
Una creación con la misma key reemplaza la fuente del mismo propietario;
otro espacio no puede reemplazarla ni retirarla. Hay hasta 64 publicaciones por
espacio y 64 solicitudes JS pendientes. Las confirmaciones se conservan si la
cola del worker está llena y despiertan su próximo pump.

## Referencias internas

```js
await dimension.anchors.create('right-grip', '__vr_right_grip_root');
```

Los strings que empiezan por `__` son referencias nativas, no selectores DOM.
Rust las registra en `NativeReferences`, separado de `SpaceHandleTables` y de
`getElementById`. El servicio de anchors resuelve el nombre una vez dentro del
espacio creador y conserva un ID de slot. El proveedor puede renovar el destino
Bevy del slot cuando su entidad se recrea.

Un nombre no registrado rechaza la creación; una referencia conocida sin
destino queda indisponible hasta que lo tenga.

## Input de VR

El root monta `luna://vr_input` en VR y lo desmonta al pasar a escritorio,
independientemente del menú curvo/plano. Ese espacio pide `vr_input,create_anchor`
y publica `left-grip` y `right-grip` desde sus referencias privadas:

- `__vr_left_grip_root`
- `__vr_right_grip_root`

El input prefiere el pivote de tracking real a la entidad
visual del mando y atiende pérdida/recreación de la sesión. Un pivote con flags
de tracking inválidos queda indisponible, sin recurrir a una pose visual vieja.

Los consumidores de estas fuentes deben tener `read_pose_stream`. Los
proveedores genéricos pueden especificar sus propias capacidades requeridas;
un anchor de una plataforma normal no requiere permisos de VR.

## Consumir

```xml
<space resources="read_pose_stream">
  <group anchor="right-grip" x="0" y="0" z="0" rx="0" ry="0" rz="0">
    <box y="0.1" sx="0.06" sy="0.3" sz="0.06" color="#3080FF"/>
  </group>
</space>
```

La posición, rotación y escala declaradas del elemento son offsets del anchor.
Se aplica la transformación mundial completa de la fuente. Los hijos conservan
sus transforms locales normales. No hay que seguir escribiendo la pose mundial
desde `posemove`: esas escrituras modificarían el offset.

El padre lógico y la propiedad DOM se conservan. Rust actualiza los transforms
globales del elemento y sus hijos después de la propagación de transforms y
antes de frusta/culling y extracción de render. Esto evita depender del tick
del worker y evita que el despawn recursivo de un mando destruya una escena ajena.

Si falta la publicación, tracking o permiso, el subtree queda indisponible
visualmente sin destruir sus entidades. Se reconecta al volver la fuente.
Los ciclos de dependencias permanecen indisponibles y no se evalúan en bucle.
Retirar el atributo `anchor` restaura el transform local respecto del padre
original; no hay una operación automática para conservar la pose mundial al
desanclar.

El registro se libera al cerrar el espacio o revocar `create_anchor`. Las
referencias nativas también se liberan con el espacio. La desaparición de una
fuente deja sus consumidores ocultos hasta que vuelva o se cambie el vínculo.

## Probar

`luna://anchor_demo` muestra dos objetos asociados a las manos y no tiene
scripts: el input de VR se encarga de publicar las fuentes.


## Escuchar la pose efectiva

```js
const listener = dimension.anchors.listen(
  ['left-grip', 'right-grip', root.getElementById('sable')],
  frame => {
    // timestamp comparte el origen y unidades de performance.now().
    // Todos los samples corresponden al mismo frame de render.
    for (const sample of frame.samples) {
      if (!sample.available || frame.discontinuous) reiniciarBarrido(sample.index);
      if (sample.available) actualizarLogica(sample, frame.timestamp);
    }
  },
  { priority: 'input', delivery: 'history' }
);
listener.dispose();
```

Una key entrega la pose mundial de la fuente publicada. Un elemento resuelto
entrega su pose efectiva de render, incluidos sus offsets y anchors heredados.
Solo se pueden escuchar elementos del propio espacio. Las referencias `__...`
son privadas del proveedor: para escuchar desde una aplicación se usa la key.
Se exige `read_pose_stream` para acceder a fuentes nativas de los mandos,
incluso cuando se las escucha mediante un elemento o un anchor DOM intermedio.

El callback recibe `{ frameId, timestamp, discontinuous, samples }`. Cada muestra
contiene `{ index, target, available, position, rotation, scale, error, input }`.
Position y scale son `{x,y,z}`; rotation es un cuaternión `{x,y,z,w}`.
Las poses indisponibles no incluyen información anterior del mando. El proveedor
VR adjunta opcionalmente `input: {trigger, grip, primary, secondary}`, obtenido
de la misma sincronización OpenXR. Las fuentes genéricas pueden omitirlo.

`priority: 'input'` despierta al worker por un buzón independiente y procesa sus
callbacks antes del siguiente comando pendiente, con el watchdog habitual.
No necesita un nuevo Tick ni dispara RAF; no interrumpe JavaScript ya ejecutándose.
Las escrituras al DOM producidas por el callback siguen pasando por el commit
normal del host. La colocación visual de los anchors permanece nativa.
`priority: 'normal'` (predeterminado) entrega antes del RAF en el siguiente Tick.

`delivery: 'latest'` (predeterminado) conserva la última muestra por suscripción.
`history` conserva hasta 64 frames y los entrega en orden, con un callback por
frame. El primer frame, cambios de disponibilidad y pérdidas por saturación se
marcan `discontinuous`; al coalescer se conserva esta señal. La revocación de
permisos descarta también las poses pendientes. Hay hasta 16 suscripciones de
16 destinos por worker. Dispose detiene la captura; cerrar el espacio destruye
el buzón. Los espacios sin listeners no generan muestras.

## Ejemplos en los demos

Sables escucha grips y hojas con historial para recorrer todos los tramos del
corte; la demo desancla las hojas. Tiro escucha ambas manos juntas y conserva
historial para no perder flancos de los gatillos ni las muestras de su filtro de
puntería. Armas y cargadores libres en la mano siguen al grip nativo; el agarre a
dos manos es un [programa](programas-de-movimiento.md), y el retroceso y los
mecanismos son offsets locales calculados en JS.
