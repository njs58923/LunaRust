# Propuesta: canal de componentes para includes

Estado: **diseño para revisión; todavía no implementado ni parte de la API pública**.

## Base actual

- `crates/luna/src/js.rs::build_local_space_snapshot_from_mirror` crea handles locales y sólo conserva relaciones con nodos del ámbito permitido. No conviene ampliar el snapshot para entregar el DOM del padre al hijo.
- El bus `ShellMessage` de `js_runtime/src/lib.rs` y su router en `luna/src/js.rs` está ligado a tabs y al shell. Requiere `UX_EMBED`; una app sólo puede escribir al shell. Reutilizar esa autorización para componentes ampliaría su alcance y mezclaría contratos distintos.
- `permissions.rs` corta permisos al cruzar un include; `resources` delega un techo de capacidades que el espacio hijo debe solicitar. Esa regla debe conservarse.
- `location.search` ya sirve para parámetros de URL. Es configuración de carga: no ofrece estado reactivo, tipos ni notificación al cambiar.

## Declaraciones posibles

| Forma | Ventaja | Límite |
|---|---|---|
| `src="player.hsml?volume=0.3"` | Compatible con URLs y servicios externos | Texto, visible en logs/cachés; cambiar src implica otra carga |
| `prop-volume="0.3"` | Cómodo para valores pequeños en marcado | Necesita reglas explícitas de tipos; objetos/listas y nombres con guiones complican el contrato |
| `props='{"volume":0.3}'` | JSON tipado, atómico, portable | Hay que escapar XML en atributos; poco cómodo para objetos grandes |
| `include.props = {...}` | Encaja con un renderer estilo React y cambios dinámicos | Requiere el puente host y snapshots; no puede pasar funciones ni objetos vivos entre isolates |
| Mensajes genéricos | Útiles para comandos y solicitudes puntuales | Sin estado inicial ni revisiones; obliga a reconstruir props/handshakes en cada componente |

Recomendación inicial: **JSON en marcado + propiedad JS `props`**, ambos con la misma semántica de reemplazo. Mantener las queries para identificar/configurar recursos del servidor. Dejar el azúcar `prop-*` para después y evitar interpolación/eval de expresiones en HSML.

```html
<include
  id="musica"
  src="/components/player.hsml"
  props='{"title":"Ambiente","volume":0.3}'
  events="volumechange,ended,error"
  resources="audio,fetch_text"/>
```

```js
// Padre
const player = hiperspace.dimention.getElementById('musica');
player.props = {...player.props, volume:0.5};
player.addEventListener('component:volumechange', event => {
  const value = event.detail.volume;
  if (typeof value === 'number' && value >= 0 && value <= 1) {
    player.props = {...player.props, volume:value};
  }
});
```

```js
// Isolate del documento cargado: API propuesta
render(component.props); // props iniciales disponibles ANTES de ejecutar scripts
component.addEventListener('propschange', event => {
  render(event.detail.props);
});
await component.emit('volumechange', {volume:0.5});
```

`props` es un snapshot de sólo lectura en ambos lados. Para actualizarlo, el padre asigna un objeto nuevo; mutar `player.props.volume` no actualiza nada. El hijo tiene propiedad getter sin setter. Un cambio observado incluye props, previous, revision y changedKeys. Coalescer varios reemplazos pendientes entrega el último snapshot completo, sin perder campos ni formar estados intermedios parciales.

El evento expresa una intención: el padre puede aceptarla, corregirla o ignorarla. No modifica automáticamente props ni atributos del padre. `component.emit()` confirma aceptación por el router, no éxito de un handler ni ejecución de una acción. Si luego hace falta request/response, añadir `requestId`, respuesta y timeout como otra operación explícita.

## Permisos

1. Leer las props que el padre entregó es parte del montaje del componente, no un permiso elevado. Se permite incluso entre orígenes: el padre eligió transmitir esos datos a ese include.
2. `events` es la lista que el padre acepta recibir. V1: sin lista no se permiten eventos hacia arriba, tampoco por ser del mismo origen. El padre puede ampliarla explícitamente. No admitir `*` por defecto.
3. El canal sólo une el padre propietario del include y el endpoint público del documento cargado. No entrega `parent.document`, IDs globales, acceso a hermanos, abuelos o al shell.
4. `resources` sigue siendo independiente. Recibir `requestnavigate` no autoriza al hijo a navegar: el padre valida destino/datos y decide si ejecuta su propia API. Estos eventos tampoco se convierten en clics confiables ni llevan activación del usuario implícita.
5. El motor determina emisor, receptor, origen efectivo, instancia y generación. El payload no puede declararlos ni falsificarlos. Las propiedades reservadas deben ir en metadatos separados de `detail`.
6. Fijar el origen esperado al `src` declarado. Una redirección o navegación a otro origen desconecta el canal antes de entregar props; debe actualizarse la declaración para aceptar otro origen. Un reload del mismo origen crea una generación nueva e invalida pendientes de la anterior.
7. La lista se valida en Rust, no solamente en el wrapper JS. El hijo no puede modificarse la autorización alterando su propio DOM. El mismo origen no implica más capacidades.

