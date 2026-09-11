// settings_ui.js — la pagina de ajustes, con el framework de interfaces.
//
// Dos paneles, como cualquier ventana de preferencias: una lista de secciones a
// la izquierda y el detalle a la derecha. Es el layout que hace falta en cuanto
// hay mas de seis opciones, y el que la version anterior no podia tener porque
// todo estaba clavado en coordenadas absolutas.
//
// Lo que se guarda, se guarda al tocarlo. No hay boton de aplicar: cada control
// llama al host en el mismo cuadro y el host escribe la configuracion en disco
// ahi mismo (`settings::apply_root_patch`). Un boton de aplicar sobre una
// pantalla de interruptores es una pantalla intermedia para nada.
(function arranque() {
  // Los <script src> llegan asincronicos y el motor NO garantiza el orden.
  // Esperar a que el framework exista es mas barato que ordenarlos, y es el
  // mismo patron que usa el shell con shell_ui.js / shell_draw.js.
  if (!globalThis.UI) return void requestAnimationFrame(arranque);

  const C = globalThis.UI_CFG.C;
  const raiz = hiperspace.dimention;

  // El estado lo publica el host por `dimention.settings`, que inyecta
  // `settings_api.js`. Si no esta, esta pagina se abrio fuera de Luna o el
  // motor es viejo: se dibuja igual, con los valores de reserva, y se dice.
  const ajustes = raiz.settings;
  if (!ajustes) {
    console.error("[settings] falta dimention.settings: la pagina se dibuja " +
                  "pero no va a mostrar ni guardar nada");
  }

  // ── El modelo ─────────────────────────────────────────────────────────────
  //
  // Dos secciones, y el corte no es por tema sino por a quien le sirve:
  // **General** es lo que cualquiera puede querer tocar; **Desarrollador** es lo
  // que solo tiene sentido si estas trabajando sobre el motor. El MCP es
  // exactamente eso: un servidor local para que un agente lea los registros y
  // abra paginas.
  const SECCIONES = [
    { id: "general", titulo: "General", icono: C.azul },
    { id: "personalizar", titulo: "Personalizar", icono: C.violeta },
    { id: "dev", titulo: "Desarrollador", icono: C.naranja },
  ];

  /** Las URLs que se ofrecen como pagina de inicio.
   *
   *  Es una lista y no un campo de texto porque **el framework no tiene entrada
   *  de texto**: no hay TextBox ni IME. La que este configurada se agrega sola,
   *  asi que una URL puesta a mano en el archivo de configuracion se ve y se
   *  puede volver a elegir, aunque no se pueda escribir una nueva desde aca. */
  const INICIOS = ["luna://home", "luna://demos", "luna://about"];
  const MODOS = ["Escritorio", "VR"];

  const datos = {
    secciones: SECCIONES.map(function (s, i) {
      return Object.assign({}, s, { fondo: i === 0 ? C.superficieAlta : "transparent" });
    }),
    seccion: "general",
    titulo: "General",

    // Del estado que publica el host: el MCP.
    estado: "sin datos",
    encendido: false,
    auto: false,
    autoTexto: "cargando…",

    // Y la configuracion raiz.
    inicioAuto: true,
    inicios: INICIOS.slice(),
    inicioElegido: 0,
    modos: MODOS.slice(),
    modoElegido: 0,
    controllers: ["Curvo (actual)", "Plano (escritorio)"],
    controllerElegido: 0,
    rutaConfig: "—",
    urlRaiz: "—",
    permisos: [],
  };

  // ── El marcado ────────────────────────────────────────────────────────────

  // La fila del sidebar es un Button y no un Border con un Grid adentro, aunque
  // el segundo se pareceria mas a una lista de verdad: un Border no recibe
  // toques —no es Interactivo— y el framework no tiene todavia un ListBox que
  // le ponga la seleccion a una plantilla.
  const FILA_SECCION = `
<Button Content="{Binding titulo}" Background="{Binding fondo}"
        Foreground="${C.texto}" FontSize="0.028" CornerRadius="0.012"
        Padding="0.014,0.011" Click="elegirSeccion"
        HorizontalAlignment="Stretch"/>
`;

  const PLANTILLA_PERMISO = `
<Grid ColumnDefinitions="*,Auto,Auto" ColumnSpacing="0.008" Margin="0.014,0.008">
  <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.002">
    <TextBlock Text="{Binding origin}" FontSize="0.023" Foreground="${C.texto}"/>
    <TextBlock Text="{Binding capability}" FontSize="0.018" Foreground="${C.textoTenue}"/>
  </StackPanel>
  <TextBlock Grid.Column="1" Text="{Binding etiqueta}" FontSize="0.02"
             Foreground="{Binding color}" VerticalAlignment="Center"/>
  <Button Grid.Column="2" Content="{Binding accion}" Background="${C.superficieAlta}"
          FontSize="0.02" CornerRadius="0.008" Click="tocarPermiso"
          VerticalAlignment="Center"/>
</Grid>
`;

  /** Un grupo de opciones: el rectangulo redondeado con las filas adentro y la
   *  nota al pie afuera, que es como se leen las preferencias en todos lados. */
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

  /** Una fila: rotulo a la izquierda, control a la derecha. Es la forma de casi
   *  todo lo que hay aca, y escribirla una vez evita que ocho copias se vayan
   *  separando de a un milimetro. */
  function fila(rotulo, detalle, control) {
    const izquierda = detalle
      ? `<StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.002">
           <TextBlock Text="${rotulo}" FontSize="0.028" Foreground="${C.texto}"/>
           <TextBlock Text="${detalle}" FontSize="0.019" Foreground="${C.textoTenue}"/>
         </StackPanel>`
      : `<TextBlock Grid.Column="0" Text="${rotulo}" FontSize="0.028"
                    Foreground="${C.texto}" VerticalAlignment="Center"/>`;
    return `
      <Grid ColumnDefinitions="*,Auto" Margin="0.014,0.012">
        ${izquierda}
        ${control}
      </Grid>`;
  }

  const RAYA = `<Separator Background="${C.borde}" Margin="0.014,0"/>`;

  // ── General ───────────────────────────────────────────────────────────────

  const PANEL_GENERAL =
    grupo(
      fila("Abrir el inicio al arrancar", null,
        `<ToggleSwitch Grid.Column="1" IsChecked="{Binding inicioAuto, Mode=TwoWay}"
                       Changed="cambiarInicioAuto" Accent="${C.verde}"
                       VerticalAlignment="Center"/>`) +
      RAYA +
      fila("Página de inicio", null,
        `<ComboBox Grid.Column="1" Items="{Binding inicios}"
                   SelectedIndex="{Binding inicioElegido, Mode=TwoWay}"
                   SelectionChanged="cambiarInicio" VerticalAlignment="Center"/>`) +
      RAYA +
      fila("Modo preferido", "se aplica al próximo arranque",
        `<ComboBox Grid.Column="1" Items="{Binding modos}"
                   SelectedIndex="{Binding modoElegido, Mode=TwoWay}"
                   SelectionChanged="cambiarModo" VerticalAlignment="Center"/>`),
      "Cada cambio se guarda solo, en el acto. La página de inicio se elige " +
      "de una lista porque todavía no hay entrada de texto: la que esté " +
      "configurada aparece igual, aunque no sea una de estas.") +
    `<StackPanel Orientation="Vertical" Spacing="0.008" Margin="0,0.008,0,0">
      <TextBlock Text="Permisos por sitio" FontSize="0.026" Foreground="${C.texto}"
                 Margin="0.014,0,0,0.002"/>
      <Border Background="${C.superficie}" CornerRadius="0.016" Padding="0.004">
        <StackPanel Orientation="Vertical" Spacing="0">
          <TextBlock Name="sinPermisos" Text="No hay decisiones guardadas."
                     FontSize="0.022" Foreground="${C.textoTenue}"
                     Margin="0.014,0.012"/>
          <ItemsControl ItemsSource="{Binding permisos}" ItemTemplate="permiso"
                        Spacing="0.002"/>
        </StackPanel>
      </Border>
      <TextBlock Text="Lo concedido se revoca; lo negado vuelve a preguntar la próxima vez, que no es lo mismo que permitirlo."
                 FontSize="0.019" Foreground="${C.textoTenue}"
                 TextWrapping="Wrap" Margin="0.014,0,0.014,0.012"/>
    </StackPanel>` +
    grupo(
      fila("Shell raíz", "{Binding urlRaiz}", "") +
      RAYA +
      fila("Archivo de configuración", "{Binding rutaConfig}", ""),
      "Dónde vive lo que se guarda. Sólo para mirar.");

  // ── Desarrollador ─────────────────────────────────────────────────────────

  const PANEL_PERSONALIZAR = grupo(
    fila("Controller", "el mismo en escritorio y VR",
      `<ComboBox Grid.Column="1" Items="{Binding controllers}"
                 SelectedIndex="{Binding controllerElegido, Mode=TwoWay}"
                 SelectionChanged="cambiarController" VerticalAlignment="Center"/>`),
    "Se aplica al elegirlo y se conserva al reiniciar. Usa el bot�n de men� para volver a abrirlo.");

  const PANEL_DEV =
    grupo(
      fila("Servidor MCP", null,
        `<ToggleSwitch Grid.Column="1" IsChecked="{Binding encendido, Mode=TwoWay}"
                       Changed="alternarMcp" Accent="${C.verde}"
                       ToolTip="Prende y apaga el servidor ahora mismo"/>`) +
      RAYA +
      fila("Estado", null,
        `<TextBlock Grid.Column="1" Text="{Binding estado}" FontSize="0.024"
                    Foreground="${C.textoTenue}" VerticalAlignment="Center"/>`) +
      RAYA +
      fila("Iniciar con Luna", "{Binding autoTexto}",
        `<ToggleSwitch Grid.Column="1" IsChecked="{Binding auto, Mode=TwoWay}"
                       Changed="alternarArranque" Accent="${C.verde}"
                       VerticalAlignment="Center"
                       ToolTip="Se guarda en disco al tocarlo"/>`),
      "El servidor MCP deja que un agente lea los registros y abra páginas. " +
      "El interruptor de arriba vale para esta sesión; el de abajo se guarda " +
      "solo, apenas se toca.") +
    grupo(
      fila("Estadísticas de caché", "luna://cache-stats",
        `<Button Grid.Column="1" Content="Ir" Background="${C.violeta}" FontSize="0.024"
                 CornerRadius="0.009" Click="ir_cache" VerticalAlignment="Center"/>`),
      "Desde un panel embebido, navegar reemplaza el contenido del panel, no el " +
      "mundo.");

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
        <StackPanel Spacing="0.004">
          <StackPanel Name="detalleGeneral">${PANEL_GENERAL}</StackPanel>
          <StackPanel Name="detallePersonalizar" Visibility="Collapsed">${PANEL_PERSONALIZAR}</StackPanel>
          <StackPanel Name="detalleDev" Visibility="Collapsed">${PANEL_DEV}</StackPanel>
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
    plantillas: { seccion: FILA_SECCION, permiso: PLANTILLA_PERMISO },
    manejadores: {
      // ── Desarrollador ──
      alternarMcp: function () {
        // El toggle ya escribió `datos.encendido` por el enlace TwoWay.
        if (ajustes) ajustes.setMcp(!!datos.encendido);
      },
      alternarArranque: function () {
        if (ajustes) ajustes.setMcpAutoStart(!!datos.auto);
      },
      ir_cache: function () { location.href = "luna://cache-stats"; },

      // ── General ──
      // Los tres mandan un parche con **sólo** lo que cambió. El host aplica
      // campo por campo, así que esta página no puede pisar una preferencia que
      // todavía no sabe que existe.
      cambiarInicioAuto: function () {
        if (ajustes) ajustes.set({ autoLoadHome: !!datos.inicioAuto });
      },
      cambiarInicio: function () {
        const url = datos.inicios[datos.inicioElegido];
        if (url && ajustes) ajustes.set({ homeUrl: url });
      },
      cambiarController: function () {
        if (ajustes) ajustes.set({ controllerStyle: datos.controllerElegido === 1 ? "flat" : "curved" });
      },
      cambiarModo: function () {
        if (ajustes) ajustes.set({ renderMode: datos.modoElegido === 1 ? "vr" : "desktop" });
      },
      tocarPermiso: function (control) {
        const item = control.itemDeLista();
        if (!item) return;
        // Conceder se revoca; negar vuelve a preguntar. No hay un botón para
        // conceder desde acá a propósito: un permiso se concede contestándole
        // al sitio que lo pide, no repartiéndolo de antemano en una lista.
        if (!ajustes) return;
        ajustes.set({
          permission: {
            origin: item.origin,
            key: item.key,
            decision: item.decision === "allow" ? "deny" : "ask",
          },
        });
      },

      elegirSeccion: function (control) {
        const item = control.itemDeLista();
        if (item) elegir(item.id);
      },
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
    const general = app.buscar("detalleGeneral");
    general.visible = general.opaco = id === "general";
    const personalizar = app.buscar("detallePersonalizar");
    personalizar.visible = personalizar.opaco = id === "personalizar";
    const dev = app.buscar("detalleDev");
    dev.visible = dev.opaco = id === "dev";
    app.invalidar();
  }

  // ── Lo que publica el host ────────────────────────────────────────────────
  // Un solo evento con todo. No hay sondeo: `settings_api.js` avisa cuando la
  // publicacion cambia de verdad, y si ya habia una la entrega al suscribirse.

  function aplicar(cfg) {
    if (!cfg || typeof cfg !== "object") return;

    const mcp = cfg.mcp || {};
    // `connection_label()` (agent.rs) devuelve exactamente una de tres cadenas.
    // Se comparan como lo que son —constantes del host— y no con una expresión
    // regular que adivine: si el día de mañana cambian, esto tiene que dejar de
    // reconocerlas y mostrar el texto crudo, no acertar por casualidad.
    const etiqueta = String(mcp.label || "");
    datos.estado = etiqueta === "MCP disabled" ? "apagado"
                 : etiqueta === "MCP connected (localhost)" ? "conectado"
                 : etiqueta === "MCP waiting for local adapter" ? "esperando al adaptador"
                 : (etiqueta || "sin datos");
    datos.encendido = etiqueta !== "" && etiqueta !== "MCP disabled";
    datos.auto = !!mcp.autoStart;
    datos.autoTexto = datos.auto ? "se abre solo al arrancar" : "hay que prenderlo a mano";

    datos.inicioAuto = cfg.autoLoadHome !== false;
    datos.rutaConfig = cfg.configPath || "—";
    datos.urlRaiz = cfg.rootUrl || "—";
    datos.controllerElegido = cfg.controllerStyle === "flat" ? 1 : 0;
    datos.modoElegido = cfg.renderMode === "vr" ? 1 : 0;

    // La URL configurada entra en la lista si no estaba. Es lo que hace que una
    // puesta a mano en el archivo se vea en vez de desaparecer detrás de la
    // primera opción, que sería mentir sobre lo que está pasando.
    const url = String(cfg.homeUrl || "");
    const inicios = INICIOS.slice();
    if (url && inicios.indexOf(url) < 0) inicios.unshift(url);
    datos.inicios = inicios;
    datos.inicioElegido = Math.max(0, inicios.indexOf(url));

    datos.permisos = (Array.isArray(cfg.permissions) ? cfg.permissions : []).map(function (p) {
      const permitido = p.decision === "allow";
      return {
        origin: String(p.origin || ""),
        capability: String(p.capability || ""),
        key: String(p.key || ""),
        decision: p.decision,
        etiqueta: permitido ? "concedido" : "negado",
        color: permitido ? C.verde : C.rojo,
        accion: permitido ? "Revocar" : "Preguntar",
      };
    });
    const vacio = app.buscar("sinPermisos");
    if (vacio) vacio.visible = vacio.opaco = datos.permisos.length === 0;

    app.invalidar();
  }

  if (ajustes) ajustes.on(aplicar);

  app.correr();

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
      if (!(Number.isFinite(slot.size.x) && slot.size.x > 0 &&
            Number.isFinite(slot.size.y) && slot.size.y > 0)) return;
      // El framework mide en metros y no se reescala solo: se ajusta el grupo
      // que lo contiene. La escala es uniforme para conservar las proporciones
      // del texto, y el layout se estira para llenar el slot.
      const escala = Math.min(slot.size.x / 1.6, slot.size.y / 0.95);
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
