# El layout del menú en VR

Maqueta de referencia para rehacer `crates/luna/src/web/ux/ux_vr.hsml`. No es
una propuesta de arte: es la aritmética del acomodo, escrita en un solo lugar y
mirada de cerca en Luna.

```
http://localhost:2057/menu.hsml            (server_noche)
http://localhost:2057/menu.hsml?modo=plano (el mismo layout sin curvatura)
```

Vive en `server_noche` pero **no es una escena del mundo**: no está en el atrio y
no se llega desde ningún lado. Los nombres son de relleno —quince genéricos—
porque lo que se prueba es dónde va cada cosa, no qué dice. Los iconos, en
cambio, son los de Material de verdad, bajados de un CDN.

Fuentes: `server_noche/src/menu.ts` (las medidas y los datos) y
`server_noche/public/menu.js` (la cuenta). Medido: **228 entidades, 430-450 fps**.

---

## Lo primero: dos bugs del motor que salieron de hacer esto

### 1. `border-radius` significa dos cosas distintas según la etiqueta

```rust
// box   — dom.rs:2341   rounded_box_radii(radius, transform.scale)
//         dom.rs:229    [ (r/sx).min(0.499), (r/sy).min(0.499), (r/sz).min(0.499) ]
//         → METROS del mundo, divididos por la escala de cada eje

// plane — dom.rs:2388   get_or_create_rounded_mesh(..., [radius, 0.0, 0.0], 6)
//         shapes.rs:741 let r = radius.clamp(0.0, 0.499); let h = 0.5 - r;
//         → FRACCIÓN del lado, sobre un cuadrado unitario, sin mirar la escala
```

El mismo atributo, dos unidades. Pasarle metros a un `plane` no falla ni avisa:
da esquinas casi rectas. Sólo se nota cuando uno intenta un círculo, que en
`plane` es `0.5` y en `box` sería la mitad del lado en metros.

**Es lo primero que arreglaría.** Cualquiera de las dos convenciones sirve; lo
que no sirve es que dependa de la etiqueta.

### 2. Por eso los iconos del shell son esferas

```js
// ux_vr.hsml:68-74
const SIZE_APP  = 0.22;
const BOX_DEPTH = 0.025;
const BORDER_R  = '0.2';      // ← metros
```

`rounded_box_radii(0.2, (0.22, 0.22, 0.025))` = `[0.499, 0.499, 0.499]`. Los tres
ejes saturados: una esfera. No es el shader ni el atlas, es el radio pasado de
rosca.

Y **bajarlo no alcanza**: el radio es uno solo para los tres ejes, así que en una
tarjeta de 0,025 de espesor cualquier esquina mayor a ~0,012 m ya redondea el
canto. Una tarjeta fina con esquinas grandes es inalcanzable con `box`.

Dos salidas, no excluyentes:

- **Radio por eje** en `box` (`border-radius="0.05 0.05 0.008"`), o un modo que
  redondee sólo en XY y deje Z recto.
- **Usar `plane`** para la cara del icono, que es lo que hace esta maqueta. El
  redondeo es 2D y no hay conflicto con el espesor. Si se quiere volumen, un
  `box` sin redondear un par de milímetros detrás.

---

## El modelo: nadie sabe dónde está

Cada pieza se registra con **dos números y un radio**, nunca con una posición:

| | |
|---|---|
| `u` | cuánto se corre sobre el arco, en metros |
| `y` | altura absoluta |
| `radio` | en qué cilindro vive (el panel y la barra usan dos distintos) |

Una sola función traduce ese par a una transformación:

```js
function enArco(u, radio, modo) {
  if (modo === "plano") return { x: u, z: -radio, ry: 0 };
  const t = u / radio;
  return { x: Math.sin(t) * radio, z: -Math.cos(t) * radio, ry: -t };
}
```

El giro es **`ry = -t`**, y conviene anotarlo porque se confunde sola: `R_y(t)`
lleva el `+Z` local a `(sen t, 0, cos t)`, y la dirección de la pieza al origen
es `(-sen t, 0, cos t)`. Es **la opuesta** a la de orientar un objeto hacia el
centro de una sala (`atan2(-x, -z)`), y la opuesta también a la del `<spawn>`.

Que la posición sea derivada y no escrita es lo que hace que alternar plano y
curvo sea recorrer una lista. Si cada nodo trajera su `x` y su `z`, el modo curvo
habría que rehacerlo a mano — y es exactamente la situación del shell de hoy,
donde `computePositions` devuelve `[x, y]` planos y no hay dónde meter el arco.

Las otras dos funciones que hay que portar:

```js
distribuir(cantidad, paso)   // n posiciones centradas en cero, paso fijo → la grilla
fila(anchos, hueco)          // anchos distintos, centrados → la barra
```

