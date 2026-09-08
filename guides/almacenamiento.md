# localStorage en Luna

Todos los isolates de documentos HTTP/HTTPS y de páginas nativas tienen
`localStorage` y `window.localStorage`. No se necesita solicitar una capability.

```js
localStorage.setItem('preferencias', JSON.stringify({ tema: 'verde' }));
const preferencias = JSON.parse(localStorage.getItem('preferencias') || '{}');

localStorage.volumen = 0.8; // Se guarda el texto "0.8".
console.log(localStorage.length, localStorage.key(0));
console.log(Object.keys(localStorage));
delete localStorage.volumen;
localStorage.removeItem('preferencias');
// localStorage.clear(); // Borra únicamente los datos de este origen.
```

Los valores y las claves se convierten a texto. `getItem` devuelve `null` cuando
no existe una clave; el acceso por propiedad devuelve `undefined`. Para objetos
se usa JSON. También se preservan cadenas con NUL, emojis y unidades UTF-16
aisladas. Cambiar un valor no cambia su posición en la enumeración.

## Separación y duración

El origen HTTP/HTTPS es **protocolo + host + puerto**. Las rutas, consultas y
fragmentos no crean almacenes nuevos. Los puertos predeterminados se normalizan:

| Documentos | Almacenamiento |
| --- | --- |
| `https://example.com/a` y `https://example.com:443/b` | Compartido |
| `http://example.com` y `https://example.com` | Separado |
| `https://example.com` y `https://sub.example.com` | Separado |
| `http://localhost:2052/a` y `http://localhost:3000/b` | Separado |
| `luna://home`, `luna://settings` y demás rutas nativas | Un almacén interno común |

El host fija el origen desde la URL base del documento **antes de ejecutar sus
scripts**. El origen de un script externo no cambia el del documento. Modificar
`location`, atributos o variables JS no permite seleccionar otra base de datos
ni leer otro origen. Los documentos sin origen admitido, incluidos `data:` y
`file:`, reciben `SecurityError` al acceder a localStorage.

Los datos sobreviven al cierre del isolate, navegación y reinicio de Luna. Se
guardan en `local-storage.sqlite3` dentro del directorio de estado del navegador;
en Windows: `%APPDATA%\LunaBrowser\local-storage.sqlite3`. No pertenecen a la
caché HTTP. SQLite puede crear archivos auxiliares `-wal` y `-shm` junto a la base.

## Cuota, concurrencia y errores

Cada origen tiene una cuota de **5 MiB**, medida sobre sus claves y valores
codificados como cadenas JSON UTF-8. Si se supera, `setItem` lanza
`QuotaExceededError` y conserva el valor anterior. Los errores de disco/base de
datos producen `InvalidStateError`; no se convierten silenciosamente en un
almacén temporal.

Las operaciones son síncronas, en el worker del isolate, fuera del hilo de
render. Cada conexión se abre al primer acceso. Las transacciones SQLite hacen
que una escritura y su comprobación de cuota sean atómicas, incluso con varios
isolates o procesos. Como en Web Storage, la secuencia leer-modificar-escribir
desde JavaScript no es una transacción: dos páginas pueden leer el mismo contador.

## Eventos

```js
addEventListener('storage', event => {
  console.log(event.key, event.oldValue, event.newValue, event.url);
  console.log(event.storageArea === localStorage); // true
});
// También se admite: window.onstorage = event => { ... };
```

Los cambios se notifican a los **otros isolates vivos del mismo origen** en el
mismo proceso de Luna. El emisor no recibe su propio evento. Escribir el mismo
valor, borrar una clave inexistente o vaciar un almacén ya vacío no genera eventos.
Para `clear`, key/oldValue/newValue son null. Los listeners admiten removeEventListener
y `{ once: true }`. La cola de notificaciones se consulta solo mientras hay listeners;
retirar el último listener cancela su temporizador. No se consulta SQLite para
recibir eventos.

Los datos son compartidos también entre procesos, pero las notificaciones
`storage` actualmente solo se distribuyen dentro de un proceso. Esto no añade
sessionStorage ni IndexedDB.

Referencia de la API: [HTML Standard — Web Storage](https://html.spec.whatwg.org/multipage/webstorage.html).

---

Volver al [índice de guías](index.md).
