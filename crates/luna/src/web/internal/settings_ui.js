// settings_ui.js — la pagina de ajustes, con el framework de interfaces.
//
// Dos paneles, como cualquier ventana de preferencias: una lista de secciones a
// la izquierda y el detalle a la derecha. Es el layout que hace falta en cuanto
// hay mas de seis opciones, y el que la version anterior no podia tener porque
// todo estaba clavado en coordenadas absolutas.
//
// Lo que se guarda, se guarda al tocarlo. No hay boton de aplicar: los dos
// interruptores llaman al host en el mismo cuadro, y el de arranque escribe la
// configuracion en disco ahi mismo (`RootConfig::save`, js.rs). Un boton de
// aplicar sobre dos toggles es una pantalla intermedia para nada.
(function arranque() {
  // Los <script src> llegan asincronicos y el motor NO garantiza el orden.
  // Esperar a que el framework exista es mas barato que ordenarlos, y es el
  // mismo patron que usa el shell con shell_ui.js / shell_draw.js.
  if (!globalThis.UI) return void requestAnimationFrame(arranque);

  const C = globalThis.UI_CFG.C;
  const raiz = hiperspace.dimention;

  // ── El buzon del host ─────────────────────────────────────────────────────
  // El estado del MCP no llega por una API: el host inyecta JavaScript que
  // escribe atributos en tres nodos de ids fijos. Se leen de ahi.
  const buzon = {
    estado: raiz.getElementById("mcp_status"),
    rotulo: raiz.getElementById("mcp_auto_start_label"),
    arranque: raiz.getElementById("mcp_auto_start"),
  };

  function leerBuzon() {
    const estado = buzon.estado ? buzon.estado.getAttribute("value") : null;
    const auto = buzon.arranque ? buzon.arranque.getAttribute("data-enabled") : null;
    return {
      estado: estado || "sin datos",
      // Hasta que el host publique el primero, `data-enabled` no existe: eso es
      // "todavia no se", no "apagado". Se distingue para no dibujar un
      // interruptor en una posicion que no es la de la configuracion real.
      auto: auto === "true" ? true : auto === "false" ? false : null,
    };
  }

  // ── El modelo ─────────────────────────────────────────────────────────────
  const SECCIONES = [
    { id: "mcp",  titulo: "MCP",        icono: C.verde },
    { id: "nav",  titulo: "Navegación", icono: C.azul },
  ];

  const datos = {
    secciones: SECCIONES.map(function (s, i) {
      return Object.assign({}, s, { fondo: i === 0 ? C.superficieAlta : "transparent" });
    }),
    seccion: "mcp",
    titulo: "MCP",
    // Los tres que vienen del buzon.
    estado: "sin datos",
    encendido: false,
    auto: false,
    autoTexto: "cargando…",
  };

  // ── El marcado ────────────────────────────────────────────────────────────

  // La fila es un Button y no un Border con un Grid adentro, aunque el segundo
  // se pareceria mas a la captura: un Border no recibe toques —no es
  // Interactivo— y el framework no tiene todavia un ListBox que le ponga la
  // seleccion a una plantilla. Con Button se pierde el punto de color al
  // costado y se gana que la fila se pueda tocar y tenga hover.
  const FILA_SECCION = `
<Button Content="{Binding titulo}" Background="{Binding fondo}"
        Foreground="${C.texto}" FontSize="0.028" CornerRadius="0.012"
        Padding="0.014,0.011" Click="elegirSeccion"
        HorizontalAlignment="Stretch"/>
`;

  // Un grupo de opciones: el rectangulo redondeado con las filas adentro y la
  // nota al pie afuera, que es como se leen las preferencias en cualquier lado.
  function grupo(filas, nota) {
    return `
<StackPanel Orientation="Vertical" Spacing="0.008">
  <Border Background="${C.superficie}" CornerRadius="0.016" Padding="0.004">
    <StackPanel Orientation="Vertical" Spacing="0">
      ${filas}
    </StackPanel>
  </Border>
  <TextBlock Text="${nota}" FontSize="0.019" Foreground="${C.textoTenue}"
             TextWrapping="Wrap" Margin="0.014,0,0.014,0.012"/>
</StackPanel>`;
  }

  const PANEL_MCP = grupo(`
      <Grid ColumnDefinitions="*,Auto" Margin="0.014,0.012">
        <TextBlock Grid.Column="0" Text="Servidor MCP" FontSize="0.028"
                   Foreground="${C.texto}" VerticalAlignment="Center"/>
        <ToggleSwitch Grid.Column="1" IsChecked="{Binding encendido, Mode=TwoWay}"
                      Changed="alternarMcp" Accent="${C.verde}"
                      ToolTip="Prende y apaga el servidor ahora mismo"/>
      </Grid>
      <Separator Background="${C.borde}" Margin="0.014,0"/>
      <Grid ColumnDefinitions="*,Auto" Margin="0.014,0.012">
        <TextBlock Grid.Column="0" Text="Estado" FontSize="0.028"
                   Foreground="${C.texto}" VerticalAlignment="Center"/>
        <TextBlock Grid.Column="1" Text="{Binding estado}" FontSize="0.024"
                   Foreground="${C.textoTenue}" VerticalAlignment="Center"/>
      </Grid>
      <Separator Background="${C.borde}" Margin="0.014,0"/>
      <Grid ColumnDefinitions="*,Auto" Margin="0.014,0.012">
        <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.002">
          <TextBlock Text="Iniciar con Luna" FontSize="0.028" Foreground="${C.texto}"/>
          <TextBlock Text="{Binding autoTexto}" FontSize="0.019"
                     Foreground="${C.textoTenue}"/>
        </StackPanel>
        <ToggleSwitch Grid.Column="1" IsChecked="{Binding auto, Mode=TwoWay}"
                      Changed="alternarArranque" Accent="${C.verde}"
                      VerticalAlignment="Center"
                      ToolTip="Se guarda en disco al tocarlo"/>
      </Grid>`,
    "El servidor MCP deja que un agente lea los registros y abra páginas. " +
    "El interruptor de arriba vale para esta sesión; el de abajo se guarda " +
    "solo, apenas se toca.");

  function filaNavegacion(texto, detalle, destino, color) {
    return `
      <Grid ColumnDefinitions="*,Auto" Margin="0.014,0.012">
        <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.002">
          <TextBlock Text="${texto}" FontSize="0.028" Foreground="${C.texto}"/>
          <TextBlock Text="${detalle}" FontSize="0.019" Foreground="${C.textoTenue}"/>
        </StackPanel>
        <Button Grid.Column="1" Content="Ir" Background="${color}" FontSize="0.024"
                CornerRadius="0.009" Click="ir_${destino}" VerticalAlignment="Center"/>
      </Grid>`;
  }

  const PANEL_NAV = grupo(
    filaNavegacion("Página de inicio", "luna://home", "home", C.azul) +
    `<Separator Background="${C.borde}" Margin="0.014,0"/>` +
    filaNavegacion("Estadísticas de caché", "luna://cache-stats", "cache", C.violeta),
    "Estas dos abren en este mismo espacio. Desde un panel embebido, la " +
    "navegación reemplaza el contenido del panel, no el mundo.");

  function marcado() {
    return `
<Border Background="${C.fondo}"
        CornerRadius="0.024" Padding="0.018">
  <Grid ColumnDefinitions="0.30,*" ColumnSpacing="0.018">

    <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.01">
      <TextBlock Text="Ajustes" FontSize="0.034" Foreground="${C.texto}"
                 Margin="0.012,0.004,0,0.008"/>
      <ItemsControl ItemsSource="{Binding secciones}" ItemTemplate="seccion"
                    Spacing="0.004"/>
    </StackPanel>

    <DockPanel Grid.Column="1">
      <TextBlock DockPanel.Dock="Top" Text="{Binding titulo}" FontSize="0.032"
                 Foreground="${C.texto}" TextAlignment="Center"
                 Margin="0,0.004,0,0.014"/>
      <ScrollViewer>
        <StackPanel>
          <StackPanel Name="detalleMcp">${PANEL_MCP}</StackPanel>
          <StackPanel Name="detalleNav" Visibility="Collapsed">${PANEL_NAV}</StackPanel>
        </StackPanel>
      </ScrollViewer>
    </DockPanel>

  </Grid>
</Border>`;
  }

  // ── La aplicación ─────────────────────────────────────────────────────────

  const app = new UI.Aplicacion({
    ancho: 1.5, alto: 0.85, y: 1.55,
    datos: datos,
    plantillas: { seccion: FILA_SECCION },
    manejadores: {
      alternarMcp: function (control) {
        // El toggle ya escribió `datos.encendido` por el enlace TwoWay.
        raiz.setMcpEnabled(!!datos.encendido);
        void control;
      },
      alternarArranque: function () {
        // Esto sí persiste: el host escribe la configuración en disco al
        // recibirlo. El buzón va a confirmarlo en el próximo medio segundo.
        raiz.setMcpAutoStart(!!datos.auto);
      },
      elegirSeccion: function (control) {
        const item = control.itemDeLista();
        if (item) elegir(item.id);
      },
      ir_home: function () { location.href = "luna://home"; },
      ir_cache: function () { location.href = "luna://cache-stats"; },
    },
  });

  app.cargar(marcado());

  // Conservar el árbol y los recursos del panel al cambiar de sección.
  // Recargar toda la raíz destruía las mallas antes de preparar sus reemplazos.
  function elegir(id) {
    if (datos.seccion === id) return;
    datos.seccion = id;
    datos.titulo = (SECCIONES.find(function (s) { return s.id === id; }) || {}).titulo || "";
    datos.secciones = SECCIONES.map(function (s) {
      return Object.assign({}, s, {
        fondo: s.id === id ? C.superficieAlta : "transparent",
      });
    });
    app.buscar("detalleMcp").visible = id === "mcp";
    const nav = app.buscar("detalleNav");
    nav.visible = nav.opaco = id === "nav";
    app.invalidar();
  }

  // ── El buzón, una vez por cuadro ──────────────────────────────────────────
  // El host publica cada medio segundo. Leer tres atributos por cuadro es más
  // barato que cualquier forma de suscripción que se pudiera inventar acá, y
  // sólo se invalida cuando algo cambió de verdad.
  let ultimo = "";

  function latir() {
    requestAnimationFrame(latir);
    const b = leerBuzon();
    const firma = b.estado + "|" + b.auto;
    if (firma === ultimo) return;
    ultimo = firma;

    // `connection_label()` (agent.rs) devuelve exactamente una de tres cadenas.
    // Se comparan como lo que son —constantes del host— y no con una expresión
    // regular que adivine: si el día de mañana cambian, esto tiene que dejar de
    // reconocerlas y mostrar el texto crudo, no acertar por casualidad.
    datos.estado = b.estado === "MCP disabled" ? "apagado"
                 : b.estado === "MCP connected (localhost)" ? "conectado"
                 : b.estado === "MCP waiting for local adapter" ? "esperando al adaptador"
                 : b.estado;
    datos.encendido = b.estado !== "MCP disabled";
    if (b.auto !== null) {
      datos.auto = b.auto;
      datos.autoTexto = b.auto ? "se abre solo al arrancar" : "hay que prenderlo a mano";
    }
    app.invalidar();
  }

  app.correr();
  requestAnimationFrame(latir);

  // ── El panel embebido ─────────────────────────────────────────────────────
  // Un mismo documento sirve para el Home espacial y para el panel de la
  // ventana. Cuando hay slot, el contenido se escala para caber en él.
  (function panel(intento) {
    const embedded = dimention.embedded;
    if (!embedded) {
      if (intento < 50) setTimeout(function () { panel(intento + 1); }, 5);
      return;
    }
    embedded.on("slot", function (slot) {
      const contenido = raiz.getElementById("settings_content");
      if (!contenido || slot.coordinateSpace !== "window-local") return;
      // El framework mide en metros y no se reescala solo: se ajusta el grupo
      // que lo contiene, que es lo mismo que hacía la versión anterior.
      if (!(Number.isFinite(slot.size.x) && slot.size.x > 0 && Number.isFinite(slot.size.y) && slot.size.y > 0)) return;
      const escala = Math.min(slot.size.x / 1.6, slot.size.y / 0.95);
      // El fondo ocupa todo el slot, incluso si cambia su relación de aspecto.
      // Escala uniforme para conservar las proporciones del texto y controles.
      app.redimensionar(slot.size.x / escala, slot.size.y / escala);
      contenido.scale = { x: escala, y: escala, z: escala };
      contenido.position = { x: 0, y: -1.55 * escala, z: 1.6 * escala };
    });
    embedded.requestSlot({
      title: "Ajustes",
      minSize: { x: 1.2, y: 0.7, z: 0.1 },
      preferredSize: { x: 1.6, y: 0.95, z: 0.1 },
    });
  })(0);
})();