`distribuir` no sirve para la barra porque ahí un reloj mide más que un icono;
`fila` no sirve para la grilla porque tiene que alinear columnas entre filas.
Son dos problemas distintos y conviene que sean dos funciones.

---

## Las medidas

```
radio del panel      1.60 m
radio de la barra    1.25 m      (más cerca: está más abajo y más al alcance)

icono                0.170 m     esquina 0.22 (fracción)
paso  x / y          0.240 / 0.300 m
grilla               5 x 3
etiqueta             0.028 m, a -0.122 del centro del icono

tira de fijados      icono 0.150, paso 0.235, y = 2.00
grilla               y = 1.38   (ojos a 1,70 → 13° abajo)
barra                y = 0.76, item 0.072 (círculo, fracción 0.5)

capas en z:
  icono  0     texto  +0.005     fondos  radio + 0.008
```

Las capas en Z no son opcionales: sin separarlas el motor decide el orden por su
cuenta y las etiquetas parpadean contra su propio fondo. El icono ya no necesita
capa propia: va como textura sobre el mismo nodo del squircle.

### La inclinación de la barra sale de la geometría

```
rx = -atan((1.70 - yBarra) / radioBarra) = -atan(0.94 / 1.25) ≈ -0.64 rad
```

La barra está 94 cm por debajo de los ojos: si no devuelve ese ángulo se le ve el
canto. Con `rx = 0` —que es lo que hace el shell hoy— la barra es una línea.

### El ancho del panel sale de la grilla, no al revés

```js
anchoPanel = (columnas - 1) * pasoX + icono + padding
```

Y el alto, de los extremos reales incluida la etiqueta que cuelga de la última
fila. Escribir el tamaño de la placa a mano es lo que obliga a retocar dos
números cada vez que se agrega una fila.

---

## Tres decisiones que la maqueta toma y valen discutir

**La barra se parte en tres placas planas, no es una pastilla curva.** Una
pastilla de 1,2 m sobre un cilindro habría que segmentarla; partida en tres
grupos —estado, notificaciones, accesos— cada placa es chica, la curvatura no se
nota, y de paso queda dicho qué cosa va con qué.

**Los accesos de la barra son círculos y los de la grilla squircles.** Es lo que
los distingue de un vistazo sin leer nada.

**La tira de fijados está fuera de la grilla, con su propia placa.** No participa
de la paginación: los puntos de página son de la grilla, no de todo el panel.

---

## Lo que la curvatura sí y no resuelve

Honestamente: **a este tamaño casi no se nota**. El panel abarca ±22°, y entre
plano y curvo la columna del borde cambia 4 % de distancia y 17° de encaramiento.
De frente las dos capturas son casi la misma.

La curvatura empieza a pagar cuando el panel se ensancha —siete u ocho columnas,
o el panel más la barra vistos al girar la cabeza— y ahí sí las puntas de un
plano quedan de costado y más lejos. Como el shell va a crecer, conviene que el
arco esté desde el principio: **meterlo después es tocar todas las posiciones**,
que es justo lo que este modelo evita.

Por eso la maqueta trae el interruptor (`?modo=plano`, o el botón a la
izquierda): la comparación es de a dos capturas con la misma cámara, no de
memoria.

---

## La gota: la selección como malla animada

La marca de selección no es un anillo quieto detrás del icono: es una **gota que
late y se deforma**, del tipo que dibujan los asistentes de voz mientras hablan.

Va en su propia malla, y la razón es la misma que hizo falta para los fondos: lo
que cambia en cada cuadro es **la forma**, no la escala. Setenta y dos radios
distintos por cuadro no son setenta y dos nodos: son un `update` sobre un abanico
de triángulos.

```js
r(θ, t) = R · escala · (1 + amp · Σ aₖ · sen(k·θ + vₖ·t + φₖ))
```

Cuatro lóbulos con `k = 3, 4, 5, 7` y velocidades distintas —y de signo distinto,
para que giren en sentidos opuestos—. Si dos comparten período la gota late como
una flor y se nota el truco.

El tamaño **no interpola: rebota**. Un resorte con amortiguación floja pasa el
objetivo y vuelve, que es lo que la hace leer como algo blando:

```js
velocidad += (objetivo - escala) * 190 * dt - velocidad * 15 * dt;
escala    += velocidad * dt;
```

Y la deformación tiene una variable aparte, `energia`, que un toque sube de golpe
y decae sola. Es lo que convierte un click en un chapoteo en vez de un cambio de
estado.

### Los tres estados

Como **hover todavía no existe en el motor**, «hover» acá es el click, y el ciclo
es de tres:

