# Dirección del documento y parámetros

Cada isolate recibe del host la URL del documento que contiene su `<space>`.
Un componente con su propio `<space>` dentro de un include recibe la URL de ese
documento incluido, con query y fragmento. No recibe la URL del mundo padre ni
la de su archivo `.js`. Un include sin un space propio no crea un isolate nuevo;
sus scripts siguen perteneciendo al space contenedor.

```xml
<include src="https://portals.test/puerta.hsml?uuid=abc&amp;titulo=La%20m%C3%A1quina&amp;spawn=llegada"/>
```

En el script del `<space>` de `puerta.hsml`:

```js
const params = new URLSearchParams(location.search);
const uuid = params.get('uuid');       // "abc"
const titulo = params.get('titulo');   // "La máquina"
const spawn = params.get('spawn');     // "llegada"
console.log(location.href, location.origin, location.pathname);

const destino = new URL('../mundo.hsml', location.href);
destino.searchParams.set('tema', 'noche');
destino.hash = 'entrada';
location.assign(destino.href);
```

## Location

`location`, `window.location` e `hiperspace.location` comparten el mismo objeto.
Están disponibles `href`, `origin`, `protocol`, `host`, `hostname`, `port`,
`pathname`, `search`, `hash`, `toString()`, `assign()`, `replace()` y `reload()`.
`search` incluye el `?`; `hash`, el `#`; ambos son cadenas vacías cuando no hay
contenido. Los valores mantienen la codificación URL; `URLSearchParams` decodifica
los parámetros, incluyendo Unicode, nombres repetidos y espacios escritos como `+`.

Asignar `href`, `window.location` o un componente de la dirección solicita una
navegación. Los enlaces relativos se resuelven contra la URL del documento actual.
El host sigue comprobando `navigate_self` / `navigate_global`: leer una URL no
otorga permisos para navegar. La propiedad conserva la dirección comprometida
hasta que se carga el destino; no simula éxito si el host bloquea la solicitud.
La identidad utilizada para permisos y almacenamiento nunca se toma de una
variable JavaScript modificable.

`reload()` vuelve a cargar el include que corresponde al documento. Solicitudes
idénticas que ya están en vuelo se agrupan. `replace()` utiliza por ahora el mismo
flujo que `assign()`: Luna todavía no implementa la distinción de historial del
navegador. Cambiar `hash` también usa la navegación de documentos actual; no se
implementaron `hashchange` ni navegación de fragmentos sin recarga en esta etapa.

En una puerta anidada, `navigate_self` sigue navegando esa puerta, no el mundo
contenedor. `navigate_world`, exportación de spawns y retorno siguen pendientes.

## URL y URLSearchParams

`URL` admite URLs absolutas y resolución con base, componentes de dirección,
`origin`, `searchParams`, `toString()`, `toJSON()` y `URL.canParse()`. Sus parámetros
son vivos: modificar `url.searchParams` actualiza `url.search`, y viceversa.

`URLSearchParams` admite cadenas, objetos y pares iterables; `get`, `getAll`,
`has`, `set`, `append`, `delete`, `sort`, `size`, `forEach` e iteradores.
No se implementan URLs de objetos (`createObjectURL`), ni se pretende cobertura
completa de todas las interfaces y casos límite del navegador.

Las rutas `luna://` también pueden consultarse. Como esquema personalizado,
`URL.origin` / `location.origin` devuelven `"null"`; la política interna de Luna
sigue usando su propia identidad `luna://internal` para almacenamiento.

## Pruebas

`cargo test -p js_runtime --lib` cubre parámetros, codificación, URLs relativas,
sincronización de URL/searchParams, identidad por isolate y navegación pendiente.
La suite de Luna cubre invalidar una carga completada para recargarla sin reiniciar
una petición idéntica todavía en vuelo.
