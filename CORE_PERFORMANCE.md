# Core: elementos dinámicos

## Corrección posterior: animación y clics en scale_demo

El watchdog del worker tenía un límite fatal de 50 ms por tick. Superarlo
destruía el isolate, incluidos sus callbacks RAF y listeners. No era un límite
de cantidad de objetos: una pausa por carga o un callback finito lento podían
dispararlo. Una regresión con un callback de 80 ms reprodujo el error
`JS tick exceeded its 50 ms execution limit` antes del cambio.

El límite de emergencia del tick ahora es de 2 segundos, igual que el de eval.
El worker sigue siendo asíncrono y sólo admite un tick en vuelo; un frame lento
no debe confundirse con un script bloqueado. Los bucles infinitos siguen siendo
interrumpidos y cerrar el worker sigue sin bloquear al hilo principal.

También se conserva la solicitud de un nuevo tick si llega un evento mientras
se procesa el anterior. Antes, la respuesta del tick viejo podía sobrescribir
esa solicitud con `needs_continuous_ticks = false`, dejando el clic sin un
pump posterior. La solicitud se consume al enviar el tick, no al recibir su
respuesta.

En `scale_demo`, las referencias a controles estáticos se guardan para evitar
búsquedas repetidas sobre miles de hijos, y los contadores/estado se actualizan
una vez por lote de creación, en lugar de una vez por objeto.

Validación posterior: **128 tests aprobados, 4 ignorados**. Se ejecuta el script
real de `luna://scale_demo` en V8 con un snapshot de 5.000 elementos ya resueltos,
verificando movimiento entre frames, recoloración, pausa y borrado por eventos
`toque`. También hay regresiones del callback finito de 80 ms y del clic recibido
durante un tick anterior. La prueba aislada de 5.000 elementos ya pasaba con el
límite anterior en este equipo; la reproducción determinista del fallo es la
del callback lento. No se midió aquí el umbral de carga en la aplicación gráfica
ni el raycast físico del ratón/visor.

## Alcance

Revisión del flujo JavaScript → colas de mutaciones → DOM en SPECS → entidades
Bevy, y del espejo usado para sincronizar el DOM con JavaScript. El core ya
comparte mallas/materiales y dispone de una vía rápida para transformaciones.
Los cambios se concentran en CPU, sin modificar las APIs de scripting.

## Cambios

- **Borrado por lotes del espejo JS:** antes cada ID eliminado recorría todos
  los nodos y sus listas de hijos. Ahora se eliminan los IDs y se limpian las
  referencias en una sola pasada. Ese tramo pasa de O(R × (N + E)) a
  O(R + N + E), donde R son eliminaciones, N nodos y E referencias a hijos.
  La reconstrucción posterior de subárboles por espacio sigue existiendo.
- **Transformaciones redundantes:** posición, rotación y escala se comparan
  con SPECS antes de marcar trabajo pendiente. Se conserva el orden de las
  escrituras; la última prevalece. En la vía rápida hacia Bevy también se
  comprueba el resultado final antes de escribir `Transform`, evitando
  `Changed<Transform>` cuando un batch vuelve al valor inicial.
- **Animación y creación simultáneas:** la vía rápida se procesa antes del
  ordenamiento topológico. Crear un padre/hijo ya no obliga a ordenar también
  todos los objetos que sólo se están moviendo. Se consulta el mapa del DOM
  directamente, sin copiar todos sus IDs a un `HashSet`. Las profundidades se
  calculan una vez por nodo genérico con `sort_by_cached_key`.
- **Menos memoria temporal:** se compacta el vector de IDs pendientes con
  `retain`, eliminando el segundo vector de tamaño N que separaba la vía genérica.
  La deduplicación inicial y su buffer temporal todavía existen.

## Validación y mediciones

Ejecutado en Windows, con el perfil de tests del repositorio: código local
con optimización de desarrollo y dependencias optimizadas. No es una medición
de release, de FPS, ni de una sesión VR.

```powershell
cargo test -p luna --lib
cargo test -p luna --lib benchmark_ -- --ignored --nocapture --test-threads=1
```

Resultado: **125 pruebas aprobadas, 4 ignoradas** en la ejecución normal.
Los dos benchmarks manuales también pasan al invocarlos explícitamente.
Las pruebas nuevas cubren 10.000 objetos animados junto con jerarquías nuevas,
escrituras idénticas, batches que vuelven al valor inicial y limpieza de
referencias a nodos eliminados, incluidos IDs ausentes e idempotencia.

### Actualización del DOM

Cada frame encola posición, rotación y escala para todos los objetos; cambia
la posición y mantiene rotación/escala. Además crea un padre y un hijo.
Se toman 100 muestras después de 20 frames de calentamiento. Se mide
`App::update()`, excluyendo la preparación de colas y del DOM SPECS.

| Objetos animados | Mediana | P95 |
| --- | ---: | ---: |
| 1.000 | 0,398 ms | 0,597 ms |
| 10.000 | 4,213 ms | 7,757 ms |
| 50.000 | 22,261 ms | 34,730 ms |

El fixture usa `MinimalPlugins`, sin GPU, runtime JS ni propagación completa
de transformaciones de una aplicación Bevy real. Estos valores caracterizan
el tramo de sincronización y no el costo total de una escena. No se obtuvo
una medición anterior equivalente para este benchmark de animación.

### Borrado del espejo

10.000 hijos, 1.000 eliminaciones por batch, 12 muestras por algoritmo.
El benchmark conserva el algoritmo anterior como referencia sólo dentro del
test manual y verifica que ambos producen los mismos hijos y subárboles.
La preparación/clonación del fixture queda fuera del tiempo medido; se incluye
la reconstrucción de subárboles tanto antes como después.

| Algoritmo | Mediana |
| --- | ---: |
| Anterior: una pasada por ID | 45,047 ms |
| Actual: limpieza por lote | 1,855 ms |

La mejora observada es **24,3× en esta operación aislada**. No extrapolar este
factor al frame completo; los tiempos varían según equipo, perfil y carga.

## Costos restantes identificados

- `js_update_snapshots_system` todavía construye el conjunto completo de IDs
  adjuntos cuando necesita sincronizar. `refresh_dom_mirror_in_place` reconstruye
  los subárboles de espacios cuando cambia el espejo, incluso en cambios sólo
  de atributos. Conviene separar invalidación estructural y de propiedades
  antes de intentar eliminar ese trabajo.
- Los raycasts de `touch.rs` recorren los elementos interactivos. Una estructura
  espacial puede ayudar con muchas entidades, pero debe medirse también el costo
  de mantenerla cuando se mueven en cada frame.
- La deduplicación de `DirtyNodes` sigue ordenando IDs; la vía rápida sigue
  convirtiendo Euler a quaternion para cada transformación procesada.
- En esta medición, 50.000 elementos ya consumen más de 22 ms de CPU sólo en
  sincronización. Para objetivos exigentes de VR falta medir el ciclo completo:
  tick JS, snapshots, DOM, propagación de jerarquías, extracción/render y GPU,
  usando una escena representativa en release y el visor objetivo.