| | |
|---|---|
| tocar un icono nuevo | la gota se apaga en el anterior y **nace** en el nuevo, con chapoteo |
| tocarlo otra vez | **se hincha** (`R` de 0,115 a 0,152) con el chapoteo más grande |
| tocarlo una tercera vez | **se va** |

La mudanza no es un salto: la gota se apaga donde estaba y recién cuando su
escala baja de 0,12 aparece en el destino. Saltar se lee como parpadeo; irse y
volver se lee como que siguió al dedo.

Sin alfa en las mallas, la profundidad la dan los colores por vértice: centro
blanco, borde gris azulado. Alcanza para que se lea como halo y no como un disco.

---

## Los iconos: `texture` en overlay, y cuándo `<image>`

Los 27 iconos son Material de Google. Están en `public/iconos/`, servidos por el
mismo servidor que el documento; `iconos.sh` los vuelve a bajar de jsDelivr:

```
https://cdn.jsdelivr.net/gh/google/material-design-icons@3.0.1
      /<categoria>/2x_web/ic_<nombre>_white_48dp.png
```

Dos cosas que costaron un rato y conviene no volver a averiguar:

- **El tag 3.0.1, no `master`.** De la versión 4 en adelante el repo dejó de
  publicar los PNG blancos; en `master` sólo quedan los negros. Y hacen falta
  blancos, por lo que sigue.
- **La categoría importa y no es adivinable.** `apps` está en `navigation`, no en
  `action`; `music_note` y `camera_alt` están en `image`; `people` en `social`.
  De 25 URLs armadas de memoria, 24 dieron 200 y una 404.

El motor resuelve una textura remota sin más —`resolve_remote_path` deja pasar
cualquier `http(s)` absoluta tal cual— y colgarlas del CDN **funcionó**, un rato.

Pero de veintisiete pedidos en paralelo, **dos no llegaban nunca**. Siempre los
mismos dos, el icono quedaba en blanco y a la vista no fallaba nada; el motor sí
lo decía, en `warn`:

```
Image …/ic_chat_white_48dp.png: error sending request … operation timed out
```

Verifiqué que no fuera el formato (los 27 PNG son idénticos: 96×96, 8 bits,
gris+alfa, sin entrelazar) ni el tope de texturas (`MAX_TEXTURES = 128`, y hay
27). Es la red y nada más. Desde localhost no pasa.

Para el shell la conclusión es la misma y más fuerte: **un menú que necesita
internet para dibujarse no es un menú.** Van en `assets/`.

### Overlay: el icono va en el mismo nodo que el squircle

```xml
<plane color="#E8912A" border-radius="0.22"
       texture="…/ic_movie_white_48dp.png"
       texture-face="front" texture-fit="contain" texture-padding="0.24"/>
```

Con `texture-face="front"` sobre una etiqueta que no es `image`, el motor prende
el modo overlay (`surface.rs:159`) y el shader hace:

```wgsl
base_color.rgb = mix(base_color.rgb, texel.rgb, texel.a)
```

Es decir **compone el PNG sobre el color de la pieza usando el alfa del PNG**. Un
nodo por icono en vez de dos, sin capa extra en z que pueda parpadear. Sacar los
glifos de geometría y poner texturas bajó la escena de 321 a 235 entidades
haciendo *más* cosas.

De ahí sale la exigencia de los PNG blancos: **en overlay el color del glifo lo
pone la textura**, y `color` es el del fondo. Con los negros el glifo saldría
negro sobre el color de la app.

`texture-padding` es lo que lo mete hacia adentro: con `p`, la textura ocupa
`1 - 2p` del lado. Sin eso el icono llega hasta el borde redondeado y el squircle
deja de leerse. 0,24 en la grilla, 0,26 en los círculos de la barra.

### `<image>`: cuando la imagen *es* el elemento

El wifi y la batería de la barra no tienen pastilla detrás, y ahí va la otra
etiqueta:

```xml
<image src="…/ic_signal_wifi_4_bar_white_48dp.png" fit="contain"
       color="#8A94A6" material-alpha="blend"/>
```

Las diferencias, todas medidas:

| | `plane` + `texture-face="front"` | `<image src>` |
|---|---|---|
| modo del shader | overlay: `mix(base, texel, texel.a)` | multiply: `base_color *= texel` |
| quién decide el color del dibujo | **la textura** | **el atributo `color`** (tiñe) |
| fondo | el `color` de la pieza | ninguno, transparente |
| `fit` por defecto | `stretch` | `contain` |
| `unlit` | como venga | `true` |

Que `color` signifique cosas opuestas en las dos es lo mismo que pasa con
`border-radius`: **el atributo cambia de sentido según la etiqueta**. Acá al
menos es útil —el estado en gris, los iconos de app en blanco— pero conviene que
esté documentado y no descubierto.

