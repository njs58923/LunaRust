# Poses locales y setJointBatch

La fuente se elige independientemente del estado del reproductor:

```xml
<model id="avatar" src="./avatar.glb" pose-source="script" />
```

- `pose-source="clip"` (predeterminado): controles de [clips](animaciones.md).
- `pose-source="script"`: poses locales explícitas mediante `setJointBatch`.
- Otras fuentes todavía producen error. No hay socket nativo ni interpolación de red.

Al cambiar la fuente se restaura el TRS de referencia de la escena. Al volver a
`clip`, se reinicia el clip seleccionado, respetando `animation-time` si existe.
No hay mezcla automática. En modo script se desliga el grafo del reproductor:
ni un clip pausado ni sus transiciones pueden sobrescribir la pose. Las poses
persisten hasta otra escritura, cambio de fuente, sustitución de `src` o desmontaje.
No requieren un rAF si no hay datos nuevos.

## Catálogo y disponibilidad

Esperar `pose-status="script"` y leer `animation-joints` con `JSON.parse`.
El estado `model-state=ready` por sí solo no garantiza disponer del catálogo.
El catálogo se construye al activar `script`; los modelos que sólo usan clips no
pagan su costo. Se conserva al volver a `clip` mientras no cambie el recurso.
Es un objeto versión 1:

- `binding`: token opaco obligatorio para escribir; exclusivo de esa instancia.
  Cambia al sustituir el recurso. No se debe reutilizar entre modelos.
- `schema`: huella del catálogo estructural, nombres y TRS de referencia; no es
  una certificación de compatibilidad humanoide ni un hash del archivo completo.
- `nodes`: `{index, name, parent, path, translation, rotation, scale}`.
  `parent` es otro índice o `null`; `path` identifica el recorrido de hijos desde
  la raíz de escena. Nombres repetidos son válidos. Incluye nodos intermedios,
  raíz de escena y nodos de geometría, además de huesos.
- `skins`: `{node, joints}` por entidad skinned; `joints` conserva el orden del
  skin importado y remapea sus entradas a índices de `nodes`. Varias primitivas
  pueden referir al mismo esqueleto.

**`index` no es el índice original del nodo glTF ni la posición dentro de
`skin.joints`.** Usar el catálogo entregado, sin asumir nombres únicos ni orden
entre archivos. El binding corresponde a la escena cargada (actualmente Scene0).
La implementación compara su jerarquía con la escena fuente de Bevy para recuperar
el TRS original, incluso si un clip ya comenzó a evaluarse.

El catálogo se vacía al cambiar `src`. `pose-error` informa errores de binding,
fuente o lotes rechazados por el host. Un lote antiguo se rechaza entero.

## Escribir una pose

```js
const root = hiperspace.dimention;
const model = root.getElementById('avatar');
// Ejecutar cuando el snapshot exponga pose-status="script".
const rig = JSON.parse(model.getAttribute('animation-joints'));
const joint = rig.nodes.find(n => n.name === 'Head'); // ejemplo: nombre del archivo
if (joint) {
  root.setJointBatch(model.nodeId, new Float32Array([
    joint.index, 0, 0, 0, 1
  ]), { binding: rig.binding });
}
```

Firma: `root.setJointBatch(nodeId, data, {binding, format?})`.

| Formato | Datos planos por entrada |
| --- | --- |
| `rotation` (predeterminado) | `index, qx, qy, qz, qw` |
| `trs` | `index, tx, ty, tz, qx, qy, qz, qw, sx, sy, sz` |

Son transformaciones **locales absolutas** relativas al padre real, no deltas ni
matrices inversas de bind. Rotaciones en cuaterniones XYZW; posiciones en unidades
locales del modelo. La escala del `<model>` sigue afectando al conjunto.
`rotation` conserva traslación y escala **de referencia**, no las del último lote TRS.
Los nodos omitidos conservan su pose. Para un índice repetido gana la última
entrada; varios lotes pendientes se combinan por nodo e índice.

Se aceptan `Float32Array` (incluidas subviews) y arrays normales. El buffer se copia
al enviar y puede reutilizarse inmediatamente. Se validan números finitos, índices
enteros y cuaterniones no nulos; se normalizan estos últimos. Máximo 4096 entradas
por llamada, 4096 nodos por binding y 65536 entradas pendientes por isolate.
Los errores de formato/cuota lanzan excepción síncrona. Los errores que requieren
conocer el modelo llegan asincrónicamente a `pose-error`.

No se añade un permiso para controlar el modelo propio: el host resuelve el
`nodeId` en la tabla local de handles del espacio, igual que los transforms.
Un número de entidad Bevy o un handle de otro isolate no concede acceso.

## Stream y estándares de avatares

El decoder y el aplicador están separados del isolate y de la red. Un futuro
`pose-source` nativo podrá alimentar el mismo aplicador con buffers acotados,
interpolación y permisos de conexión, sin generar comandos DOM por hueso.

La base acepta esqueletos generales. VRM 1.0 y VRM Animation pueden ir por encima:
son extensiones de glTF que asignan roles humanoides a nodos. Un adaptador debe
importar esa asignación, resolverla contra el binding y hacer retargeting/rest-pose
cuando corresponda. Este cambio **no implementa** carga VRM completa, expresiones,
spring bones, restricciones ni importación del mapeo original de nodos glTF.
No confundir escribir TRS de un esqueleto con reproducir cualquier avatar VRM.

Referencias oficiales:
- [VRM humanoid](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/humanoid.md).
- [VRM Animation](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm_animation-1.0/README.md).
