# Interfaz declarativa: `luna://internal/ui.js`

Un panel con seis opciones se puede escribir a mano —seis `<box>`, seis `<text>`,
todos en coordenadas absolutas— y funciona. Con dieciséis deja de funcionar:
mover una fila obliga a recalcular las de abajo, y no hay forma de que algo
crezca con su contenido.

El motor trae un framework de interfaces para eso, en la línea de WPF: árbol
declarativo, layout en dos pasadas —medir y acomodar—, paneles que reparten el
espacio, y enlaces a datos. Se dibuja **en una malla**, no en nodos: una interfaz
de doscientos rectángulos es un nodo, no doscientos.

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
            <TextBlock Text="Toques: {Binding n}" FontSize="0.04" Foreground="#FFFFFF"/>
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

Los dos nodos —`ui_malla` y `ui_nodos`— son dónde vuelca su trabajo: la malla
para todo lo que se dibuja, el grupo para lo que no puede ser malla (las
imágenes y los blancos táctiles). `material-unlit` no es opcional: sin él el
material estándar ilumina la interfaz y los colores dependen de dónde esté
parado el visitante.

## Lo que hay

| | |
|---|---|
| **paneles** | `StackPanel`, `Grid`, `DockPanel`, `WrapPanel`, `Canvas` |
| **decoradores** | `Border`, `RenderPanel`, `ScrollViewer`, `Separator` |
| **texto e imagen** | `TextBlock` (con `TextWrapping="Wrap"`), `Image` |
| **controles** | `Button`, `CheckBox`, `RadioButton`, `ToggleSwitch`, `Slider`, `ProgressBar` |
| **capa de arriba** | `ComboBox`, `ToolTip`, y `superponer()` para diálogos modales |
| **vectores** | `Path` (`M L Q C Z`), `Ellipse` |
| **listas** | `ItemsControl` con plantilla, reciclado y virtualización |
| **navegación** | `TabControl` / `TabItem` |
| **enlaces** | `{Binding ruta, Mode=TwoWay, Format=%v %}` |
| **estilos** | juego de atributos por tipo de elemento |

Todas las medidas son **metros**. `Depth` da espesor y `Elevation` despega un
control del plano de su padre.

## Lo que no hay

- **No hay entrada de texto.** Ni `TextBox` ni IME. Una URL se elige de un
  `ComboBox`, no se escribe.
- **No hay `ListBox` con selección.** `ItemsControl` dibuja la lista; la
  selección la lleva la aplicación. Un control adentro de la plantilla sabe de
  qué fila es con `itemDeLista()`.
- **`Aplicacion` no se redimensiona.** `ancho` y `alto` se fijan al construirla.
  Para un panel embebido que puede cambiar de tamaño, hoy se escala el `<group>`
  que lo contiene.
- No hay sombras, ni recortes que no sean rectangulares, ni foco de teclado.

---

## Cómo llega acá

El framework **no se escribe en este repo**: vive en `server_ui`, con sus demos,
su banco de pruebas y sus 68 pruebas sin motor. `bun run build` concatena las
piezas y escribe `crates/luna/src/web/internal/ui.js`, que es el archivo que
sirve esta ruta.

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
> `csp.rs` y no tiene un solo llamador fuera de su test. El día que se conecte,
> esa carga es la primera candidata a caerse. Para algo que tiene que seguir
> andando, copiá el archivo a tu propio origen.

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

**El slot embebido.** Con `dimention.embedded`, el panel escala el `<group>` que
contiene la interfaz para caber en el slot. Los toques siguen andando porque el
evento trae `localX`/`localY` calculados con la inversa de la transformada del
nodo — sin eso, escalar rompería todos los impactos.

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
