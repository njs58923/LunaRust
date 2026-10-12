# Anclajes nativos

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

Para otros proveedores nativos, Rust puede usar `NativeReferences::register`,
`set_target` y `restrict`. Un slot sin destino es conocido pero indisponible;
un nombre no registrado rechaza la creación.

## Input de VR

El root monta `luna://vr_input` en VR y lo desmonta al pasar a escritorio,
independientemente del menú curvo/plano. Ese espacio pide `vr_input,create_anchor`
y publica `left-grip` y `right-grip` desde sus referencias privadas:

- `__vr_left_grip_root`
- `__vr_right_grip_root`

La parte Rust vive en `vr_input.rs`; la declaración JS está en
`web/input/vr_input.hsml`. El servicio genérico de anchors no conoce esos nombres
ni consulta OpenXR. El input prefiere el pivote de tracking real a la entidad
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

## Verificar

`luna://anchor_demo` muestra dos objetos asociados a las manos y no tiene
scripts. El input de VR se encarga de publicar las fuentes.

```powershell
cargo test -p js_runtime --test anchors_test
cargo test -p luna --lib anchors::tests
cargo test -p luna --lib anchor_ack
cargo check -p luna --bins
```

Los tests verifican el transform en el mismo frame, offsets e hijos, tracking
perdido/recreado, flags inválidos, permisos, referencias privadas, colisiones,
fuentes DOM ajenas, ciclos, desanclaje, cierre y confirmaciones bajo saturación.
La lógica del juego y sus colisiones en JS siguen usando su propio stream;
esta función elimina el recorrido JS de la colocación visual.
