# Programas de movimiento

Un programa es un anclaje calculado: combina las poses de otros anclajes con
una secuencia corta de instrucciones que corre en el motor cada frame. Antes,
leé [anclajes.md](anclajes.md). Esta página describe la versión 1.


`dimension.anchors.create(key, {type: "program", version: 1, inputs, constants, code})`
compila un programa de poses y publica su salida como un anchor. La promesa
confirma la validación del host. Una declaración inválida se rechaza completa,
con diagnóstico de línea, y conserva una publicación anterior válida. El handle
usa el mismo `dispose()` y protección de versiones que los anchors normales.
Requiere `create_anchor`; acceder a grips requiere además `read_pose_stream`.

```js
const handle = await dimension.anchors.create("herramienta", {
  type: "program", version: 1,
  inputs: { principal: "right-grip", secundario: "left-grip" },
  constants: {
    offset: [0, 0.05, 0], up: [0, 1, 0], forward: [0, 0, -1]
  },
  code: `
    input pose principal
    input pose secundario
    const vec3 offset
    const vec3 up
    const vec3 forward
    temp vec3 target
    temp vec3 direction
    output pose result = principal
    transformPoint target, secundario, offset
    subtract direction, target, principal.position
    aimRotation result.rotation, direction, up, forward, up
  `
});
objeto.setAttribute("anchor", "herramienta");
```

La salida es una pose mundial. El transform local del consumidor sigue siendo
un offset, por lo que puede aplicar efectos locales o tener hijos normales.
El programa lee poses de anchors públicos (no aliases privados `__...`). Los
inputs pueden ser fuentes nativas, DOM o programas. El scheduler resuelve las
dependencias en el frame actual antes de colocar consumidores; ciclos quedan
indisponibles. Las referencias se actualizan al cambiar el registro y se leen
por índice/ID mientras permanece estable. Cerrar el espacio, revocar su permiso
o retirar la publicación libera también su entidad virtual.

## Declaraciones

Una declaración por línea, antes de cualquier instrucción. `//` inicia un
comentario. Nombres ASCII con letras, números y `_`, sin número inicial.

- `input pose nombre`: binding existente en `inputs`, sólo lectura.
- `const vec3|quat|float|bool nombre`: valor existente en `constants`, sólo lectura.
- `temp pose|vec3|quat|float|bool nombre`: temporal; debe inicializarse antes de leerlo.
- `output pose nombre = input`: exactamente una salida, inicializada con la pose
  completa de un input. Campos de pose: `.position`, `.rotation`, `.scale`.

Vectores son arrays de 3 números, quaterniones `[x,y,z,w]`, escalares números y
booleanos booleanos. Todos deben ser finitos; quaterniones no pueden ser cero y
se normalizan. Las escrituras parciales requieren una pose ya inicializada.
Los operandos son registros o campos; los literales se declaran en `constants`.
Límites: 16 inputs, 32 constantes, 64 registros, 128 instrucciones contando la
inicialización de salida, 16 KiB de código y 32 KiB de definición de entrada.
No hay bucles, saltos, memoria arbitraria ni ejecución JS.

## Instrucciones

El primer operando es el destino. Los operandos se separan con comas.

| Instrucción | Operandos después del destino | Resultado |
|---|---|---|
| `copy` | valor del mismo tipo | copia |
| `transformPoint` | pose, vec3 local | vec3 mundial, incluye escala |
| `add`, `subtract` | vec3, vec3 o float, float | vec3 o float |
| `rotate` | quat, vec3 | vec3 |
| `aimRotation` | dirección mundial, up mundial, dirección local, up local | quat |
| `multiplyQuat` | quat, quat | quat compuesto, orden conservado |
| `inverseQuat` | quat | quat inverso |
| `normalize` | vec3 | vec3; cero permanece cero |
| `length` | vec3 | float |
| `greater` | float, float | bool |
| `clamp` | float, mínimo float, máximo float | float; límites invertidos invalidan el frame |
| `multiply` | float, float o vec3, float | float o vec3 |
| `select` | bool, valor si true, valor si false | mismo tipo de ambos valores |

`aimRotation` construye dos marcos con frente `-Z` y arriba `+Y`, y compone el
marco mundial por el inverso del local. Reproduce los ejes alternativos de
`qMirando` para up paralelo: +Y mundial y después -Z. Una dirección cero produce
un marco identidad y puede descartarse mediante `select`, como hace tiro con
su umbral de 5 cm. Las instrucciones son secuenciales; select no es evaluación
perezosa. Resultados no finitos dejan el anchor indisponible en ese frame.

## Tracking y estado

Cada ejecución reinicia registros con inputs y constantes; no hay temporales
persistentes. Por defecto, un input ausente deja el programa indisponible.
`inputs.secundario = {anchor: "left-grip", holdLast: true}` conserva su última
pose mundial válida mientras la publicación sigue existiendo y sus permisos
siguen vigentes. El cache se invalida al retirar/reemplazar la fuente o revocar
acceso. Actualizar constantes del mismo programa conserva poses retenidas si
los inputs y sus políticas coinciden. Nunca retiene una fuente que aún no se
observó. Este modo preserva el comportamiento de tiro al perder tracking.

## Costo

La VM usa registros de tamaño fijo y no reserva memoria al ejecutar. El
programa de tiro (11 instrucciones) cuesta unos 0,4 µs por evaluación, sin
contar la resolución de fuentes ni el render.

## Ejemplos en los demos

**Tiro.** Las armas de una mano usan un programa de seguimiento con su montaje.
Las de agarre secundario usan un programa de 11 instrucciones que orienta el
arma de la mano principal hacia la secundaria, con los ajustes de cada modelo
como constantes; soltar el segundo agarre vuelve al programa de una mano. El
retroceso es un offset local del modelo hijo, así que variarlo no recompila
nada. La detección de agarres y los disparos siguen en JS.

**Arco.** Al tomarlo se publica un programa de agarre rígido con la rotación
relativa capturada en ese momento. Con flecha cargada, un segundo programa
apunta desde la mano que tensa hacia la del arco, conservando el arriba de la
muñeca. Disparar retira el anclaje de la flecha antes de que empiece su vuelo
en JS. Soltar el arco retira los programas.

**Sables.** Cada sable es un programa de seguimiento con la inclinación de la
hoja calibrada; sólo se recompila al cambiar la calibración. Las hojas se
escuchan con `listen` y `delivery: 'history'` para recorrer cada tramo del
corte. Cambiar de programa se marca como discontinuidad, así que no aparece un
barrido falso entre las dos orientaciones.
