// settings_general.js — Ajustes · General.
//
// Lo que cualquiera puede querer tocar: la pagina de inicio, el modo preferido
// y los permisos que se le dieron a cada sitio. Se guarda al tocarlo: cada
// control manda un parche con **solo** lo que cambio, y el host lo escribe en
// disco en el acto (`settings::apply_root_patch`).
(function arranque() {
  if (!globalThis.AjustesKit || !AjustesKit.listo()) return void requestAnimationFrame(arranque);
  const K = AjustesKit, C = K.C();
  const ajustes = hiperspace.dimention.settings;


  const datos = {
    inicioAuto: true,
    inicioUrl: '', errorInicio: '',
    modos: ["Escritorio", "VR"], modoElegido: 0,
    rutaConfig: "—", urlRaiz: "—",
    permisos: [],
  };

  const PLANTILLA_PERMISO = `
<Grid ColumnDefinitions="*,Auto,Auto" ColumnSpacing="0.008" Margin="0.014,0.008">
  <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.002">
    <TextBlock Text="{Binding origin}" FontSize="0.023" Foreground="${C.texto}"/>
    <TextBlock Text="{Binding capability}" FontSize="0.018" Foreground="${C.textoTenue}"/>
  </StackPanel>
  <TextBlock Grid.Column="1" Text="{Binding etiqueta}" FontSize="0.02"
             Foreground="{Binding color}" VerticalAlignment="Center"/>
  <Button Grid.Column="2" Content="{Binding accion}" Background="${C.superficieAlta}"
          FontSize="0.02" CornerRadius="0.008" Click="tocarPermiso" VerticalAlignment="Center"/>
</Grid>`;

  const contenido =
    K.grupo(
      K.fila("Abrir el inicio al arrancar", null,
        `<ToggleSwitch Grid.Column="1" IsChecked="{Binding inicioAuto, Mode=TwoWay}"
                       Changed="cambiarInicioAuto" Accent="${C.verde}" VerticalAlignment="Center"/>`) +
      K.raya() +
      K.fila("Página de inicio", null,
        `<TextBox Name="inicioUrl" Grid.Column="1" Text="{Binding inicioUrl, Mode=TwoWay}"
                  Width="0.59" MaxLength="2048" Placeholder="https://…"
                  Changed="cambiarInicio" VerticalAlignment="Center"/>`) +
      `<TextBlock Text="{Binding errorInicio}" FontSize="0.021" Foreground="${C.rojo}" TextWrapping="Wrap"/>` +
      K.raya() +
      K.fila("Modo preferido", "se aplica al próximo arranque",
        `<ComboBox Grid.Column="1" Items="{Binding modos}"
                   SelectedIndex="{Binding modoElegido, Mode=TwoWay}"
                   SelectionChanged="cambiarModo" VerticalAlignment="Center"/>`),
      "La dirección se guarda con Enter o al salir del campo. En VR, tocá el campo para abrir el teclado.") +
    K.encabezado("Permisos por sitio") +
    `<StackPanel Orientation="Vertical" Spacing="0.008">
      <Border Background="${C.superficie}" CornerRadius="0.016" Padding="0.004">
        <StackPanel Orientation="Vertical" Spacing="0">
          <TextBlock Name="sinPermisos" Text="No hay decisiones guardadas."
                     FontSize="0.022" Foreground="${C.textoTenue}" Margin="0.014,0.012"/>
          <ItemsControl ItemsSource="{Binding permisos}" ItemTemplate="permiso" Spacing="0.002"/>
        </StackPanel>
      </Border>
      <TextBlock Text="Lo concedido se revoca; lo negado vuelve a preguntar la próxima vez, que no es lo mismo que permitirlo."
                 FontSize="0.019" Foreground="${C.textoTenue}" TextWrapping="Wrap"
                 Margin="0.014,0,0.014,0.012"/>
    </StackPanel>`;

  const app = K.montar({
    datos: datos,
    plantillas: { permiso: PLANTILLA_PERMISO },
    contenido: contenido,
    manejadores: {
      cambiarInicioAuto: function () { if (ajustes) ajustes.set({ autoLoadHome: !!datos.inicioAuto }); },
      cambiarInicio: function () {
        const url = String(datos.inicioUrl || '').trim();
        try {
          const parsed = new URL(url);
          if (!['luna:', 'http:', 'https:'].includes(parsed.protocol)) throw new Error('scheme');
          datos.errorInicio = ''; datos.inicioUrl = url;
          if (ajustes) ajustes.set({ homeUrl: url });
        } catch (_) { datos.errorInicio = 'Usá una dirección luna://, http:// o https:// válida.'; }
        app.invalidar();
      },
      cambiarModo: function () {
        if (ajustes) ajustes.set({ renderMode: datos.modoElegido === 1 ? "vr" : "desktop" });
      },
      tocarPermiso: function (control) {
        const item = control.itemDeLista();
        // Conceder se revoca; negar vuelve a preguntar. No hay boton para
        // conceder desde aca a proposito: un permiso se concede contestandole
        // al sitio que lo pide, no repartiendolo de antemano en una lista.
        if (!item || !ajustes) return;
        ajustes.set({ permission: { origin: item.origin, key: item.key,
                                    decision: item.decision === "allow" ? "deny" : "ask" } });
      },
    },
  });

  function aplicar(cfg) {
    if (!cfg || typeof cfg !== "object") return;
    datos.inicioAuto = cfg.autoLoadHome !== false;
    datos.modoElegido = cfg.renderMode === "vr" ? 1 : 0;
    const url = String(cfg.homeUrl || "");
    if (!app.buscar('inicioUrl')?._focused) datos.inicioUrl = url;
    datos.permisos = (Array.isArray(cfg.permissions) ? cfg.permissions : []).map(function (p) {
      const permitido = p.decision === "allow";
      return {
        origin: String(p.origin || ""), capability: String(p.capability || ""),
        key: String(p.key || ""), decision: p.decision,
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
  else console.error("[ajustes] falta dimention.settings: la sección se dibuja pero no guarda nada");
})();
