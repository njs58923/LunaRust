# Componentes: hablar con un `<include>`

Un `<include>` es **otro documento adentro**, con su propio isolate y sus propios
permisos. Eso lo hace un componente de verdad, y también lo dejaba mudo: el padre
podía ver sus nodos con `getElementById` y escribirles atributos, pero **el toque
nunca cruza el borde** —se despacha sólo al espacio dueño del nodo— así que el
hijo no tenía forma de avisar nada hacia arriba.

El canal cierra eso con dos atributos: **`props`** baja datos y **`events`**
declara qué puede emitir el hijo.

```xml
<include id="puerta_cueva" src="./puerta.hsml" events="abrir"
         props='{"titulo":"La cueva","roca":"#4A515C"}'/>
```

```js
// El padre
const p = hiperspace.dimention.getElementById('puerta_cueva');
p.addEventListener('component:abrir', () => {
  p.props = { ...p.props, estado: 'abriendo' };   // la vuelta
  location.href = './cueva.hsml';                 // lo que el hijo no puede hacer
});
```

```js
// El hijo
render(component.props);                          // ya están, antes del primer script
component.addEventListener('propschange', e => render(e.detail.props));
component.emit('abrir', { id: 'cueva' });
```

**Lo que el canal no arregla: un include sigue sin poder navegar** el documento
que lo contiene. `location.href` adentro de un include navega el include, y el
destino aparece dentro del marco. Por eso el patrón es siempre el mismo — el
componente avisa, el padre actúa.

> Un caso entero, con veintiocho instancias del mismo archivo: las puertas del
> atrio en `server_noche` (`public/puerta.hsml` + `src/atrio.ts`). Antes los datos
> viajaban en la query, así que eran veintiocho URLs distintas del mismo
> documento, y el vano tocable lo tenía que declarar el atrio porque el toque no
> llegaba. Hoy son **una URL** y cada puerta es una puerta entera.

Demo mínima: `luna://component_demo` (requiere recompilar Luna).

## El contrato

Se usa JSON estándar en el atributo `props`. Evita otro lenguaje de valores, conserva tipos y no evalúa código. Para datos grandes o dinámicos, asignar un objeto desde JavaScript. No se admite sintaxis CSS ni interpolación de expresiones.

```html
<include id="musica" src="/components/player.hsml"
  props='{"title":"Ambiente","volume":0.3}'
  events="volumechange,ended,error"
  resources="audio,fetch_text"/>
```

Como en cualquier atributo HSML, escapar los caracteres XML (`&amp;`, `&lt;`, etc.). Las queries de `src` siguen siendo configuración de carga; las props son datos locales y no se agregan a la URL ni se envían al servidor.

```js
// Padre: el include pertenece a su propio isolate.
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
// Hijo: props iniciales disponibles antes del primer script.
render(component.props);
component.addEventListener('propschange', event => render(event.detail.props));
await component.emit('volumechange', {volume:0.5});
```

- `include.props` lee un snapshot congelado. Asignar un objeto nuevo reemplaza todas las props, también mediante `setAttribute('props', textoJSON)`. Las asignaciones consecutivas a `.props` pueden leer su último valor local antes del próximo snapshot.
- `component.props` es de sólo lectura. `component.connected` indica conexión y `component.revision` la revisión del snapshot (0 desconectado, 1 al conectar).
- `propschange` entrega `event.detail = {props, previous, revision, changedKeys}`. Varios reemplazos pendientes se agrupan en el último estado completo. No se emite un cambio inicial: el primer render debe leer `component.props`.
- `connect` y `disconnect` notifican transiciones observadas mientras el worker vive. No se garantiza un callback de despedida al destruir el isolate. Un cambio de generación puede observarse directamente como `connect` y `propschange`.
- `component.emit(nombre, datos)` devuelve una Promise: confirma encolado, no ejecución ni éxito del handler del padre. Rechaza eventos no autorizados, datos inválidos, desconexión o cola llena. Los mensajes aceptados pero pendientes se cancelan al cerrar el canal.
- El padre escucha `component:nombre` **en ese include**. El evento contiene `detail`, `origin`, `generation`, `sequence`, `isTrusted:false` y `bubbles:false`. El host establece los metadatos; los datos del hijo quedan dentro de `detail`, congelados. No hay burbujeo ni activación de usuario implícita.
- Ambos objetos admiten `addEventListener`/`removeEventListener`; en `component` se reciben callbacks de función.

