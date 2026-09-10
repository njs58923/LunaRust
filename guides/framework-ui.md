# Interfaz declarativa: `luna://internal/ui.js`

Un panel con seis opciones se puede escribir a mano —seis `<box>`, seis `<text>`,
todos en coordenadas absolutas— y funciona. Con dieciséis deja de funcionar:
mover una fila obliga a recalcular las de abajo, y no hay forma de que algo
crezca con su contenido.

El motor trae un framework de interfaces para eso, en la línea de WPF: árbol
declarativo, layout en dos pasadas —medir y acomodar—, paneles que reparten el
espacio, y enlaces a datos. Conserva **mallas por panel**, con la raíz como límite
implícito. Los fondos y controles comparten geometría; el texto usa mallas por
panel y página de atlas. Las imágenes y los blancos táctiles mantienen nodos
propios. No hay una garantía de un único nodo o draw call por ventana.

Es un subconjunto inspirado en WPF, no una implementación de XAML ni del DOM
HTML. El marcado pasado a `app.cargar()` lo interpreta la biblioteca JavaScript;
`<Border>` o `<Button>` no son etiquetas HSML nativas.

```xml
<space resources="navigate_self">
  <model id="ui_malla" material-unlit="true"/>
  <group id="ui_nodos"/>

  <script>
    globalThis.UI_CFG = { base: "luna://", distancia: 1.6, C: { texto: "#FFFFFF" } };
  </script>
  <script src="luna://internal/ui.js"/>
  <script>
    (function arranque() {
      if (!globalThis.UI) return void requestAnimationFrame(arranque);
      const app = new UI.Aplicacion({ ancho: 1.2, alto: 0.8, datos: { n: 0 } });
      app.cargar(`
        <Border Background="#1C1C26" CornerRadius="0.02" Padding="0.024">
          <StackPanel Orientation="Vertical" Spacing="0.012">
            <TextBlock Text="{Binding n, Format=Toques: %v}" FontSize="0.04" Foreground="#FFFFFF"/>
            <Button Content="Tocá" Background="#0A84FF" Click="sumar"/>
          </StackPanel>
        </Border>`);
      app.manejadores.sumar = () => { app.datos.n++; app.invalidar(); };
      app.correr();
    })();
  </script>
</space>
```

---

## Los tres scripts, y por qué son tres

El orden importa y no está garantizado, así que cada pieza se defiende sola:

1. **`UI_CFG` va en línea.** Un `<script>` sin `src` no espera a la red y se
   evalúa antes que cualquier externo; el framework lee la configuración **al
   cargarse**. Al revés —la config en un archivo aparte— la paleta llega tarde y
   los colores salen con los valores de reserva, sin un solo aviso.
2. **`ui.js` va con `src`.**
3. **Tu aplicación espera al framework** en un `requestAnimationFrame`. Es el
   mismo patrón que usa el shell con `shell_ui.js`, y está ahí porque los
   `<script src>` llegan asincrónicos: la aplicación puede ganarle la carrera a
   la biblioteca y morir con `ReferenceError`. Está medido.

Los dos nodos —`ui_malla` y `ui_nodos`— son los puntos de montaje. El primero
recibe la geometría general; bajo el grupo se crean también los paneles retenidos,
las mallas de texto, las imágenes y los blancos táctiles. El backend configura
sus mallas como unlit; declarar así también `ui_malla` mantiene los colores
independientes de la iluminación del mundo.

`Aplicacion` acepta `x`, `y`, `z` como centro del panel (por defecto 0, 1.62 y
`-UI_CFG.distancia`, con distancia de reserva 1.6), además de `ancho`, `alto`,
`datos`, `plantillas`, `estilos` y `manejadores`. Para varias aplicaciones en el
mismo documento, proporcionar contenedores distintos mediante las opciones
`malla` y `nodos`, cuyos valores son ids.

## Lo que hay

