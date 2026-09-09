# Fetch HTTP

Para leer, el documento pide `resources="fetch_text"` (GET/HEAD).
Para usar también POST, PUT, PATCH, DELETE u OPTIONS pide `resources="fetch_http"`.
El shell ofrece ambos permisos a mundos espaciales; cada documento debe pedirlos
explícitamente. Un include requiere delegación del padre y petición del hijo.
Los grants personalizados pueden excluirlos. Una app montada por el shell recibe
`fetch_text` entre sus concesiones predeterminadas, pero igual tiene que pedirlo.

```xml
<space resources="fetch_http">
  <script src="./puerta.js"/>
</space>
```

```js
const response = await fetch('./api/puertas', {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'Authorization': 'Bearer ' + credencial
  },
  body: JSON.stringify({ uuid, titulo: 'La máquina' })
});
const resultado = await response.json();
if (!response.ok) {
  console.error('HTTP', response.status, resultado);
} else {
  console.log(response.status, response.headers.get('x-revision'), resultado);
}
```

## Contrato

- Input: cadena o `URL`. Las rutas relativas se resuelven contra el documento
  del isolate, incluido el documento de un componente externo.
- Métodos HTTP; CONNECT, TRACE y TRACK se rechazan. GET/HEAD no aceptan cuerpo.
- `headers`: objeto, pares o `Headers`, con nombres insensibles a mayúsculas.
- `body`: cadena UTF-8, `URLSearchParams`, `Blob`, `ArrayBuffer` o una vista.
  JSON requiere `JSON.stringify` y `Content-Type: application/json`. Un formulario
  recibe content-type urlencoded; las demás cadenas reciben text/plain si no se
  especificó otro tipo. Un `Blob` con `type` aporta el content-type; un
  `ArrayBuffer` no aporta ninguno, hay que ponerlo a mano.
- Respuesta: `status`, `statusText`, `ok`, `url`, `redirected`, `headers`,
  `bodyUsed`, `clone()` antes de consumir el cuerpo, y para el cuerpo
  `text()`, `json()`, `arrayBuffer()`, `bytes()` y `blob()`.
- 4xx/5xx resuelven la promesa con `ok: false`. Fallos de transporte, permisos,
  validación o límites rechazan con `TypeError`. JSON inválido rechaza `json()`.
- El cuerpo se consume una vez; usar `clone()` si se necesitan dos lecturas.
- `redirect: 'follow'` (default) o `'error'`. Máximo cinco saltos. POST cambia a
  GET ante 301/302; 303 cambia a GET salvo GET/HEAD; 307/308 conservan método y cuerpo.
  No se reintentan escrituras automáticamente fuera de esas redirecciones.

## URLs `blob:`

`fetch` resuelve una URL de `URL.createObjectURL` **sin salir del isolate y sin
tocar la red**, así que no necesita ningún permiso. Sólo `GET`; el content-type
sale del `type` del blob. Ver [binario.md](binario.md).

## Alcance y límites

Las peticiones remotas siguen restringidas al mismo esquema, host y puerto.
Cada salto se valida antes de enviarlo, también con Authorization. No se aceptan
credenciales embebidas en la URL. Las cabeceras de transporte y de identidad del
navegador (Host, Content-Length, Cookie, Origin, Referer, Sec-*, Proxy-*, etc.)
no pueden suministrarse desde JS. Authorization y cabeceras de la aplicación sí.
No se registran cuerpos ni cabeceras de autorización en los logs de fetch.

Límites: cuerpo de petición 1 MiB —de bytes, sea texto o binario—, respuesta 8 MiB recibidos, 128 cabeceras
de petición / 64 KiB y timeout global de 30 s. Se conservan el límite de peticiones
pendientes por espacio y la cancelación al descargarlo. Las respuestas HTTP
aparecen con su código en los diagnósticos de red.

Las respuestas se almacenan completas en memoria; nada se entrega por partes.
`text()` y `json()` las decodifican como UTF-8 (con reemplazo, no fatal, y
sacando el BOM); `arrayBuffer()`, `bytes()` y `blob()` entregan los bytes tal
cual llegaron, BOM incluido. No hay todavía streams, FormData, Request,
constructor Response, AbortSignal, modo manual de redirects ni caché
configurable. Las opciones no
soportadas se rechazan, no se ignoran. No hay cookies automáticas: sólo se admite
`credentials: 'omit'`, que también es el comportamiento predeterminado; las
cabeceras Set-Cookie no se exponen. `statusText` usa la razón canónica del código.

El origen nativo conserva GET/HEAD de rutas `luna://`; no se habilita escritura
sobre esas rutas. Sus peticiones HTTP también verifican el origen en redirects.

Esto no implementa el servicio de puertas ni la bóveda de credenciales.

---

Volver al [índice de guías](index.md).