Y `material-alpha="blend"` no es opcional en `<image>`: sin eso lo transparente
del PNG se dibuja negro y el icono queda dentro de un cuadrado.

---

## Los fondos van en una malla, no en `<plane>`

Es el cambio que más ordena el resultado, y salió de un síntoma raro: con la
placa de un grupo de la barra retranqueada 6 mm, **el plano de la placa cortaba
al ítem del extremo** — el avatar salía con un mordisco recto, como un pac-man.

La causa es que un `<plane>` es plano y las piezas van sobre un cilindro. Cada
pieza es tangente **en su propio punto**, así que la placa y el ítem del extremo
son dos planos que se cruzan con un ángulo de `d / R`, y cerca del borde la
separación entre ambos se vuelve del orden del retranqueo: lo decide el z-buffer.

La flecha del arco sobre el ancho de un fondo es `d² / 2R`, y para el panel
—1,27 m de ancho sobre radio 1,6— eso da **12 cm**. Retranquear no es una
solución: la placa se hunde justo donde los iconos se adelantan.

Así que los cinco fondos —panel, tira de fijados y los tres grupos de la barra—
son **una sola malla dinámica que sigue el arco**: 380 triángulos en un nodo.

```js
// Para cada muestra a lo largo del fondo:
//   n(t)   = (-sen t, 0, cos t)                    hacia el visitante
//   up(t)  = (sen t · sen φ, cos φ, -cos t · sen φ)  el "arriba" ya inclinado
//   normal = n · cos φ + (0,1,0) · sen φ
// y se emiten dos vértices, centro ± up · medioAlto(x).
```

`medioAlto(x)` es el perfil: recto en el medio, arco de radio `r` en las puntas.
Eso da la forma de pastilla sin depender de `border-radius`, que en un `plane`
es una fracción y no habría sobrevivido al cambio de ancho.

El orden de los índices sale de que `(tangente, up, normal)` es una terna a
derechas: `tangente × up = normal`, así que los triángulos van
`(abajo_i, abajo_i+1, arriba_i)` y `(abajo_i+1, arriba_i+1, arriba_i)`.

Dos cosas que se ganaron aparte de que deje de cortar:

- **Los iconos se pudieron acercar.** El fondo sigue el mismo arco, así que 8 mm
  de separación valen igual en el centro que en la punta. Con el plano hacía
  falta retranquear 30 mm, y a esa distancia el icono se lee despegado.
- **Cinco nodos menos**, y el fondo entero es un `<model>`.

La malla se **actualiza** al alternar plano y curvo en vez de recrearse: el
número de vértices no cambia, y `MeshResource` tiene un tope de 128 recursos que
además no se liberan al cambiar de espacio.

### Dos trampas de la API, las dos silenciosas

**1. Los arrays tienen que ser tipados y planos.**

```js
{ positions: Float32Array,  // x,y,z,x,y,z…  no [[x,y,z], …]
  indices:   Uint32Array,
  normals:   Float32Array,
  colors:    Float32Array }  // r,g,b,a en [0,1]
```

Pasarle arrays de ternas no da un error de tipo: `mesh.js` los acepta y hace
`new Float32Array(value)`, que sobre un array de arrays da `NaN` en cada
posición. El mensaje que sale es `Mesh attributes must be finite and bounded`, y
es literal — el `NaN` lo fabricó la conversión.

**2. `update` manda los cinco buffers, siempre.**

```js
malla.update({ positions, normals });   // ← MAL
malla.update(d);                        // ← los cuatro
```

En `mesh.js`, `submit` llama a la op con las cinco entradas y las que uno no pasa
viajan **vacías**, no "sin cambios". Y del lado de Rust un `indices` vacío
significa *índices implícitos* —0,1,2, 3,4,5…—, así que la malla se rearma como
tiras de triángulos entre vértices consecutivos: en pantalla, un erizo de púas
donde había pastillas.

Es la optimización obvia —mandar sólo lo que cambió— y está mal. Si el motor
quiere permitirla, un buffer ausente debería distinguirse de uno vacío.

---

## Lo que ya existe en el motor y conviene usar

Salió al leer el shell actual, y no está en la guía:

- **`texture-region`** (atlas normalizado x,y,w,h) permite un solo PNG para los
  32 iconos en vez de 25 pedidos. El shell ya lo hace con `assets/icons/menu.png`;
  la maqueta pide uno por icono sólo porque vienen de un CDN ajeno.
- **`material-alpha` acepta `opaque|mask|blend`** y **`material-unlit`** existe.
- **`visible` existe de verdad** y hace lo correcto: sin el atributo hereda, en
  `false` fuerza oculto y propaga (`dom.rs:331`).
- **`root.createElement` + `appendChild`** es barato: las 321 entidades de esta
  escena se crean en un frame sin que baje de 430 fps.
