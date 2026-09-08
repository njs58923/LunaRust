# El documento: anatomía, tags y atributos

Cómo se arma un `.hsml` y qué acepta cada etiqueta. Para lo que hay que saber
antes de escribir la primera línea, ver el [índice](index.md); para las trampas,
[trampas.md](trampas.md).

---

## Anatomía de un documento

```xml
<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Mi espacio</name>
    <meta type="position" x="0" y="0" z="0"/>
    <meta type="rotation" x="0" y="0" z="0"/>
    <meta type="scale"    x="1" y="1" z="1"/>
  </head>

  <space resources="navigate_self,skybox">
    <!-- contenido -->
    <script src="app.js"/>
  </space>
</hsml>
```

- `<head>` no se renderiza. `<name>` es el título. Los `<meta type="...">` son
  **decorativos**: ningún código del motor los lee, pese a lo que decía la guía vieja.
  Para mover el documento, usá un `<group>` con transform alrededor del contenido.
- `<space>` es la raíz visible y **donde se declaran los permisos** (`resources`).
- `<script src="...">` se resuelve relativo a la URL del documento; también acepta
  `luna://internal/*.js`. Corre en el isolate del espacio.

### Tags válidos

De `tags.rs`, la lista completa:

| Tag | Qué es |
|---|---|
| `hsml`, `head`, `name`, `meta`, `state` | estructura / metadatos |
| `script` | carga y ejecuta JS |
| `space` | raíz del espacio, portadora de `resources` |
| `include` | embebe otro documento HSML (sub-documento, permisos propios) |
| `group` (alias `div`) | nodo de agrupación / transform |
| `box`, `sphere`, `cylinder`, `plane` | primitivas |
| `text` | texto renderizado a textura sobre un plano |
| `model` | malla glTF/GLB |
| `skybox` | cubemap de fondo (requiere permiso) |
| `posezone` | volumen invisible que emite eventos `posemove` de manos/mandos |
| `spawn` | punto invisible de aparición del visitante; requiere `resources="spawn"` |
| `image` | plano texturado; `src`, `fit`, `naturalWidth`, `onload`. Ver [SUPERFICIES.md](superficies.md) |

> `runtime.js` define además `HSMLButtonElement` y `HSMLVideoElement`, pero los tags
> `button` y `video` no existen en `tags.rs` ni en `dom.rs`: son vestigios de la capa
> de compat con Unity. No los uses.

---

---

## Atributos

### Transformación (cualquier nodo)

| Atributo | Significado |
|---|---|
| `x`, `y`, `z` | posición local (metros) |
| `rx`, `ry`, `rz` | rotación local (radianes) |
| `s` | escala uniforme |
| `sx`, `sy`, `sz` | escala por eje |
| `visible` | `"false"` oculta el subárbol |

Las unidades son metros y el suelo está en `y = 0`. Altura de ojos ≈ `1.6`.
`-z` es "hacia adelante" desde el origen del espacio.

### Por tag

| Tag | Atributos |
|---|---|
| `box`, `sphere`, `cylinder`, `plane` | `color="#RRGGBB"`, `touchable`, `border-radius` (sólo `box` y `plane`), y los de textura (abajo) |
| `image` | `src`, `fit`, `color` (tiñe), `material-alpha`; expone `naturalWidth`/`naturalHeight`/`onload`/`onerror` |
| `text` | `value` (default `"Text"`), `size` (default `0.1`), `color` (default blanco) |
| `model` | `src` (glTF/GLB, relativo o absoluto), `rigidbody`, `collider`, y los de animación (abajo) |
| `skybox` | `src` = **patrón con `$1`** (ver abajo) |
| `include` | `src`, `resources` |
| `space` | `resources`, `system-space` |

`touchable` acepta `true`, `1`, `yes`, `on`. Sin él, el nodo no recibe `toque`
**ni hover**.
La forma de colisión sale del tag: `box`→caja, `plane`→plano, `sphere`/`cylinder`→esfera.

### Textura sobre una primitiva

`box`, `plane`, `sphere`, `cylinder` y `model` aceptan una textura encima del
color. Resumen; el detalle está en **[SUPERFICIES.md](superficies.md)**.

| Atributo | Valores | Default |
|---|---|---|
| `texture` | URL (absoluta, relativa o `luna://`) | — |
| `texture-region` | `x,y,w,h` normalizados — **es el atlas** | `0,0,1,1` |
| `texture-face` | `front` \| `all` | `all` |
| `texture-fit` | `contain` \| `cover` \| `stretch` | `stretch` |
| `texture-padding` | `[0, 0.5)`; con `p` la textura ocupa `1 − 2p` | `0` |
| `texture-revision` | cadena; rehace el pedido sin cambiar la URL | vacío |
| `material-unlit` | `true` \| `false` | — |
| `material-alpha` | `opaque` \| `mask` \| `blend` | — |

**`texture-face="front"` no es sólo "una cara": cambia el modo de composición.**
Prende el modo *overlay*, donde el shader hace
`mix(color_del_nodo, texel, texel.a)` — o sea que **el color del dibujo lo pone
la textura y `color` es el fondo**. Con un PNG negro sobre un botón azul el icono
sale negro. En un `<image>`, que va por *multiply*, es al revés: `color` tiñe.

El ancho del `text` se calcula como `size * nChars * 0.6`; es una aproximación
monoespaciada, no layout real. Para centrar, colocá el texto en el mismo `x` que su fondo.

### Animación de un `<model>`

Los clips que trae el `.glb` se controlan por atributos (`model_animation.rs`).
**Sin `animation-clip` no se reproduce nada**: es el único que no tiene default
útil.

| Atributo | Default | Qué hace |
|---|---|---|
| `animation-clip` | *(vacío = no anima)* | nombre del clip, o su índice (`"0"` es el primero) |
| `animation-state` | `playing` | `playing`, `paused` o `stopped` |
| `animation-loop` | `true` | `true`/`1` o `false`/`0` |
| `animation-speed` | `1` | multiplicador, no negativo |
| `animation-time` | — | busca un instante del clip, en segundos |
| `animation-restart` | — | cambiar su valor reinicia el clip desde cero |

El motor **escribe de vuelta** tres atributos sobre el nodo, que se pueden leer
desde JS con `getAttribute`:

| Atributo | Qué trae |
|---|---|
| `animation-clips` | JSON con el catálogo de clips del modelo: nombres y duraciones |
| `animation-status` | `idle`, `playing`, `paused`, `stopped` o `error` |
| `animation-error` | el motivo, cuando `animation-status` es `error` |

`animation-clips` es la forma de averiguar cómo se llaman los clips de un
modelo ajeno sin abrirlo en Blender.

```xml
<model src="bicho.glb" animation-clip="0" animation-loop="true"/>
```

### Skybox

`src` es un patrón donde `$1` se sustituye por cada cara, en este orden:

```
pz, nz, nx, px, py, ny
```

```xml
<skybox src="http://localhost:2052/cielo/$1.png"/>
```

Carga las 6: `cielo/pz.png`, `cielo/nz.png`, … Requiere `resources="skybox"`;
sin el permiso el nodo se monta vacío y se loguea un warning.

---

---

Volver al [índice de guías](index.md).
