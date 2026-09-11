// settings_personalizar.js — Ajustes · Personalizar.
//
// Por ahora, el controller: el curvo o el plano, el mismo en escritorio y VR.
(function arranque() {
  if (!globalThis.AjustesKit || !AjustesKit.listo()) return void requestAnimationFrame(arranque);
  const K = AjustesKit;
  const ajustes = hiperspace.dimention.settings;

  const datos = { controllers: ["Curvo", "Plano (escritorio)"], controllerElegido: 0 };

  const app = K.montar({
    datos: datos,
    contenido: K.grupo(
      K.fila("Controller", "el mismo en escritorio y VR",
        `<ComboBox Grid.Column="1" Items="{Binding controllers}"
                   SelectedIndex="{Binding controllerElegido, Mode=TwoWay}"
                   SelectionChanged="cambiarController" VerticalAlignment="Center"/>`),
      "Se aplica al elegirlo y se conserva al reiniciar. Usá el botón de menú para volver a abrirlo."),
    manejadores: {
      cambiarController: function () {
        if (ajustes) ajustes.set({ controllerStyle: datos.controllerElegido === 1 ? "flat" : "curved" });
      },
    },
  });

  if (ajustes) ajustes.on(function (cfg) {
    if (!cfg) return;
    datos.controllerElegido = cfg.controllerStyle === "flat" ? 1 : 0;
    app.invalidar();
  });
})();
