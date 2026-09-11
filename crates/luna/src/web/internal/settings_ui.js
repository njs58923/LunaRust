// settings_ui.js — la ventana de Ajustes: la lista de secciones y el hueco.
//
// Dos paneles, como cualquier ventana de preferencias: las secciones a la
// izquierda y el detalle a la derecha. Este script dibuja **solo** la lista y
// el titulo. El detalle es un <include> de luna://settings/<seccion>, que corre
// en su propio isolate y se dibuja en el hueco que este le indica por props.
//
// Asi abrir Ajustes evalua la seccion que se mira y no la logica de todas: el
// include tiene una sola seccion montada a la vez y al elegir otra se le cambia
// el src. Lo que una seccion rompa no se lleva puestas a las otras.
//
// Las secciones guardan solas —el host autoriza por la URL de cada una, ver
// `agent::SETTINGS_SECTIONS`— y cuando necesitan ir a otra pagina lo piden con
// el evento `navegar`, porque desde un include location.href carga el destino
// adentro del hueco.
(function arranque() {
  // Los <script src> llegan asincronicos y el motor NO garantiza el orden.
  if (!globalThis.UI) return void requestAnimationFrame(arranque);

  const C = globalThis.UI_CFG.C;
  const raiz = hiperspace.dimention;

  const SECCIONES = [
    { id: "general", titulo: "General" },
    { id: "personalizar", titulo: "Personalizar" },
    { id: "dev", titulo: "Desarrollador" },
    { id: "info", titulo: "Información" },
  ];

  /** La geometria de la ventana, en metros. El hueco del detalle se calcula de
   *  aca, asi que el marcado usa las mismas medidas: si una cambia sin la otra,
   *  la seccion se dibuja corrida respecto de su titulo. */
  const G = { ancho: 1.5, alto: 0.85, x: 0, y: 1.55, z: -1.6,
              padding: 0.018, lista: 0.30, separacion: 0.018, titulo: 0.064 };

  const inicial = (function () {
    const q = new URLSearchParams(location.search || "").get("seccion");
    return SECCIONES.some(function (s) { return s.id === q; }) ? q : "general";
  })();

  const datos = {
    secciones: marcar(inicial),
    titulo: SECCIONES.find(function (s) { return s.id === inicial; }).titulo,
  };

  function marcar(id) {
    return SECCIONES.map(function (s) {
      return Object.assign({}, s, { fondo: s.id === id ? C.superficieAlta : "transparent" });
    });
  }

  // La fila del sidebar es un Button: un Border no recibe toques.
  const FILA_SECCION = `
<Button Content="{Binding titulo}" Background="{Binding fondo}"
        Foreground="${C.texto}" FontSize="0.028" CornerRadius="0.012"
        Padding="0.014,0.011" Click="elegirSeccion" HorizontalAlignment="Stretch"/>`;

  const app = new UI.Aplicacion({
    ancho: G.ancho, alto: G.alto, x: G.x, y: G.y, z: G.z,
    datos: datos,
    plantillas: { seccion: FILA_SECCION },
    manejadores: {
      elegirSeccion: function (control) {
        const item = control.itemDeLista();
        if (item) elegir(item.id);
      },
    },
  });

  app.cargar(`
<Border Background="${C.fondo}" CornerRadius="0.024" Padding="${G.padding}">
  <Grid ColumnDefinitions="${G.lista},*" ColumnSpacing="${G.separacion}">
    <StackPanel Grid.Column="0" Orientation="Vertical" Spacing="0.01">
      <TextBlock Text="Ajustes" FontSize="0.034" Foreground="${C.texto}" Margin="0.012,0.004,0,0.008"/>
      <ItemsControl ItemsSource="{Binding secciones}" ItemTemplate="seccion" Spacing="0.004"/>
    </StackPanel>
    <TextBlock Grid.Column="1" Text="{Binding titulo}" FontSize="0.032" Foreground="${C.texto}"
               TextAlignment="Center" VerticalAlignment="Top" Margin="0,0.004,0,0"/>
  </Grid>
</Border>`);
  app.correr();

  // ── El hueco y el include ─────────────────────────────────────────────────

  const include = raiz.getElementById("seccion");
  let actual = null;

  /** El rectangulo del detalle, en las coordenadas del grupo de Ajustes. */
  function hueco() {
    const izquierda = G.x - G.ancho / 2 + G.padding + G.lista + G.separacion;
    const derecha = G.x + G.ancho / 2 - G.padding;
    const arriba = G.y + G.alto / 2 - G.padding - G.titulo;
    const abajo = G.y - G.alto / 2 + G.padding;
    return {
      x: (izquierda + derecha) / 2, y: (arriba + abajo) / 2,
      // Un pelo delante de la ventana: en el mismo plano, fondo y seccion se
      // pelean el mismo pixel.
      z: G.z + 0.003,
      ancho: derecha - izquierda, alto: arriba - abajo,
    };
  }

  function elegir(id) {
    if (actual === id || !include) return;
    actual = id;
    datos.secciones = marcar(id);
    datos.titulo = SECCIONES.find(function (s) { return s.id === id; }).titulo;
    app.invalidar();
    // Una sola seccion montada: cambiar el src desmonta la anterior y su
    // isolate. Conservar las cuatro vivas y esconder tres seria volver a tener
    // la logica de todas cargada, que es justo lo que se esta evitando.
    include.props = Object.assign({}, hueco());
    include.setAttribute("src", "luna://settings/" + id);
  }

  if (include) {
    include.addEventListener("component:navegar", function (e) {
      const url = e && e.detail && e.detail.url;
      // Solo paginas internas: una seccion no tiene por que mandar a nadie a
      // otro sitio, y si lo intentara es que algo anda mal.
      if (typeof url === "string" && url.indexOf("luna://") === 0) location.href = url;
    });
    elegir(inicial);
  } else {
    console.error("[settings] falta el <include id=\"seccion\">: el detalle no se va a ver");
  }

  // ── El panel embebido ─────────────────────────────────────────────────────
  // Un mismo documento sirve para el Home espacial y para el panel de la
  // ventana. Cuando hay slot, el contenido se escala para caber en el, y el
  // hueco de la seccion se recalcula con las medidas nuevas.
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
      const escala = Math.min(slot.size.x / 1.6, slot.size.y / 0.95);
      G.ancho = slot.size.x / escala;
      G.alto = slot.size.y / escala;
      app.redimensionar(G.ancho, G.alto);
      contenido.scale = { x: escala, y: escala, z: escala };
      contenido.position = { x: 0, y: -1.55 * escala, z: 1.6 * escala };
      if (include) include.props = Object.assign({}, hueco());
    });
    embedded.requestSlot({
      title: "Ajustes",
      minSize: { x: 1.2, y: 0.7, z: 0.1 },
      preferredSize: { x: 1.6, y: 0.95, z: 0.1 },
    });
  })(0);
})();
