# Optimizaciones para la calle de objetos con meshes

Trabajo realizado sobre `server_expo/src/calle.ts`, variante `alto_mesh`, entre el
25 y el 27 de septiembre de 2026. La entrada de referencia es
`http://localhost:2053/calle.hsml?detalle=alto_mesh`.

## Qué hace realmente esta escena

La calle monta 25 fachadas como includes independientes. Cada fachada recibe una
receta en props y ejecuta la concatenación de `_base.js`, `_vr.js`, `_obra.js` y
`pared.js` (188.064 bytes en el fixture medido). Cada isolate publica una mesh y
conserva handlers para mensajes y cambios de propiedades.

Aunque `tocable` sea falso, `_vr.js` instala un intervalo de 150 ms para el estado
de las manos. Esos timers no reconstruyen geometría. No se puede destruir el
isolate después del primer dibujo sin perder los handlers; se puede dormir hasta
el próximo evento/deadline y hacer muy barato un callback sin cambios.

El fixture real genera 25 uploads, 294.280 vértices, 1.114.944 índices y 16.230.976
bytes de buffers preparados. Comprueba que los timers no vuelvan a dibujar, que
`informar` siga respondiendo y que cambiar el material reemplace sólo una mesh.
Este fixture termina en el límite con el renderer: no mide frames ni GPU.

## Cambios del engine

- Los workers esperan comandos sin polling periódico. Un scheduler de watchdogs
  compartido reemplaza un hilo auxiliar por isolate. Los timers guardan deadlines
  locales, sin un hilo de sistema por timeout.
- El host despierta sólo por eventos, deadlines vencidos, RAF o trabajo nativo
  pendiente. El bridge de pose publica sólo a subscriptores autorizados y cuando
  cambia el valor efectivo; mantiene reintentos para revocaciones y colas llenas.
- La compilación del bootstrap se comparte entre isolates. Los scripts
  persistentes usan una caché de bytecode de V8 con verificación exacta de fuente
  y nombre, hasta 128 entradas y 8 MiB residentes. Cada isolate ejecuta su propio
  código y conserva globals, callbacks y permisos independientes. Las entradas
  prestadas a ejecuciones concurrentes pueden sobrevivir brevemente a su desalojo.
- Las meshes generadas se montan directamente, sin fabricar una Scene ni pasar
  por SceneSpawner. Los handles iguales comparten geometría; una modificación
  separa el recurso si estaba compartido. Los cambios exclusivos conservan el ID.
- Normales faltantes, fingerprint y límites se calculan en el worker. Los índices
  usan 16 bits cuando caben. Los bounds viven con el recurso y se actualizan al
  reemplazarlo. Bevy extrae los buffers al mundo de render sin conservar otra
  copia permanente en Assets del mundo principal.
- Los bindings de componentes conservan sus dependencias para no volver a parsear
  props/URLs por cambios ajenos. El parser XML evita duplicar atributos y sólo
  construye un buffer normalizado cuando lo necesita. Audio no recorre contextos
  cuando no hay voces activas.
- Los uploads de todos los isolates de un frame se aplican en un lote y visitan
  los modelos una sola vez. Se conserva el orden de scopes y el estado final de
  mesh/bounds incluso al eliminar y recrear un recurso con la misma URL.
- Un callback sin efectos conserva sus deadlines, RAF, ACKs y cambios de scope,
  pero omite el procesamiento de mutaciones del host. Los candidatos para pose
  reutilizan query/buffer y dejan de clonar modelos y handles al montar.
- `component.props` y el pump de mensajes sólo traen un snapshot cuando cambia
  generación o revisión. La comparación ocurre bajo el lock del canal antes de
  clonar/serializar JSON. Se conservan deep freeze, identidad de props, observers
  incluso tras un getter anticipado, desconexión y reemplazo de canal.

Se mantienen la cuota de meshes por isolate, la separación de propietarios y el
descarte de recursos al cerrar o reemplazar un worker.

## Mediciones reproducibles