| | |
|---|---|
| **paneles** | `StackPanel`, `Grid`, `DockPanel`, `WrapPanel`, `Canvas` |
| **decoradores** | `Border`, `RenderPanel`, `ScrollViewer`, `Separator` |
| **texto e imagen** | `TextBlock` (con `TextWrapping="Wrap"`), `Image` |
| **controles** | `Button`, `CheckBox`, `RadioButton`, `ToggleSwitch`, `Slider`, `ProgressBar` |
| **capa de arriba** | `ComboBox`, atributo `ToolTip`, y `app.superponer()` para diálogos modales |
| **vectores** | `Path` (`M L Q C Z`), `Ellipse` |
| **listas** | `ItemsControl` con plantilla, reciclado y virtualización |
| **navegación** | `TabControl` / `TabItem` |
| **enlaces** | `{Binding ruta, Mode=TwoWay, Format=%v %}` |
| **estilos** | juego de atributos por tipo de elemento |

Las distancias de layout son **metros locales**, con X hacia la derecha e Y
hacia abajo. El backend convierte a Y arriba. `Margin` y `Padding` aceptan un
valor, dos (horizontal, vertical) o cuatro (izquierda, arriba, derecha, abajo).
`Width`/`Height`, mínimos, máximos y alineaciones controlan la medida; `Grid`
admite columnas/filas fijas, `Auto` y proporciones `*`.

`Depth` extruye el fondo hacia +Z local (0 por defecto, máximo 1 m). No extruye
letras ni trazados arbitrarios y no cambia la altura de layout. `Elevation`
desplaza el elemento y su contenido entre -1 y 1 m; no se acumula entre hermanos.
`CornerRadius` redondea la silueta, no bisela el canto del volumen.

`Background="transparent"` y colores `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`
conservan el alfa **al final**, a diferencia del formato ARGB de WPF. La
transparencia del fondo no se aplica a sus hijos. Se usa mezcla alfa convencional,
no un compositor independiente del orden; varias superficies transparentes
intersectadas todavía pueden mostrar limitaciones de ordenación.

## Lo que no hay

- **No hay entrada de texto.** Ni `TextBox` ni IME. Una URL se elige de un
  `ComboBox`, no se escribe.
- **No hay `ListBox` con selección.** `ItemsControl` dibuja la lista; la
  selección la lleva la aplicación. Un control adentro de la plantilla sabe de
  qué fila es con `itemDeLista()`.
- No hay sombras, recortes arbitrarios, foco de teclado, captura de puntero ni
  drag continuo. `CornerRadius` no recorta automáticamente los hijos con una
  máscara redondeada.
- La tipografía usa Fira Sans y atlas bitmap compartido. No hay shaping complejo,
  fallback completo de fuentes ni SDF.

## Visibilidad: el framework hereda

Los nodos que crea el framework —las mallas de los paneles, las de texto, las
imágenes— se muestran con `visible="inherit"`, nunca con `"true"`. La diferencia
no es cosmética: `"true"` se dibuja aunque un padre esté oculto, y sin
`removeAttribute` no hay vuelta atrás.

Con `"inherit"`, ocultar el `<group>` o el `<space>` que contiene la interfaz la
oculta entera, que es lo que espera cualquiera que minimice una ventana. Hay una
prueba que falla si algún nodo vuelve a quedar pinneado en `"true"`.

## Bindings y propiedades

Un binding debe ocupar **todo** el valor del atributo. Para prefijos o sufijos,
usá `Format`, que sustituye `%v`:

```xml
<TextBlock Text="{Binding n, Format=Toques: %v}"/>
<Slider Value="{Binding potencia, Mode=TwoWay}"/>
```

`Text="Toques: {Binding n}"` es texto literal: no existe interpolación dentro
de cadenas. Las rutas admiten campos separados por puntos; no son expresiones
JavaScript. `TwoWay` escribe de vuelta sólo en controles que implementan esa
propiedad. Los bindings se aplican a las propiedades registradas en el `mapa`
de cada control y a `Depth`/`Elevation`, no a cualquier atributo de XAML.
En particular, no hay binding general de `Visibility` ni de `Width`/`Height`.

`Name` permite buscar un elemento con `app.buscar("nombre")`; no crea un id de
nodo HSML. Los estilos son un objeto JavaScript pasado como `estilos`, con
atributos por nombre de tipo, sin triggers ni cascada CSS.

## Invalidación, tamaño y cambio de vista

