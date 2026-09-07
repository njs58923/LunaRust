# navigate_world — propuesta

> **Estado: propuesta. Nada de este contrato está implementado.**
> Lo que sí está verificado en Luna se marca como **medido**; lo que sale de leer
> el código, como **en el código**. El resto es diseño y se puede discutir entero.
>
> Sale de construir veintiocho escenas y un índice con veintiocho puertas contra
> el motor tal como está hoy (`server_noche`). Los tropiezos que se citan son
> reales, no hipotéticos.

## Por qué

Hoy hay dos destinos de navegación y ninguno sirve para una puerta.
`plan_navigation_for_space` (`crates/luna/src/js.rs`), **en el código**:

```rust
if caps.contains(NAVIGATE_SELF) {
    if let Some(inc) = find_nearest_ancestor_include(space) { return SelfNav { inc, url }; }
}
if caps.contains(NAVIGATE_GLOBAL) { return GlobalNav { url }; }
Blocked
```

Un `<include>` con `navigate_self` **se navega a sí mismo**. Verificado, y el
síntoma es memorable: al tocar el vano de una puerta, la escena de destino se
carga *adentro del marco* y el mundo de alrededor sigue estando. Se ve la cueva
dentro de la puerta.

`navigate_global` sí navegaría —el planificador lo contempla— pero no se concede.
`web/internal/root_api.js`, **en el código**:

```js
// kind === 'spatial'
grants = ['navigate_self', 'read_pose_stream', 'read_camera_pose', 'skybox',
          'fetch_text', 'fetch_http', 'spawn'];
```

Y los grants de un include se intersecan con los del padre, así que ningún hijo
puede tenerlo si el mundo no lo tiene.

**El dato que reordena todo el análisis: el mundo mismo es un include.**
`mountSpace` monta el documento raíz dentro de un `<include>`. Por eso
`navigate_self` en una escena se *siente* como navegar el mundo, y por eso una
puerta anidada —que tiene su propio include más cerca— no puede.

## Tres destinos, no dos

| destino | qué reemplaza | quién lo necesita |
|---|---|---|
| `self` | mi propio include | un componente que se recarga a sí mismo |
| **`world`** | **el include más externo de mi montaje** | **una puerta, un portal, un ascensor entre mundos** |
| `global` | el shell entero | el shell |

`navigate_world` **no es `navigate_global` con mejor nombre.** Es un tercer
destino, más angosto: reemplaza el documento espacial dejando el chrome, la
pestaña y Luna donde estaban. Que sea el más angosto es lo que lo hace concedible
por defecto — `navigate_global` no puede serlo nunca.

## El contrato propuesto

```js
hiperspace.world.navigate(url, {
  entry: 'llegada',                              // spawn del destino
  returnTo: { url: location.href, entry: 'desde_cueva' },
  replace: false,                                // sin entrada de historial
});

// Dentro del mismo mundo: no hay carga, sólo se mueve al visitante.
hiperspace.world.navigate({ entry: 'plaza_norte' });
```

Tres decisiones adentro de esa firma, y las tres tienen motivo:

**El destino es explícito en la llamada, no se infiere de la capability.** Es la
trampa que ya tiene el código: con la forma actual, un espacio que tenga
`navigate_self` **y** `navigate_world` nunca alcanza el segundo `if`, se navega a
sí mismo, y el autor no entiende por qué. La capability decide el **permiso**; la
llamada decide el **blanco**. `location.href` sigue siendo `self` y no cambia.

**La llegada viaja en la llamada.** Ver la sección siguiente.

**`navigate` sin `url` es movimiento dentro del mismo mundo.** Ver «La ciudad de
includes».

## Permiso: delegación, no un privilegio nuevo

Éste es el punto que más cambia respecto de tratarlo como "un `navigate` más
fuerte". Como el documento raíz **ya** navega el mundo entero con `navigate_self`,
`navigate_world` **no le da poder nuevo al origen que ya es dueño del mundo**.
Sólo deja que un documento *anidado* haga lo que su padre ya podía.

Entonces el modelo de confianza ya existe y es el de los includes:

```xml
<include resources="navigate_world" src="/puerta.hsml?..."/>
```

El padre, al escribir eso, está diciendo explícitamente **"este componente puede
llevarse a mi visitante"**. La intersección con los grants del padre hace el
resto, y no hace falta UX de consentimiento nueva.

