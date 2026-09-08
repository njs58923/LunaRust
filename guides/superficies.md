# Superficies: texturas e imágenes en HSML

Desde `add superficies` (`crates/luna/src/surface.rs`, 996 líneas) las primitivas
pueden llevar una textura, y existe un tag `<image>` que **sí renderiza** —la
guía todavía decía que no—.

Son dos cosas distintas y conviene separarlas desde el principio, porque el mismo
atributo `color` significa lo contrario en cada una:

|  | `<image src>` | `<box texture>` / `plane` / `sphere` / `cylinder` / `model` |
|---|---|---|
| qué es | **la imagen es el elemento** | una textura **encima** de una primitiva con color |
| atributo de origen | `src` | `texture` |
| atributo de ajuste | `fit` | `texture-fit` |
| `fit` por defecto | `contain` | `stretch` |
| `unlit` por defecto | `true` | como venga |
| **qué hace `color`** | **tiñe** la imagen (multiplica) | es el fondo; el dibujo lo pone la textura |
| API de carga | `onload`, `naturalWidth`, … | ninguna |

---

## `<image>`: la imagen es el elemento

```xml
<image src="/iconos/mapa.png" fit="contain" sx="0.2" sy="0.2"
       color="#8A94A6" material-alpha="blend"/>
```

Por dentro es un `plane` texturado (`dom.rs:1703` lo traduce), así que acepta
todo lo de un plano: transformaciones, `touchable`, `border-radius`.

**`material-alpha="blend"` no es opcional si el PNG tiene transparencia.** Sin
eso lo transparente se dibuja negro y el icono queda dentro de un cuadrado.

### Tiene la API de `HTMLImageElement`

Esto es lo que menos se espera y lo más útil:

```js
const img = root.getElementById('foto');
img.onload  = () => console.log('lista', img.naturalWidth, '×', img.naturalHeight);
img.onerror = () => console.log('falló:', img.error);
img.src = base + '/otra.png';        // vuelve a disparar load/error
```

| | |
|---|---|
| `src` | lectura y escritura |
| `naturalWidth`, `naturalHeight` | tamaño real en píxeles, `0` hasta que carga |
| `complete` | si el pedido vigente terminó (`ready`, `error` o `empty`) |
| `error` | el mensaje, o cadena vacía |
| `onload` / `onerror`, y `addEventListener('load'|'error')` | |

El host publica el estado en atributos que se pueden leer directamente:
`image-status` (`loading` \| `ready` \| `error` \| `empty`), `image-width`,
`image-height`, `image-error`.

> Que exista `naturalWidth` importa para cualquier capa de layout encima: es la
> diferencia entre poder acomodar una imagen por su tamaño real y tener que
> declararlo a mano. Ojo con el ciclo, igual: **no se sabe hasta que cargó**, así
> que un layout que corre antes tiene que medir de nuevo en `load`.

---

## `texture`: una textura sobre una primitiva

```xml
<plane color="#E8912A" border-radius="0.22"
       texture="/iconos/movie.png"
       texture-face="front" texture-fit="contain" texture-padding="0.24"/>
```

| atributo | valores | default | qué hace |
|---|---|---|---|
| `texture` | URL | — | de dónde sale |
| `texture-region` | `x,y,w,h` normalizados en [0,1] | `0,0,1,1` | recorta un pedazo: **es el atlas** |
| `texture-face` | `front` \| `all` | `all` | sólo la cara +Z, o todas |
| `texture-fit` | `contain` \| `cover` \| `stretch` | `stretch` | cómo entra en la cara |
| `texture-padding` | `[0, 0.5)` | `0` | aire alrededor: con `p` la textura ocupa `1 − 2p` |
| `texture-revision` | cualquier cadena | vacío | cambia el pedido sin cambiar la URL |
| `material-unlit` | `true` \| `false` | — | ignorar la iluminación |
| `material-alpha` | `opaque` \| `mask` \| `blend` | — | cómo tratar el canal alfa |

Aplica a `box`, `plane`, `sphere`, `cylinder` y `model`.

### `texture-face="front"` cambia el modo de composición

No es sólo "dibujá en una cara". Prende el modo **overlay** del shader
(`surface.rs:159`), y ahí:

```wgsl
base_color.rgb = mix(base_color.rgb, texel.rgb, texel.a)   // overlay
base_color    *= texel                                      // multiply (el otro)
```

