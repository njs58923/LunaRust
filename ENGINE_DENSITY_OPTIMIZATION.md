# Engine: costo de escenas con muchos modelos

## Cambios

- Se eliminó la cámara 2D auxiliar del arranque. En bevy_egui 0.28, el pase
  de egui se registra por ventana después de CameraDriver y no requiere esa
  cámara. En Bevy 0.14, `check_visibility` recorre las mallas para cada cámara
  activa, aunque su grafo 2D no dibuje las primitivas PBR de los GLB. La cámara
  sobrante agregaba culling y podía marcar mallas como visibles para extracción
  que no estaban en el frustum de la cámara 3D.
- La UI sincroniza URL/título de tabs desde el DomMirror ya disponible. Una
  animación que cambia el mirror no obliga a recorrer otra vez el mundo Specs
  ni a crear un mapa nuevo de snapshots de pestañas. Se leen los espacios y los
  hijos directos del root/tab, y solo se reemplazan los strings que cambian.
- El capturador de MCP hace su sondeo GPU adicional solo mientras tiene buffers
  pendientes de mapear. El contador se mantiene hasta ejecutar el callback,
  incluso si ya se consumió la petición. La cámara spectator no marca `Camera`
  como modificado cuando su estado activo/inactivo no cambia.
- Se agregó el span `ui_system` a `LUNA_PROFILE_CSV`. La UI conserva su frecuencia
  e interacción: no se saltea navegación, permisos ni entrada de egui.

La escena, los modelos, la densidad, los materiales y los clips no se modificaron.

## Verificación reproducible

```powershell
cargo test -p luna --lib
cargo test -p luna --lib density_tests::benchmark_dense_visibility -- --ignored --nocapture
cargo check -p luna --bins
```

El benchmark utiliza el `check_visibility` real de Bevy, con 30.085 y 76.779
primitivas sintéticas, cámara 3D fija y frustum ortográfico auxiliar de tamaño
2000×1041. Alterna una y dos vistas activas durante 160 pares, después de
20 frames de calentamiento, y publica medianas. No carga el campamento ni usa
GPU: cuantifica el trabajo CPU de la vista redundante; no representa FPS finales.

Resultado local (perfil de tests optimizado, 2026-09-07):

| Primitivas | Dos vistas activas | Una vista activa | Reducción de esta fase |
| --- | ---: | ---: | ---: |
| 30.085 | 0,8379 ms | 0,4017 ms | 52,1% |
| 76.779 | 2,3708 ms | 1,1686 ms | 50,7% |

La cámara 3D veía 16.552 y 63.246 primitivas respectivamente. Son medianas del
benchmark sintético; no se extrapolan directamente al campamento o a VR.

Las regresiones comprueban igualdad del conjunto visible de la cámara 3D,
actualización de URL/título con un mirror de 10.000 modelos y ausencia de
notificaciones de cambio en la cámara spectator inactiva.

## Límites de la conclusión

Todavía hay que repetir el campamento con la misma cámara, resolución, foco y
modo de presentación para cuantificar la ganancia de frame completo, y medir VR
por separado. No se debe atribuir todo el piso de 2,5 ms a egui sin medir su span.
El devtool ya tenía condiciones de visibilidad y sus cuerpos colapsados no se
ejecutan automáticamente todos los frames. Tampoco una curva sublineal demuestra
por sí sola cuánto corresponde al culling frente al piso fijo o al batching.