Perfil de tests del workspace en Windows (código local con optimización 1,
dependencias con optimización 3). Son medidas de CPU acotadas, no una comparación
de FPS ni de memoria total del proceso. Las referencias sintéticas están dentro
de cada benchmark; los valores dependen del equipo y de otras cargas.

| Operación | Referencia | Optimizada | Alcance |
| --- | ---: | ---: | --- |
| Crear Engine, mediana | 10,661 ms | 7,828 ms | 48 muestras alternadas; caché bootstrap |
| Evaluar `_obra.js`, mediana | 3,649 ms | 1,089 ms | 112.739 bytes; caché caliente; 24 pares |
| Parsear 10.000 modelos, mediana | 51,403 ms | 36,895 ms | XML sintético; P95 ruidoso |
| Binding de props de 13.902 bytes | 0,1141 ms | 0,0017 ms | Invalidación ajena; mediana |
| Bridge de pose, 5.000 workers estáticos | 1,2165 ms | 0,0108 ms | Canales simulados; no V8/GPU |
| Subir 48 meshes distintas sin normales, hilo principal | 40,347 ms | 5,142 ms | 32.768 vértices / 147.456 índices por mesh |
| Uploads de 25 isolates ante 5.000 modelos | 7,6406 ms | 0,4326 ms | Mediana de 5 pares; individual frente a lote |
| Uploads de 1.000 isolates ante 5.000 modelos | 269,9918 ms | 2,3475 ms | Mediana de 5 pares; buffers ya preparados |
| Recoger candidatos de pose al montar 10.000 modelos | 4,0243 ms | 0,1328 ms | Query/IDs frente a query/clones; mediana |
| Recoger candidatos de pose sin cambios, 10.000 modelos | 0,0196 ms | 0,0176 ms | Conserva el scan de Changed para observar mutaciones |
| 25 lecturas de props sin cambios, 13.598 bytes | 10,3051 ms | 0,0051 ms | 9 pares de 2.500 lecturas en V8; mediana normalizada |

La última fila desplaza trabajo: preparar esas 48 meshes en workers costó 62,597
ms sumados en serie. El total de CPU no bajó (67,739 ms frente a 40,347 ms); baja
la parte que bloquea el hilo principal y se agregan fingerprint y bounds. La
deduplicación sí conserva una sola geometría para 48 recursos iguales, pero no
beneficia por igual a cargas con datos ya distintos o meshes animadas. El caso
con normales provistas e iguales tuvo 10,437 ms de host frente a 9,203 ms de
referencia: no se presenta como una mejora universal de tiempo de subida.

La primera ejecución satisfactoria del fixture real tardó 1.097,852 ms en montar
secuencialmente los 25 workers y sus fachadas. En una ventana posterior de 350 ms
se atendieron 70 callbacks de timer en 9,221 ms acumulados, sin nuevos uploads.
Son valores absolutos del fixture, no un A/B de la calle completa.

## Heap de los builders que quedan inactivos

`benchmark_real_street_retained_heap` mide cada fachada en su propio thread y
suma sus estadísticas, después de resolver callbacks de creación y atender
350 ms de timers. Una notificación de baja memoria a V8 redujo el heap usado
sumado de 170.412.376 a 43.164.728 bytes; el heap físico reportado por V8 pasó de
360.181.760 a 88.485.888 bytes y la memoria externa de 15.470.435 a 2.766.682 bytes.
No son medidas de RSS del proceso ni incluyen los buffers de Rust/GPU.

La colección tuvo una mediana de 3,196 ms y un máximo de 8,679 ms por isolate.
Los timers, el listener `informar` y el redraw por props siguieron funcionando.
El máximo de un redraw posterior fue 76,418 ms: esta medición no demuestra que
forzar GC reduzca la latencia de la siguiente reconstrucción.

