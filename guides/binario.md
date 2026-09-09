# Bytes: Blob, texto y buffers

Todo esto es **global y sin permisos**: es cómputo dentro del isolate, no acceso
a nada. Lo que sí pide permiso es lo que se hace con los bytes
([`fetch`](red.md), [audio](audio.md)).

---

## `TextEncoder` / `TextDecoder`

```js
const bytes = new TextEncoder().encode('así');      // Uint8Array(4)
const texto = new TextDecoder().decode(bytes);
```

Sólo **UTF-8**. Pedir otra etiqueta tira `RangeError`; no es un alias que caiga
en UTF-8 por las suyas.

- `decode(bytes, { stream: true })` **no está implementado** y tira `TypeError`.
  Para decodificar por partes hay que juntar los bytes primero, porque un
  carácter multibyte partido al medio no se puede reconstruir después.
- `fatal: true` rechaza bytes inválidos; por defecto se reemplazan con `U+FFFD`
  en silencio.
- `ignoreBOM` por defecto es `false`, que en esta API —como en la web— significa
  que **el BOM se saca**. Con `true` queda como carácter.
- `encodeInto` existe pero recorre carácter por carácter: sirve para completar
  un búfer, no para codificar cantidad.

## `Blob`

```js
const b = new Blob([bytes, 'texto', otroBlob], { type: 'audio/wav' });
b.size; b.type;
await b.arrayBuffer(); await b.bytes(); await b.text();
b.slice(0, 100, 'application/octet-stream');
```

Las partes se copian al construir: el `Blob` es inmutable y no referencia los
buffers originales. Las cadenas se codifican en UTF-8.

**Un `Blob` no puede pasar de 8 MiB**, y el chequeo es acumulativo mientras se
arman las partes: pasarse tira `RangeError`.

### URLs de objeto

```js
const url = URL.createObjectURL(blob);   // blob:<origen>/luna-1
const r = await fetch(url);              // GET, sin permiso de red
URL.revokeObjectURL(url);
```

La resolución es **local al isolate**: no hay registro global de blobs, y una URL
`blob:` no viaja a otro espacio ni a un `<include>`. Sólo acepta `GET`;
cualquier otro método tira `TypeError`.

Cuota: 128 URLs vivas y 64 MiB en total. **No se revocan solas** — un bucle que
crea una por cuadro se queda sin cuota, y el error llega en el
`createObjectURL`, lejos de donde está el problema real.

## `IO.Buffer` e `IO.pipe`

Utilidades de bytes en memoria; ningún archivo, ningún socket.

```js
const buf = new IO.Buffer(1024);          // o new IO.Buffer(bytesExistentes)
buf.write(algo);                          // devuelve cuántos bytes entraron
buf.seek(0);
const out = new Uint8Array(64);
const n = buf.read(out);                  // n bytes, o null si no queda nada
buf.bytes();                              // copia de lo escrito
buf.close();
```

`Buffer` **no crece**: `write` recorta a la capacidad y devuelve lo que escribió.
Ignorar ese número es la forma corriente de perder la cola de un mensaje.
Máximo 8 MiB.

```js
const { reader, writer } = IO.pipe({ capacity: 65536 });
await writer.write(bytes);   // resuelve cuando entró (parcial, devuelve n)
const n = await reader.read(destino);  // null = fin
writer.close();              // fin de datos
reader.close();              // cancela ambos lados
```

Un anillo con una lectura y una escritura pendiente **como máximo**: una segunda
en vuelo rechaza con `Only one pending read is allowed`. `writer.close()` marca
EOF; `reader.close()` cancela y rechaza lo que hubiera pendiente.

---

Volver al [índice de guías](index.md).