Propuesta concreta: agregar `navigate_world` a los grants por defecto de
`spatial` —para que el raíz pueda delegarlo— y **no** agregar `navigate_global`.
Un include no lo recibe salvo que su padre lo escriba.

Se mantiene la exclusión que ya tiene `spawn`: `app` y `app-embedded` no lo
reciben ni delegándolo. Mover al visitante fuera del mundo no es una capacidad de
una app embebida.

## La llegada: `entry` y `returnTo`

Sin esto, cada mundo inventa su propia convención. En `server_noche` la
convención terminó siendo un parámetro llamado `volver`:

```
atrio  →  /cueva.hsml?volver=<atrio.hsml%23entry%3Ddesde_cueva>
cueva  →  location.href = new URLSearchParams(location.search).get('volver') || '/atrio.hsml'
```

Anda —**medido**: entrar por `#entry=desde_cueva` deja la cámara en
`(10.09, 1.70, 8.04)`, que es el arco de la cueva, mirando al centro— pero es
userland puro: las veintiocho escenas tuvieron que ponerse de acuerdo en el nombre
del parámetro. Es exactamente lo que `LOCATION.md` lista como pendiente bajo
«exportación de spawns y retorno».

Propuesta: que el destino lo lea de un objeto del runtime y no parseando query.

```js
hiperspace.arrival  // { entry: 'llegada', from: 'https://…/atrio.hsml', returnTo: {url, entry} }
```

Con eso, el portal de vuelta de cualquier escena es una línea que no sabe nada de
quién la invocó:

```js
const v = hiperspace.arrival.returnTo;
if (v) hiperspace.world.navigate(v.url, { entry: v.entry });
```

**`returnTo` es dato, no identificador.** Es una URL entera con su entry adentro,
así que cualquier mundo puede mandar a cualquier otro diciéndole por dónde
devolver, sin que ninguno de los dos aprenda nada del otro. Ésa es la propiedad
que hay que conservar.

## Interacción con `spawn`

Dos filos, y el segundo es una decisión de política, no de implementación.

**1. Remount vs. swap.** `SPAWN.md` dice que la aparición se aplica *una vez por
instancia del documento*, y que «ocultar/mostrar un documento ya utilizado no
repite su aparición». Si `navigate_world` se implementa como **cambiar el `src`
del include raíz** en vez de un remount limpio, volver a un mundo ya visitado
**no va a mover al visitante**, y no va a avisar. Silencioso.

Es el bug que yo predeciría de esta feature. Si va por swap, `entry` tiene que
forzar la re-aplicación explícitamente: es una orden, no una preferencia.

**2. Moverse dentro del mismo mundo es el teleport que `spawn` se negó a dar.**
`SPAWN.md` es explícito: «no requiere un diálogo de consentimiento: es una
capacidad limitada a la llegada a ese mundo, **no una API general para
teletransportar al usuario**». Y: «un cambio de fragmento tampoco constituye por
sí solo una orden de teletransporte».

`hiperspace.world.navigate({ entry: 'plaza_norte' })` **es** esa API general.
Hay que decidirlo a propósito, no que entre de costado. Tres notas para esa
decisión:

- Que sea una **llamada explícita** y no una asignación de fragmento respeta la
  letra de la regla actual: el fragmento sigue sin teletransportar; lo que
  teletransporta es pedirlo.
- El permiso puede ser el mismo `navigate_world`, o uno separado. Yo los separaría
  sólo si aparece un caso donde uno se quiera sin el otro.
- Mover al visitante sin que lo pida es la clase de cosa que marea en VR. La
  política de confort (fundido, o rechazo si el visitante se está moviendo) es
  parte del contrato, no un detalle de la aplicación.

## La ciudad de includes: el caso que pide todo esto

El caso que hace que la feature valga no es una puerta suelta, es un mundo grande
compuesto de muchos documentos incluidos —barrios, salas, piezas de terceros— con
carga por cercanía y LOD, y navegación libre entre ellos sin recargar el mundo.

Lo que ese caso necesita, además de `navigate_world`:

- **Descarga de includes lejanos.** Hoy se cargan todos y eager: **medido**, el
  atrio monta sus 28 includes de una, 564 entidades, 387 fps. Veintiocho anda; una
  ciudad no. Hace falta cargar y **descargar** por distancia.
- **Y ahí vuelve el filo 1**: si un include descargado y vuelto a cargar es «un
  documento ya utilizado», su `spawn` no se re-aplica. Para una ciudad, entrar dos
  veces al mismo barrio es lo normal, no el caso raro.
