# Clips GLB — etapa 2

Los modelos GLB/glTF pueden reproducir sus clips importados mediante Bevy. No
necesitan `requestAnimationFrame` ni actualizaciones de transformaciones desde JS.
El `<model>` sigue siendo la raíz estable: moverlo mueve la escena animada entera.
Los controles se aplican también si se escriben antes de terminar la carga.

```xml
<model id="butterfly" src="./butterfly.glb"
       animation-clip="Fly" animation-loop="true" animation-speed="1" />
```

Los nombres dependen del archivo. `animation-clip="0"` selecciona el primer clip.
Si un nombre es numérico, tiene prioridad sobre el índice. Sin clip seleccionado
el modelo se carga sin reproducción automática.

## Controles HSML y JavaScript

| Atributo | Significado | Predeterminado |
| --- | --- | --- |
| `animation-clip` | Nombre exacto o índice desde cero; vacío desactiva el clip | vacío |
| `animation-state` | `playing`, `paused`, `stopped` | `playing` |
| `animation-loop` | Repetir indefinidamente (`true`) o una vez (`false`) | `true` |
| `animation-speed` | Multiplicador finito y no negativo; cero congela el tiempo | `1` |
| `animation-time` | Salto en segundos, limitado a la duración del clip | sin salto |
| `animation-restart` | Cambiar este valor reinicia el clip seleccionado | vacío |

```js
const butterfly = document.getElementById('butterfly');
butterfly.setAttribute('animation-state', 'paused');
butterfly.setAttribute('animation-time', '0.5');
// Más tarde:
butterfly.setAttribute('animation-state', 'playing');
butterfly.setAttribute('animation-speed', '1.5');
// Reiniciar incluso después de terminar una reproducción única:
let restart = 0;
function replay() {
  butterfly.setAttribute('animation-restart', String(++restart));
}
```

Pausar conserva el tiempo y reanudar continúa desde ahí. Parar lleva al primer
fotograma y lo mantiene; volver a `playing` inicia de nuevo. Un cambio de clip
también reinicia. Vaciar `animation-clip` desactiva la evaluación y conserva la
última pose; no restaura automáticamente la pose de reposo.

`animation-time` es una posición solicitada, no un reloj de lectura. Solo se
aplica cuando cambia, al seleccionar otro clip, al volver desde `stopped`, o al
cambiar `animation-restart`. Si se conserva, esos reinicios parten de ese tiempo.
Para reiniciar desde cero, quitar el atributo o establecerlo en `0`. Repetir una
escritura idéntica no reinicia nada. Un clip finalizado requiere un reinicio o un
nuevo salto para volver a avanzar.

## Datos de salida

Los atributos de salida llegan por el mecanismo existente de snapshots del DOM;
una escritura y su resultado no necesariamente están disponibles en el mismo tick.

- `animation-clips`: JSON con `{index, names, duration}` por clip. Duración en segundos.
- `animation-status`: `loading`, `idle`, `playing`, `paused`, `stopped`, `finished` o `error`.
- `animation-error`: diagnóstico de un control inválido o clip inexistente; vacío si no hay error.

Un control inválido conserva la reproducción válida anterior y publica el error.
Los fallos de carga siguen en `model-error`. `animation-state` expresa la orden;
`animation-status` informa el resultado. No hay eventos ni promesas nuevos.

```js
const clips = JSON.parse(butterfly.getAttribute('animation-clips') || '[]');
```

## Recursos, rendimiento y alcance

Cada revisión GLB comparte el grafo y sus metadatos entre instancias. Cada escena
conserva sus propios `AnimationPlayer`, tiempo y estado. La caché usa handles
débiles y no retiene los assets al cerrar los modelos. Cambiar `src` elimina los
reproductores anteriores y vuelve a enlazar los controles con la nueva generación.

La jerarquía se recorre una vez al enlazar la escena. Después se usan los IDs de
reproductores guardados, y las instancias sin cambios salen de la cola de control.
Solo las reproducciones finitas pendientes de terminar consultan finalización;
no se escriben atributos por fotograma. Se conserva la vía rápida de transformaciones.

Esta etapa reproduce un clip a la vez, con cambio inmediato, en `Scene0` (la escena
que ya cargaba Luna). No agrega mezclas/transiciones, selección de escenas, eventos
de animación ni resolución nueva de archivos externos de glTF; GLB autocontenido
es el camino cubierto por la prueba de integración. Los canales y rigs soportados
son los del cargador de Bevy instalado.

La regresión en `model_animation.rs` genera un GLB mínimo y usa el cargador real,
SceneSpawner y AnimationPlugin sin ventana: verifica movimiento, pausa, seek,
parada, velocidad, repetición, finalización, reinicio, aislamiento de instancias,
compartición de recursos y reemplazo de `src`.

---

Volver al [índice de guías](index.md).

## Fuente de poses

`pose-source="clip"` conserva este comportamiento. Para controlar nodos del
esqueleto directamente, consultar [Poses y setJointBatch](poses.md).
