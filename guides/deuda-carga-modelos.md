# Deuda técnica: carga masiva de modelos

Estado: pendiente. No se activa todavía carga escalonada ni un presupuesto por cuadro.

## Problema

Cargar cientos de megabytes de modelos a la vez puede bloquear cuadros en cuatro
etapas distintas: descarga y copias en memoria, decodificación, instanciación de
la escena y preparación de recursos en GPU. Ejecutar la descarga de forma
asíncrona no distribuye automáticamente las etapas posteriores.

El banco de `server_jugadores` muestra además un caso de duplicación importante.
`mika.glb` ocupa aproximadamente 12 MB, pero sus PNG de 2500 x 2500 y 4000 x
4000 ocupan alrededor de 84,9 MiB al expandirse a RGBA8, sin mipmaps. Las 64
variantes cambian únicamente `baseColorFactor`; mantener imágenes independientes
para todas puede representar aproximadamente 5,3 GiB. Es una estimación de
almacenamiento, no una medición de VRAM.

## Trabajo pendiente

- [ ] Medir por separado descarga, memoria residente, decodificación,
  instanciación y preparación GPU, registrando p95, p99 y peor cuadro.
- [ ] Limitar concurrencia y memoria en tránsito. `prepare_model_asset` acumula
  la respuesta completa y realiza otra copia antes de escribirla a disco.
- [ ] Descargar y calcular hashes con buffers acotados, conservando revisiones
  inmutables y usando validación HTTP para evitar descargas innecesarias.
- [ ] Distribuir la admisión de escenas. Limitar la cantidad de escenas no limita
  el costo de una escena individual enorme, que necesita tratamiento propio.
- [ ] Presupuestar la preparación GPU con prioridad para UI y contenido visible.
  Bevy 0.14.2 incluye `RenderAssetBytesPerFrame`, pero Luna no lo configura y su
  límite es blando: un asset individual puede excederlo.
- [ ] Conservar el asset anterior hasta que su reemplazo esté preparado. En esta
  versión de Bevy, activar solamente el presupuesto puede hacer desaparecer
  temporalmente texturas o mallas actualizadas.
- [ ] Compartir imágenes y geometría idénticas entre GLB distintos, respetando
  formato, espacio de color, sampler y uso. Un hash del archivo completo no
  descubre los subrecursos compartidos de las variantes.
- [ ] Añadir políticas de residencia, resolución y LOD para recursos grandes.
- [ ] Distinguir una escena instanciada de sus recursos listos en GPU sin cambiar
  silenciosamente el significado existente de `model-state="ready"`.

## Criterios de aceptación

Comparar caché frío y caliente, archivos compartidos y distintos, y texturas
grandes con UI activa tanto en escritorio como en VR. Registrar memoria máxima y
tiempo hasta la aparición visual. Al agotar el presupuesto, la UI y los recursos
anteriores deben permanecer visibles.

La estabilidad puede requerir aparición progresiva o menor detalle. No se debe
prometer ausencia de tirones para cualquier archivo y hardware.

## Código relacionado

- `crates/luna/src/io.rs`: `prepare_model_asset` y `request_model_prepare`.
- `crates/luna/src/models.rs`: `poll_model_instances`.
- Bevy 0.14.2: `render_asset.rs` y `scene_spawner.rs`.
- `server_jugadores/README.md` y `tools/variantes_glb.py`.