El framework no mantiene un RAF permanente. Agrupa invalidaciones en un frame
pendiente; las animaciones explícitas solicitan frames durante su intervalo.
La espera inicial de la biblioteca y cualquier polling escrito por la aplicación
son independientes de ese mecanismo. Ajustes, por ejemplo, todavía sondea su
buzón mediante su propio RAF.

```js
const boton = app.buscar("accion");
boton.elevation = 0.03;
boton.invalidateRender();       // apariencia del panel afectado
boton.contenido = "Otro texto";
boton.invalidateMeasure();      // bindings y layout
app.invalidateArrange();        // colocar otra vez, sin medir
app.datos.n++;
app.invalidar();                // bindings y layout completos
app.redimensionar(1.6, 0.95);   // conserva paneles y recursos, centra el origen
```

Cambiar un objeto JavaScript no invalida por sí solo. Los nombres de propiedades
en JavaScript no siempre coinciden con el marcado (`Content` → `contenido`,
`Background` → `fondo`). Usá `invalidateRender()` para cambios de apariencia e
`invalidateMeasure()`/`invalidar()` cuando cambie el contenido o el layout.

`RenderPanel` (o `RenderPanel="true"` en un control) crea un límite de geometría
retenida. Un cambio de apariencia invalida su panel y sus ancestros, conservando
los vecinos. El panel modificado reconstruye sus buffers; no hay actualización
parcial de rangos de vértices.

`app.redimensionar(ancho, alto)` acepta valores finitos positivos. Actualiza la
medida y el origen sin recrear la aplicación. Asignar sólo `app.ancho`/`app.alto`
no actualiza correctamente ese origen.

`app.cargar(marcado)` **reemplaza el árbol**, no lo reconcilia. No lo llames en
cada click para alternar dos secciones: reemplazar el panel obliga a cargar nuevas
mallas y puede producir un hueco visible. Conservá la raíz y alterná los hijos,
como hace `settings_ui.js`:

```js
function mostrar(elemento, activo) {
  elemento.visible = activo; // false colapsa el espacio de layout
  elemento.opaco = activo;  // habilita o impide emitir su contenido
}
mostrar(app.buscar("detalleMcp"), seccion === "mcp");
mostrar(app.buscar("detalleNav"), seccion === "nav");
app.invalidar();
```

En el marcado inicial se admite `Visibility="Visible"`, `Hidden` (reserva
espacio) o `Collapsed`. Al pasar desde `Collapsed` a visible en JavaScript,
ajustá ambos campos como en el ejemplo. `opaco` aquí significa que se dibuja;
no controla el alfa del color.

`app.detener()` cancela el frame del framework; `app.correr()` lo reactiva.
`app.dispose()` libera los recursos y nodos que administra. Los timers/listeners
externos creados por tu aplicación siguen siendo responsabilidad de ella.

## Curvas y texto

```xml
<Ellipse Width="0.3" Height="0.15" Fill="#9050CC80"
         Stroke="#FFFFFF" StrokeThickness="0.003"/>
<Path Data="M0 0.1 C0.1 -0.1 0.3 0.3 0.4 0.1 Q0.5 0 0.6 0.1"
      Stroke="#67DFCA" StrokeThickness="0.005" Height="0.2"/>
```

`Path` admite `M L Q C Z` absolutos/relativos y subtrazados; `FillRule="EvenOdd"`
es el valor de reserva y permite huecos, con `NonZero` opcional. `Stretch`
admite `Uniform` (por defecto), `Fill` y `None`. `Tolerance` se mide antes del
escalado del trazado (0.00005 por defecto en la UI). No admite SVG completo,
arcos `A`, gradientes ni extrusión de paths. `Path`/`Ellipse` requieren el backend
nativo `PathGeometry`; el resultado sigue siendo una malla de triángulos.

Las esquinas se triangulan con tolerancia adaptativa y caché acotada, sin ajuste
continuo por distancia a cámara. Los límites nativos por trazado y la API de
`TextLayout`/`PathGeometry` están en [interfaz.md](interfaz.md).

El texto se agrupa por panel/página de atlas y su recorte rectangular ajusta
posiciones y UV de cada glifo. `TextWrapping="Wrap"` usa el ancho disponible.
Las imágenes conservan nodos propios. Para validar estas funciones usá una build
actual; el fallback de texto con nodos de builds antiguas tiene otras limitaciones.

---

