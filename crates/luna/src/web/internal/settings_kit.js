// settings_kit.js — lo comun a las secciones de Ajustes.
//
// Cada seccion corre en su propio isolate (ver settings_section.hsml), asi que
// lo que antes eran funciones de un solo archivo —la fila, el grupo, la raya—
// tiene que estar en un script que las cuatro cargan. Ademas arma la
// aplicacion en el hueco que le da el que la aloja y la mueve cuando el hueco
// cambia.
(function (global) {
  "use strict";

  const K = {};

  /** Listo para usar: el framework, la configuracion y el canal de componente.
   *  Los <script src> llegan en cualquier orden, asi que cada seccion espera a
   *  esto antes de tocar nada. */
  K.listo = function () {
    return !!(global.UI && global.UI_CFG && typeof component !== "undefined");
  };

  K.C = function () { return global.UI_CFG.C; };

  /** Una fila: rotulo a la izquierda, control a la derecha. */
  K.fila = function (rotulo, detalle, control) {
    const C = K.C();
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
        ${control || ""}
      </Grid>`;
  };

  /** Un grupo de opciones: el rectangulo redondeado con las filas adentro y la
   *  nota al pie afuera. */
  K.grupo = function (filas, nota) {
    const C = K.C();
    return `
<StackPanel Orientation="Vertical" Spacing="0.008">
  <Border Background="${C.superficie}" CornerRadius="0.016" Padding="0.004">
    <StackPanel Orientation="Vertical" Spacing="0">
      ${filas}
    </StackPanel>
  </Border>
  ${nota ? `<TextBlock Text="${nota}" FontSize="0.019" Foreground="${C.textoTenue}"
             TextWrapping="Wrap" Margin="0.014,0,0.014,0.012"/>` : ""}
</StackPanel>`;
  };

  /** El titulo de un grupo, afuera y arriba del rectangulo. */
  K.encabezado = function (texto) {
    const C = K.C();
    return `<TextBlock Text="${texto}" FontSize="0.024" Foreground="${C.textoTenue}"
                       Margin="0.014,0.01,0,0.004"/>`;
  };

  K.raya = function () {
    return `<Separator Background="${K.C().borde}" Margin="0.014,0"/>`;
  };

  /** Pedirle al que aloja la seccion que navegue. Desde un include,
   *  location.href cargaria el destino adentro del hueco de la seccion. */
  K.navegar = function (url) {
    component.emit("navegar", { url: url })
      .catch(function (e) { console.error("[ajustes] navegar: " + String(e)); });
  };

  /** Armar la aplicacion de la seccion en el hueco que dicen las props.
   *
   *  `op.contenido` es el marcado de la seccion; se envuelve en un
   *  ScrollViewer porque el hueco es el que es y una seccion larga tiene que
   *  poder bajar. Con `props.fondo` —luna://about, que la muestra sola— lleva
   *  ademas el panel de fondo que en Ajustes pone el que la aloja. */
  K.montar = function (op) {
    const C = K.C();
    const p = component.props || {};
    const app = new UI.Aplicacion({
      x: num(p.x, 0.159), y: num(p.y, 1.52),
      // Un pelo delante del panel de Ajustes: en el mismo plano, el fondo y la
      // seccion se pelean el mismo pixel.
      z: num(p.z, -1.597),
      ancho: num(p.ancho, 1.1), alto: num(p.alto, 0.75),
      datos: op.datos, plantillas: op.plantillas || {}, manejadores: op.manejadores || {},
    });
    const cuerpo = `<ScrollViewer><StackPanel Spacing="0.006">${op.contenido}</StackPanel></ScrollViewer>`;
    app.cargar(p.fondo
      ? `<Border Background="${C.fondo}" CornerRadius="0.024" Padding="0.022">${cuerpo}</Border>`
      : cuerpo);
    app.correr();

    component.addEventListener("propschange", function (e) {
      ubicar(app, (e && e.detail && e.detail.props) || {});
    });
    return app;
  };

  function num(v, def) { return Number.isFinite(v) ? v : def; }

  /** Mover y redimensionar. `redimensionar` conserva el centro y no hace nada
   *  si el tamaño no cambio; aca cambia tambien el centro, asi que se le da un
   *  tamaño que seguro difiere para que recalcule el origen. */
  function ubicar(app, p) {
    if (!Number.isFinite(p.ancho) || !Number.isFinite(p.alto)) return;
    app.x = num(p.x, app.x);
    app.y = num(p.y, app.y);
    app.ancho = -1;
    app.redimensionar(p.ancho, p.alto);
  }

  global.AjustesKit = K;
})(globalThis);