No recomendaría un permiso global `communicate_with_parent` ni `ux_embed`: el montaje y su lista de eventos ya son una concesión acotada y revisable. Si más adelante se permite mensajería entre componentes no relacionados, eso sí requiere otro contrato de permisos.

## Identidad y jerarquía

El padre es el isolate **que posee el nodo include**, no el grupo transform más próximo, un ID público ni una búsqueda por nombre. El host guarda la relación al completar el montaje. Así sobreviven wrappers/grupos y pueden coexistir muchas puertas con el mismo archivo.

V1 debe exigir un único espacio raíz público en el documento incluido. Si hay múltiples espacios raíz, emitir un diagnóstico y no elegir uno arbitrariamente; una extensión posterior puede declarar el endpoint público. Los espacios internos no obtienen automáticamente autoridad para hablar como el componente. Un include anidado abre su propio canal hacia su propietario, sin saltar al abuelo.

Al reparentar, remover, cambiar src, reiniciar un worker o revocar la lista, cerrar/revalidar el canal en el host. No reenviar mensajes pendientes a una instancia nueva que reutiliza el mismo nodo o ID.

## Robustez y coste

- Sobre por mensaje: identidad host de la instancia, generación, revisión/sequence, clase (props/event) y payload.
- Payload V1: JSON estricto —null, boolean, números finitos, strings, listas y objetos de datos— sin funciones, ciclos, DOM, handles nativos ni permisos. Validar profundidad y tamaño, y evitar merges con efectos sobre prototipos.
- Límites iniciales a ajustar con mediciones: 64 KiB por mensaje/props, profundidad 32, 64 eventos pendientes por canal y 1 MiB agregado por isolate. Límite también en canales activos.
- Props: conservar sólo el último snapshot completo pendiente. Eventos: FIFO por instancia/generación, sin descarte silencioso; cola llena rechaza `emit` con error explícito.
- Entrega con mensajes al worker y `needs_tick`, aprovechando el scheduler existente; nada de un intervalo por componente ni reconstrucción del DOM completo.
- Props iniciales en la configuración/snapshot previo al primer script. Registrar listeners temprano; no reproducir eventos antiguos a nuevos listeners. Cancelar lo pendiente al desconectar.
- `component:*` usa un namespace reservado y por defecto no burbujea fuera del nodo include. Nunca inyectar `toque`, `pointerenter`, mensajes del shell o eventos marcados como confiables.
- Diagnósticos MCP: conectado/cerrado, orígenes, generación, revisión, tamaños de cola y motivo de rechazo; no registrar contenido sensible de props por defecto.

## Implementación sugerida

1. Tipos `ComponentBinding`/`ComponentEnvelope` en Rust y registro por montaje, sin exponer IDs globales a JS.
2. Parseo/validación de `props` y `events`; binding entre propietario y raíz pública al cargar un include.
3. Snapshot inicial al crear el worker; op de actualización del padre, op de emisión del hijo y un inbox independiente del bus del shell.
4. Wrapper `include.props`, `component.props`, `propschange` y eventos `component:*`; ACK/errores con límites acotados.
5. Invalidación al descargar/reparentar/navegar, integrada con el índice incremental y las generaciones de workers.
6. Pruebas: datos iniciales antes del primer script, muchas instancias del mismo src, cambio de props durante carga, reload con eventos en vuelo, mismo/origen distinto, origen redirigido, suplantación, evento no declarado, cola llena, padre lento, hijo cerrado y rechazo de objetos no serializables.

Una segunda fase podría añadir structured clone de ArrayBuffer/Blob con transferencia y cuotas. No enviar PCM por el canal de props: usar un puerto de datos dedicado con contrapresión y transferencia explícita; los props sólo configuran el reproductor.
