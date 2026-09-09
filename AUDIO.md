# Audio y datos binarios

Primera implementación: reproducción 2D por isolate, clips WAV/OGG Vorbis/MP3 y PCM mono/estéreo por fragmentos. Usa el mezclador de Bevy. El decodificador de archivos trabaja en el worker JS; el callback de audio consume bloques de 256 muestras sin hacer red ni reservar memoria. Los controles se aplican con una latencia aproximada de un bloque más la del dispositivo.

## Permisos y vida útil

El documento pide `<space resources="audio,fetch_text">`. El shell ofrece `audio` por defecto a mundos y apps; el documento debe solicitarlo. Un include recibe sólo los recursos que su padre delega, por ejemplo `<include src="/player.hsml" resources="audio,fetch_text"/>`.

Los identificadores de audio y las URLs Blob pertenecen al isolate. Descargar/reiniciar el worker detiene sus voces. Revocar `audio` corta su salida. Ocultar una app no equivale a descargarla: la reproducción puede continuar mientras exista el isolate. No se solicita micrófono ni acceso al disco.

## Clips

```js
const music = new Audio('/audio/ambiente.ogg');
music.loop = true;
music.volume = 0.3;
music.addEventListener('ended', () => console.log('terminó'));
music.addEventListener('error', e => console.error(e.error));
await music.play(); // carga, decodifica y espera al mezclador
music.pause();
music.currentTime = 2;
await music.play();
music.stop(); // pausa y vuelve al inicio
music.dispose(); // libera la voz; el objeto ya no se reutiliza
```

`Audio` también acepta `Blob`, `ArrayBuffer`, vistas de buffers y URLs creadas con `URL.createObjectURL`. `load()` es asíncrono. `src` puede cambiar; invalida la carga anterior. Propiedades: `paused`, `ended`, `duration`, `currentTime`, `volume`, `muted`, `loop`. Eventos: `loadeddata`, `play`, `pause`, `ended`, `timeupdate`, `error`. Son APIs propias compatibles en lo básico con HTML Audio; no implementan todo HTMLMediaElement.

No hay reproducción automática por declarar una fuente. `play()` rechaza errores de carga, permisos, decodificación o ausencia de mezclador/dispositivo. Si la salida no llega a estar lista en cinco segundos, la carga falla y libera la voz.

## PCM y contrapresión

```js
const stream = new AudioStream({sampleRate:48000, channels:2, bufferSeconds:2});
await stream.ready;
await stream.play();
// Float32 intercalado L,R,L,R...; muestras finitas entre -1 y 1.
await stream.writeAll(samples);
stream.end(); // termina después de consumir lo que queda
// stream.dispose() cancela y libera, incluso con writeAll pendiente.
```

`write(samples)` devuelve los **frames** aceptados; puede devolver 0 si está lleno. `writeAll` espera espacio y acepta una sola operación pendiente. `bufferedFrames` y `writableFrames` permiten regular el productor. Una interrupción temporal produce silencio; sólo `end()` marca EOF. No se puede buscar ni repetir un stream. Un stream pausado y lleno necesita reanudarse o cancelarse para completar `writeAll`.

Límites por isolate: 16 voces, 64 MiB de PCM reservado/decodificado; entrada de archivo de hasta 8 MiB; mono/estéreo a 8–192 kHz. Buffer PCM entre 0,1 y 10 segundos. El productor debe enviar fragmentos acotados y respetar la contrapresión.

## Fetch, Blob y buffers

Los cuerpos HTTP conservan sus bytes. `Response` ofrece `arrayBuffer()`, `bytes()`, `blob()`, `text()`, `json()` y `clone()`, manteniendo consumo único. `fetch` acepta cuerpos de texto, URLSearchParams, Blob, ArrayBuffer y vistas; conserva el byteOffset/byteLength de cada vista. Los permisos, el mismo origen y las reglas de redirección siguen vigentes. Respuestas hasta 8 MiB, peticiones hasta 1 MiB.

`Blob` implementa constructor con partes, `size`, `type`, `slice`, `arrayBuffer`, `bytes`, `text`. Copia las entradas. Las URLs de objeto son locales al isolate y revocables; revocar impide nuevas resoluciones, pero no invalida una respuesta ya adquirida. Máximo 128 URLs / 64 MiB referenciados. Cada Blob admite hasta 8 MiB.

`TextEncoder` y `TextDecoder` soportan UTF-8, incluida escritura parcial por caracteres completos con `encodeInto`; el decoder no implementa todavía el modo incremental `stream:true`.

```js
const buffer = new IO.Buffer(4096); // capacidad fija
const n = IO.write(buffer, bytes); // escritura parcial si no cabe
buffer.seek(0);
const read = IO.read(buffer, destination); // número de bytes, null al final
buffer.close();

const {reader, writer} = IO.pipe({capacity:65536});
// En tareas distintas: writes esperan espacio, reads esperan datos.
await IO.write(writer, chunk); // puede escribir parcialmente: usar el resultado
writer.close(); // EOF después de consumir la cola
await IO.read(reader, destination);
reader.close(); // cancela y rechaza operaciones pendientes
```

`IO` trabaja con memoria/canales. No expone archivos arbitrarios, sockets ni permisos implícitos al sistema operativo. Una pipe admite una lectura y una escritura pendientes a la vez; capacidad máxima 8 MiB.

## Límite actual y evolución

La cola PCM es una base de streaming real, pero `fetch` aún descarga la respuesta completa. Faltan lectura incremental de HTTP con cancelación, WebSocket binario y decodificadores incrementales para radio/formatos comprimidos continuos. No hay HLS, MediaSource, Web Audio graph, captura de micrófono ni sonido espacial/HRTF. Una futura fuente de red podrá alimentar la misma cola PCM sin cambiar sus controles de reproducción.

Demo: `luna://audio_demo`, con un WAV generado en memoria y ocho segundos de PCM estéreo producido por fragmentos; sólo suena al pulsar un botón. Requiere recompilar Luna. Las pruebas cubren bytes no UTF-8, Blob/IO, WAV, controles, cuotas, EOF, contrapresión, aislamiento y limpieza. La salida física debe comprobarse en el equipo con dispositivo de audio.