Overlay compone el PNG **sobre** el color del nodo usando el alfa del PNG. La
consecuencia práctica, que cuesta descubrir:

> **En overlay, el color del dibujo lo pone la textura, y `color` es el fondo.**
> Con un PNG negro sobre un botón azul, el icono sale negro; para que salga
> blanco hay que usar un PNG blanco. En `<image>`, que va por multiply, es al
> revés: `color` tiñe.

Poner la textura en el **mismo nodo** que el color ahorra la mitad de los nodos:
un icono con fondo es un nodo, no dos, y no hay dos capas en z que puedan
parpadear.

### `texture-region` es para atlas

Un PNG y treinta y dos iconos. `tools/build_icon_atlas.py` arma uno de 32 casillas
con huecos transparentes y una huella de contenido —si las entradas no cambian,
el PNG no se reescribe—. El shell lo usa así:

```js
box.setAttribute('texture', 'luna://icons/menu.png');
box.setAttribute('texture-region', [(i % 8) / 8, Math.floor(i / 8) / 4, 1/8, 1/4].join(','));
```

El shader mete medio téxel de inset en cada celda, así que el muestreo lineal no
arrastra la casilla vecina.

**Un atlas es un pedido en vez de treinta y dos**, y eso importa más de lo que
parece — ver la sección de red, abajo.

---

## De dónde salen las URLs

`resolve_remote_path` (`render.rs:231`) resuelve en este orden:

1. una URL virtual (`luna://…`) se deja como está;
2. una `http://` o `https://` **absoluta se deja como está**, sea del host que
   sea;
3. cualquier otra cosa se resuelve contra la URL base del nodo.

O sea que **colgar texturas de un CDN funciona**. Y conviene saber cómo falla:

> Con veintisiete PNG pedidos en paralelo a jsDelivr, **dos no llegaban nunca**.
> Siempre los mismos dos, el nodo quedaba sin textura y a la vista no fallaba
> nada. El motor sí lo decía, en `warn`:
> `Image …/ic_chat_white_48dp.png: error sending request … operation timed out`.
>
> No era el formato (los 27 PNG eran idénticos: 96×96, 8 bits, gris+alfa) ni el
> tope de texturas. Era la red. Desde `localhost` no pasa.

Para cualquier cosa que tenga que dibujarse siempre —un menú, una barra— las
texturas van en `assets/`, y mejor en un atlas.

---

## Límites

| | |
|---|---|
| bytes por imagen | 16 MiB codificados |
| píxeles | 4096 × 4096 |
| texturas residentes | 128 |
| materiales | 1024 |
| memoria de texturas | 256 MiB |

La caché de texturas es **acotada y con LRU**: al llegar a 128 desaloja la más
vieja que no esté cargando. Si todas están cargando, el pedido nuevo falla con
`Too many pending textures`. Hay además un barrido por inactividad a los 300 s
para las que ya no tiene nadie montado.

A diferencia de las mallas dinámicas —que **no** se liberan al cambiar de
espacio, ver `docs/deuda_tecnica/deudas_de_plataforma.md`— las texturas sí se
reciclan solas.

---

## Errores

`SurfaceDesc::parse` valida y deja el motivo en el descriptor; el host lo publica
en `image-status` / `image-error` y lo manda al log como `warn`. Los mensajes:

- `texture-region requires normalized x,y,width,height inside [0,1]`
- `fit must be contain, cover or stretch`
- `texture-face must be front or all`
- `material-unlit must be true or false`
- `material-alpha must be opaque, mask or blend`
- `texture-padding must be in [0,0.5)`
- `Cannot resolve texture URL`
- `Image exceeds 16 MiB encoded limit`

En un `<image>` el nodo además se oculta mientras carga y si falla, así que un
error no deja un rectángulo blanco en pantalla.

---

## Ejemplo mínimo

`crates/luna/src/web/surface_demo.hsml` es la demo de la casa. Lo esencial:

```xml
<image id="foto" y="0.45" sx="2.8" sy="0.6" src="luna://icons/menu.png" fit="contain"/>
<text id="status" y="-0.02" value="Cargando imagen…" size="0.06"/>
```

```js
image.onload  = () => status.setAttribute('value',
                  'Imagen lista: ' + image.naturalWidth + ' × ' + image.naturalHeight);
image.onerror = () => status.setAttribute('value', image.error);
```

---

Volver al [índice de guías](index.md).
