// settings_dev.js — Ajustes · Desarrollador.
//
// Lo que solo tiene sentido si estas trabajando sobre el motor: el servidor MCP
// y las estadisticas de cache.
(function arranque() {
  if (!globalThis.AjustesKit || !AjustesKit.listo()) return void requestAnimationFrame(arranque);
  const K = AjustesKit, C = K.C();
  const ajustes = hiperspace.dimention.settings;

  const datos = { estado: "sin datos", encendido: false, auto: false, autoTexto: "cargando…" };

  const app = K.montar({
    datos: datos,
    contenido:
      K.grupo(
        K.fila("Servidor MCP", null,
          `<ToggleSwitch Grid.Column="1" IsChecked="{Binding encendido, Mode=TwoWay}"
                         Changed="alternarMcp" Accent="${C.verde}"
                         ToolTip="Prende y apaga el servidor ahora mismo"/>`) +
        K.raya() +
        K.fila("Estado", null,
          `<TextBlock Grid.Column="1" Text="{Binding estado}" FontSize="0.024"
                      Foreground="${C.textoTenue}" VerticalAlignment="Center"/>`) +
        K.raya() +
        K.fila("Iniciar con Luna", "{Binding autoTexto}",
          `<ToggleSwitch Grid.Column="1" IsChecked="{Binding auto, Mode=TwoWay}"
                         Changed="alternarArranque" Accent="${C.verde}" VerticalAlignment="Center"
                         ToolTip="Se guarda en disco al tocarlo"/>`),
        "El servidor MCP deja que un agente lea los registros y abra páginas. El " +
        "interruptor de arriba vale para esta sesión; el de abajo se guarda solo.") +
      K.grupo(
        K.fila("Estadísticas de caché", "luna://cache-stats",
          `<Button Grid.Column="1" Content="Abrir" Background="${C.violeta}" FontSize="0.024"
                   CornerRadius="0.009" Click="irCache" VerticalAlignment="Center"/>`),
        null),
    manejadores: {
      alternarMcp: function () { if (ajustes) ajustes.setMcp(!!datos.encendido); },
      alternarArranque: function () { if (ajustes) ajustes.setMcpAutoStart(!!datos.auto); },
      irCache: function () { K.navegar("luna://cache-stats"); },
    },
  });

  if (ajustes) ajustes.on(function (cfg) {
    if (!cfg) return;
    const mcp = cfg.mcp || {};
    // `connection_label()` (agent.rs) devuelve exactamente una de tres cadenas.
    // Se comparan como constantes del host: si cambian, esto tiene que mostrar
    // el texto crudo y no acertar por casualidad.
    const etiqueta = String(mcp.label || "");
    datos.estado = etiqueta === "MCP disabled" ? "apagado"
                 : etiqueta === "MCP connected (localhost)" ? "conectado"
                 : etiqueta === "MCP waiting for local adapter" ? "esperando al adaptador"
                 : (etiqueta || "sin datos");
    datos.encendido = etiqueta !== "" && etiqueta !== "MCP disabled";
    datos.auto = !!mcp.autoStart;
    datos.autoTexto = datos.auto ? "se abre solo al arrancar" : "hay que prenderlo a mano";
    app.invalidar();
  });
})();