La política del worker abre una oportunidad sólo tras un upload de mesh, espera
500 ms sin efectos/comandos de actividad y exige al menos 3 MiB de heap usado.
Se cancela si hay RAF, se pospone ante comandos, mensajes o timers inminentes,
y vence a los 5 segundos. Un mutex global sin espera permite una colección a la
vez. Cada intento termina la oportunidad y aplica 30 segundos de cooldown;
los timers ordinarios no la vuelven a armar. Al terminar se vuelve al receive
bloqueante, sin polling periódico. El resultado del tick se publica antes.

Esta API de V8 no permite interrumpir la pausa nativa de GC ni imponerle un
presupuesto estricto. La pausa ocurre en el worker con el watchdog JS desarmado;
puede retrasar un mensaje que llegue mientras se está colectando. Los objetos
vivos permanecen alcanzables, y no se ejecuta un event loop adicional de V8.

El segundo fixture ejecuta esa política completa, atendiendo los timers reales
durante los 500 ms de espera. Colectó 21 fachadas y omitió cuatro por tamaño:
el heap usado sumado pasó de 171.015.072 a 46.778.120 bytes (−72,6%), el físico
de 362.156.032 a 104.738.816 y el externo de 15.470.597 a 3.066.586.
La mediana de colección fue 3,322 ms y el máximo 7,867 ms. Las validaciones de
listeners/redraw pasaron; hubo un redraw posterior de 95,274 ms. Estos datos
corresponden a `benchmark_real_street_idle_heap_policy`, con la política de
producción, y no suponen que se colecten todos los isolates pequeños.

## Ejecutar las comprobaciones

Desde la raíz del repo, en PowerShell:

```powershell
cargo test -p js_runtime
cargo test -p virtual-dom
cargo test -p luna --lib
cargo check -p luna --bins

$env:LUNA_STREET_SOURCE_DIR = 'J:/archivos/proyectos/Unity/LunaExperiments/server_expo'
cargo test -p luna --lib benchmark_real_street_facades -- --ignored --nocapture
cargo test -p js_runtime --lib benchmark_street_script_compilation_cache -- --ignored --nocapture
cargo test -p luna --lib benchmark_static_mesh_uploads -- --ignored --nocapture
cargo test -p luna --lib benchmark_mesh_command_batches -- --ignored --nocapture
cargo test -p luna --lib benchmark_empty_timer_dispatch -- --ignored --nocapture
cargo test -p luna --lib benchmark_static_pose_binding_candidates -- --ignored --nocapture
cargo test -p js_runtime --test components_test benchmark_unchanged_component_context -- --ignored --nocapture
cargo test -p js_runtime --lib benchmark_real_street_retained_heap -- --ignored --nocapture
cargo test -p js_runtime --lib benchmark_real_street_idle_heap_policy -- --ignored --nocapture
```

El fixture de fachadas requiere Bun y el checkout externo de `server_expo`.
Los tests de integración del runtime escriben resultados en
`crates/js_runtime/tests/logs/`; esos archivos de mediciones no son cambios de
implementación.

La línea base de Luna ya fallaba en cuatro tests: `hex_color_invalid_length`,
`home_route_requests_only_navigate_self`,
`shell_slot_matches_menu_pose_on_first_and_repeated_reposition` y
`shell_lifecycle_uses_host_identity_and_stable_window_controls`. Se comprobó la
misma lista antes y después de las primeras optimizaciones. Se corrigió aparte
una carrera del fixture de readiness que iniciaba una carga de `test.scene`
inexistente y descartaba sus handles antes del resultado asíncrono.
La suite de Luna con la aplicación por lotes, query de poses reutilizada y
colección diferida pasó 254 tests y conservó esos cuatro fallos, con 16 tests
ignorados/manuales. La suite del runtime pasó 70 tests; adicionalmente pasaron
los seis tests determinísticos de la política de colección. `cargo check -p luna
--bins` también terminó correctamente.

Queda pendiente una medición visual de la calle completa en desktop/VR: tiempo
de frame, picos durante montaje, extracción GPU y memoria residente. El endpoint
MCP de Luna no estaba conectado durante esta validación.
