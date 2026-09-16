# Audio

Dos clases globales, sin ningún tag HSML de por medio: **`Audio`** para un
archivo codificado, **`AudioStream`** para PCM que se genera desde el script.
El sonido no está posicionado en el espacio: sale por la mezcla del shell.

Pide `resources="audio"`.

## Salida de escritorio y visor

En escritorio, Luna usa la salida predeterminada del sistema. Con una sesión VR
activa, selecciona la salida de **Quest Link / Air Link** cuyo nombre contiene
`Oculus Virtual Audio Device`. Solo cambia la salida de Luna, no la configuración
global de Windows. Si no encuentra una salida única o no puede abrirla, intenta
la predeterminada y deja un aviso en el log.

La selección se realiza al arrancar y al entrar/salir de una sesión VR. No hay
sondeo periódico de dispositivos. Al cambiar, las voces existentes continúan
desde su estado actual, conservando volumen, pausa y bucle; puede haber una
breve interrupción y perderse audio ya almacenado en los buffers del dispositivo.
Esto también se aplica a `AudioStream`.

Para otro visor o un dispositivo con nombre distinto, se puede indicar el nombre
completo de la salida antes de iniciar Luna:

```powershell
$env:LUNA_VR_AUDIO_DEVICE = 'Auriculares (Oculus Virtual Audio Device)'
```

La comparación ignora mayúsculas pero exige el nombre completo. Esta opción
solo se aplica en VR; el log `[Audio] VR output: ...` confirma la salida elegida.
Si conectás una salida después de que falló la selección, salí de VR y volvé a
entrar para repetirla. Si no había ninguna salida disponible, una nueva petición
de audio también vuelve a intentarlo.

```xml
<space resources="audio,fetch_text">
  <script src="./sonido.js"/>
</space>
```

```js
const timbre = new Audio('./timbre.ogg');
timbre.volume = 0.4;
await timbre.play();          // carga, decodifica y arranca
```

---

## `Audio` — un clip

El constructor **no carga nada**. La descarga y el decodificado pasan en el
primer `play()` (o en un `load()` explícito), en el worker del isolate, nunca en
el hilo de render.

`src` acepta tres cosas:

| | |
|---|---|
| una URL | se baja con `fetch`, así que **hace falta también `fetch_text`** |
| un `Blob` | ver [binario.md](binario.md) |
| un `ArrayBuffer` o una vista | los bytes ya codificados |

Formatos compilados: **WAV, OGG/Vorbis y MP3**. No hay FLAC ni AAC. Mono o
estéreo, entre 8 y 192 kHz; cualquier otra cosa rechaza con un error del
decodificador.

Propiedades y métodos, deliberadamente parecidos a `HTMLAudioElement`:

- `play()` → promesa que resuelve cuando ya suena; `pause()`, `stop()`
  (pausa y vuelve a cero), `load()`, `dispose()`;
- `currentTime` (lee y escribe: escribir es un seek), `duration`, `paused`,
  `ended`, `volume` (0..1, fuera de rango tira `RangeError`), `muted`, `loop`;
- eventos `loadeddata`, `play`, `pause`, `ended`, `timeupdate`, `error`, por
  `addEventListener` o por `onended`/`onerror`.

**`dispose()` no es opcional.** Libera la voz y su cuota; sin eso el espacio se
queda sin voces a las dieciséis.

## `AudioStream` — PCM generado

```js
const voz = new AudioStream({ sampleRate: 24000, channels: 2, bufferSeconds: 0.5 });
await voz.ready;
await voz.play();
for (const trozo of trozos) await voz.writeAll(trozo);  // Float32Array intercalado
voz.end();
```

- `bufferSeconds` va de 0.1 a 10; el búfer es un anillo, y su tamaño es lo que
  banca de retraso el generador antes de que se escuche un hueco.
- `write(samples)` copia lo que **entra ahora** y devuelve cuántos frames
  aceptó: puede ser menos que lo que le diste, y puede ser cero. Es la
  contrapresión, no un error.
- `writeAll(samples)` hace ese bucle por vos, esperando 10 ms cuando el búfer
  está lleno. **Sólo puede haber un `writeAll` pendiente**; dos en paralelo
  tiran `Error`.
- `bufferedFrames` y `writableFrames` dejan ver el anillo.
- `end()` marca fin de datos. Recién entonces, cuando además se vacía el búfer,
  llega `ended`.

Comparte con `Audio` el resto: `play`, `pause`, `volume`, `muted`, eventos,
`dispose`. No tiene `loop` ni `currentTime` de escritura.

---

## Trampas

**Quedarse sin datos no es terminar.** Un `AudioStream` que se vacía emite
silencio y sigue esperando; `ended` sólo llega después de `end()`. Un generador
que se atrasa produce una pausa muda, no un error, y no hay evento que avise.

**El sonido muere con el espacio.** Las voces pertenecen al worker que las creó:
si el espacio se descarga o el runtime reinicia, se descartan y su entidad
desaparece. Tampoco sobreviven a que se revoque el permiso — ahí la voz se corta
y `error` queda en `Audio permission revoked`.

**Sin permiso no falla al crear.** `new Audio(...)` funciona igual; el error
`Permission denied: audio was not granted` aparece recién cuando el comando
llega al mixer, o sea en el `play()`. Lo mismo si no hay salida de audio
instalada: `play()` espera 5 s y rechaza con `Audio output unavailable (no
active mixer/device)`.

**`timeupdate` es un sondeo a 20 Hz**, y sólo corre mientras suena. Para
sincronizar visuales finos conviene leer `currentTime` en el
`requestAnimationFrame`, no esperar el evento.

**`duration` miente de dos maneras**: es `NaN` antes de cargar, e `Infinity` en
un `AudioStream`.

**Escribir `src` cancela la carga en curso** y libera la voz anterior. Un
`play()` que estaba en vuelo rechaza con `Audio load cancelled` — hay que
catchearlo, no es un fallo real.

## Límites

| | |
|---|---|
| Voces vivas por isolate | 16 |
| Audio decodificado por isolate | 64 MiB |
| Un clip codificado | 8 MiB |
| Canales | 1 o 2 |
| Frecuencia | 8 000 a 192 000 Hz |

La cuota se mide en muestras decodificadas: un MP3 chico puede ocupar mucho.
Un minuto estéreo a 48 kHz son 23 MiB.

## Lo que todavía no hay

Vale saber dónde termina esto, porque la cola PCM se parece lo suficiente a un
streaming como para hacer creer que ya lo es: **`fetch` baja la respuesta
entera**. No hay lectura incremental de HTTP con cancelación, ni WebSocket
binario, ni decodificadores incrementales para radio o formatos comprimidos
continuos. Tampoco hay HLS, MediaSource, grafo de Web Audio, captura de
micrófono ni sonido espacial/HRTF — el audio es 2D y sale por la mezcla del
shell.

Una fuente de red futura va a poder alimentar esta misma cola PCM sin cambiarle
los controles de reproducción, que es justamente por qué la cola está separada
del origen de los datos.

## Demo

`luna://audio_demo` — un WAV armado a mano en el propio script (sin bajar nada)
y ocho segundos de PCM estéreo por fragmentos.

---

Volver al [índice de guías](index.md).