- **LOD de include**, que hoy no existe en ninguna forma. Nota de lo que **no**
  funciona, medido: la escala de un `<group>` **no** se aplica al contenido de un
  `<include>`. La idea de meter el mundo vecino encogido como maqueta —que sería
  el LOD más barato imaginable— no sale por ese camino.
- **Un presupuesto de mallas dinámicas que se recicle.** Ver abajo.

Vale notar que en una ciudad así, `navigate` dentro del mismo mundo deja de ser un
lujo y pasa a ser la operación principal: la mayoría de los viajes son entre
lugares del mismo documento, no entre documentos.

## Dependencia dura: la deuda de mallas

`docs/deuda_tecnica/deudas_de_plataforma.md`, deuda #10: las mallas dinámicas **no
se liberan al cambiar de espacio**, y hay 128 por sesión. Hoy cada puerta que se
toca gotea, y la única salida es reiniciar Luna.

`navigate_world` existe justamente para que saltar de mundo sea barato y
frecuente. O sea que **convierte una fuga lenta en el camino más rápido al tope**.
Recomendación: sale junto con la liberación por `scope` al desmontar, o después.
No antes.

## Qué **no** debería hacer

- No navegar el shell. Eso es `navigate_global` y es otra cosa.
- No concederse a `app` ni a `app-embedded`, ni por delegación.
- No inferir el destino de qué capabilities se tengan.
- No simular éxito si el host bloquea: igual que `location` hoy, la propiedad
  conserva la dirección comprometida hasta que el destino carga.

## Etapas sugeridas

1. **`hiperspace.world.navigate(url, { entry })`** con delegación por
   `<include resources="navigate_world">`. Sin historial, sin `returnTo`. Con esto
   sólo, una puerta de un tercero ya funciona.
2. **`hiperspace.arrival`** con `from`, `entry` y `returnTo`. Es lo que jubila la
   convención `?volver=` y lo que hace que dos mundos ajenos se compongan.
3. **Movimiento dentro del mismo mundo** (`navigate({ entry })`), con la decisión
   de política de la sección de spawn y la política de confort en VR.
4. **Historial**, si aparece la necesidad. Ojo: volver atrás tiene que restaurar
   *dónde estabas parado*, así que las entradas del historial cargan el mismo dato
   que `returnTo`. Historial sin eso no alcanza.
5. Carga/descarga de includes por distancia y LOD — la ciudad. Es el proyecto
   grande y depende de todo lo anterior más la deuda #10.

## Lo que ya funciona hoy, sin nada de esto

Importante para priorizar, porque baja la urgencia de una lectura y la sube de
otra. Un componente **cableado por el host** ya navega perfecto, y es lo que hace
el atrio ahora mismo. Las cuatro piezas, **medidas**:

| | |
|---|---|
| dos `<include>` del mismo archivo son dos isolates con dos URLs | sí |
| el hijo lee sus parámetros con `location.search` | sí |
| el **padre** ve los nodos del include con `getElementById` y les escribe atributos | sí |
| el hijo puede cambiarse su propio `id` con `setAttribute` | sí |
| el **padre** puede escuchar un `toque` en un nodo del include | **no** |

La última acota bastante lo que "cableado por el host" quiere decir.
`dispatch_toque_events_to_js` resuelve el destinatario con `find_owner_space_id`,
que sube hasta el `<space>` más cercano, así que el evento va sólo al isolate del
componente. **Se puede pintar a través del borde, no escuchar.**

El reparto que funciona hoy: **el componente dibuja, y el nodo tocable lo declara
el host**, encima del marco del include. Está implementado en
`server_noche/public/puerta.hsml` + `src/atrio.ts`.

Lo cual agrega un requisito a la propuesta que no estaba: si un componente ajeno
va a navegar, también tiene que poder **recibir su propio toque y actuar**, cosa
que hoy sí puede (el evento le llega a él). O sea que `navigate_world` cierra el
circuito justo donde hace falta — el componente ya escucha lo suyo; lo único que
le falta es poder mover el mundo.

Entonces lo que `navigate_world` compra de verdad **no es** "los componentes
pueden navegar". Es **que un componente ajeno navegue sin que el host lo cablee**.
Composabilidad con documentos de terceros.

Eso baja la prioridad si lo que viene son mundos propios, y la sube mucho si lo
que viene es que alguien publique una puerta —o un barrio— y otro lo use sin
hablar con él. Conviene decidir cuál de las dos es antes de gastar el rediseño.