## Cómo llega acá

El framework **no se escribe en este repo**: vive en `server_ui`, con sus demos,
su banco de pruebas y las pruebas automatizadas sin motor. `bun run build` concatena las
piezas y escribe `crates/luna/src/web/internal/ui.js`, que es el archivo que
sirve esta ruta. Después de regenerarlo hay que recompilar Luna: se incorpora
al binario con `include_str!`.

Desde `server_ui`:

```sh
bun run test       # framework y regresiones
bun run build      # regenera el bundle dentro de bevy_oxr
bun run humo       # Ajustes contra el host simulado
bun run verificar  # bundle sincronizado y rutas; requiere el servidor activo
```

Estas pruebas no sustituyen una comprobación visual en Luna.

Por eso el archivo dice *GENERADO — NO EDITAR A MANO* en la primera línea: un
arreglo hecho acá se pierde en el próximo build, y peor, deja las páginas
internas y las demos corriendo frameworks distintos que los dos andan. El
`verificar` de `server_ui` chequea justamente eso.

No está minificado a propósito. No viaja por red —Luna lo tiene en el binario
con `include_str!`— así que lo único que se ganaría son kilobytes que no cuestan,
y lo que se perdería son los comentarios.

> **Desde un documento remoto**: hoy funciona. La carga de `<script src>` no
> chequea origen (`load_text_resource`, `io.rs`), así que una página en
> `http://…` puede pedir `luna://internal/ui.js` y lo recibe. Pero eso se apoya
> en que **el CSP todavía no está cableado** — `csp_allows_script` existe en
> `csp.rs` y no tiene un solo llamador fuera de su test. No conviene tratar ese detalle como un contrato de permisos futuro.
> Podés servir una copia versionada del bundle desde tu origen; las APIs nativas
> que usa (`TextLayout`, `PathGeometry`, `MeshResource`) siguen dependiendo de la
> versión del motor. Cargar la biblioteca no concede los privilegios de Ajustes.

## El caso de referencia

`luna://settings` es la primera página interna hecha así, y muestra los dos
patrones que hacen falta cuando la interfaz vive adentro del motor:

**El buzón.** El host publica el estado del MCP inyectando JavaScript que hace
`getElementById('mcp_status').setAttribute('value', …)`, con los ids escritos en
`agent.rs`. El framework dibuja su texto **adentro de una malla** y sus nodos son
de pileta, sin ids estables: no hay nada que el host pueda encontrar. Así que el
documento declara esos nodos **invisibles**, como buzón, y la aplicación los lee.

```xml
<group visible="false">
  <text id="mcp_status" value="MCP local" size="0.01"/>
</group>
```

Vale para cualquier estado que el host publique así. Si se borran, deja de
llegar y **no falla nada**: simplemente no se entera nunca.

**El slot embebido.** La app dibuja su fondo; el shell conserva la barra de
ventana y el ancla, sin una placa fija detrás. Ajustes recibe el evento `slot`
de `dimention.embedded` y, cuando `coordinateSpace` es `window-local`, calcula
una escala uniforme. Llama a `app.redimensionar(slot.size.x / escala,
slot.size.y / escala)` y aplica esa escala al grupo contenedor: así llena el
slot sin deformar letras ni controles, incluso si cambia la relación de aspecto.
La traslación del grupo compensa el centro y la distancia de esa aplicación;
no copies esos números si usás otros `y`/`z`.

Los toques usan `localX`/`localY` calculados con la inversa de la transformación
del blanco. El origen de las mallas de texto sigue su profundidad real para
ordenarlas delante del fondo transparente sin mover sus vértices en el mundo.
MCP y Navegación conservan la raíz y alternan sus contenidos, evitando recrear
el panel durante cada cambio de sección.

---

## Fuentes de verdad

| Tema | Archivo |
|---|---|
| El framework y su arquitectura | `server_ui/README.md` |
| Malla retenida, texto, curvas, límites | `server_ui/RENDERING.md` |
| El build | `server_ui/src/construir.ts` |
| La página de ajustes | `crates/luna/src/web/settings.hsml` + `web/internal/settings_ui.js` |
| Texto medido, glifos y curvas del motor | [interfaz.md](interfaz.md) |

---

Volver al [índice de guías](index.md).