## Declaración y permisos

El canal se activa declarando `props` o `events`. No requiere `ux_embed` ni un permiso global. El mismo origen y los orígenes externos siguen las mismas reglas:

1. Las props son datos que el padre decide entregar al documento incluido.
2. `events` declara exclusivamente los nombres que el hijo puede emitir. Sin lista no hay eventos hacia arriba. Admite hasta 32 nombres separados por comas o espacios, con formato `[a-z][a-z0-9_-]{0,63}`; no admite `*`.
3. `resources` mantiene su contrato de delegación. El canal no concede navegación, audio, red ni acceso al DOM del padre. Recibir una intención no obliga al padre a ejecutarla: debe validar sus datos antes de usar sus propias capacidades.
4. Sólo el host enlaza los endpoints. JS no puede seleccionar otro padre, hermano, abuelo ni shell mediante un ID.
5. El documento incluido necesita exactamente **un espacio raíz público**. Puede contener grupos intermedios y espacios internos; éstos no heredan el endpoint. Un include anidado tiene su propio canal con el espacio propietario inmediato.
6. El origen declarado en `src` debe coincidir con el del documento cargado. **Los includes rechazan ahora redirecciones HTTP a otro origen**, tanto en carga inicial como diferida, incluso si aún no declararon props. Para ese caso hay que poner explícitamente la URL de destino en `src`. Una redirección dentro del mismo origen se admite.

El padre es el espacio propietario del nodo include, no su grupo transform más cercano ni su ID de texto. Varias instancias del mismo archivo reciben props y colas independientes.

## Ciclo de vida y límites

Cambiar `src`, la declaración `events`, el padre directo del include o sus workers invalida el canal y crea otra generación. Remover el include, comenzar su recarga o detener un worker cancela los mensajes pendientes. La generación se comprueba también al entregar eventos: un handle reutilizado no recibe eventos antiguos.

V1 transporta JSON estricto: objetos de datos, arrays densos, strings, números finitos, booleanos y null. Props exige un objeto raíz. No admite funciones, undefined, símbolos, accessors, ciclos, Date, Blob, buffers ni handles nativos. Para audio/PCM usar las APIs de audio/IO; este canal configura al reproductor, no transporta muestras.

- 64 KiB UTF-8 por props/evento; profundidad máxima 32.
- 64 eventos pendientes por canal, FIFO dentro de cada instancia/generación.
- 1 MiB agregado por padre de datos serializados y coste de sobre estimado; no representa una medida exacta del heap.
- 128 canales hijos por worker. Cola llena produce rechazo explícito; no hay reintentos automáticos.
- Props conserva el último estado, sin acumular una cola de reemplazos.

El transporte usa puertos Rust enlazados por el host y despierta el scheduler existente mediante `needs_tick`. No crea timers por componente ni amplía los snapshots con DOM ajeno. El binding examina workers y límites de documentos; no recorre las mallas de cada componente.

Los logs `[component]` muestran conexión, cierre, generación, origen y motivos de rechazo del montaje sin imprimir las props. Los rechazos de `emit` se entregan al llamador. Todavía no hay contadores de cola dedicados en `luna_status`.

## Validación y alcance

Pruebas de runtime y montaje nativo cubren props antes del primer script, cambios agrupados, orden, metadatos, aislamiento de instancias, generaciones, recarga de misma URL, raíces ambiguas, espacios privados, origen incorrecto, eventos no declarados y límites de datos/colas.

La demo usa dos instancias del mismo contador. Cada hijo emite una intención; el padre incrementa su estado y devuelve nuevas props. No necesita permisos elevados.

Quedan fuera de V1: RPC/request-response, transferencia de Blob/ArrayBuffer, callbacks entre isolates y validación de esquemas de negocio. Se pueden añadir posteriormente sin cambiar el contrato de props y eventos.

---

Volver al [índice de guías](index.md).
