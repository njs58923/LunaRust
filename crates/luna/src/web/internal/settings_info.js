// settings_info.js — Ajustes · Informacion.
//
// Lo que antes era luna://about: que es Luna, que version corre, con que esta
// hecho y que sabe hacer. La pagina vieja era una lista de <text> en
// coordenadas absolutas con la version escrita a mano —decia "0.1.0-alpha" con
// el workspace en 0.1.1—; aca la version, el perfil de compilacion y el sistema
// los publica el host (`settings::acerca_de`), y las bibliotecas estan
// controladas por una prueba contra los Cargo.toml.
//
// luna://about sigue existiendo: incluye esta misma seccion, sola.
(function arranque() {
  if (!globalThis.AjustesKit || !AjustesKit.listo()) return void requestAnimationFrame(arranque);
  const K = AjustesKit, C = K.C();
  const ajustes = hiperspace.dimention.settings;

  /** Lo que sabe hacer, en una linea cada cosa. El punto de color es para que
   *  la lista se lea de un vistazo y no como un parrafo cortado en renglones. */
  const CAPACIDADES = [
    { nombre: "HSML", detalle: "documentos espaciales, como HTML pero en 3D", color: C.azul },
    { nombre: "JavaScript", detalle: "V8 por deno_core, un isolate por espacio", color: C.amarillo },
    { nombre: "Componentes", detalle: "includes con props y eventos hacia el padre", color: C.violeta },
    { nombre: "Interfaces", detalle: "el framework con que está hecho esta ventana", color: C.cian },
    { nombre: "Audio", detalle: "archivos y AudioStream, que sintetiza sin archivos", color: C.verde },
    { nombre: "luna://", detalle: "páginas internas: inicio, demos, ajustes", color: C.naranja },
    { nombre: "Caché HTTP", detalle: "modelos y documentos se bajan una sola vez", color: C.textoTenue },
    { nombre: "Agente (MCP)", detalle: "un agente local puede mirar y abrir páginas", color: C.rojo },
  ];

  const datos = {
    version: "—", chip: "cargando…",
    perfil: "—", sistema: "—",
    bibliotecas: [],
    capacidades: CAPACIDADES,
    rutaConfig: "—", urlRaiz: "—",
  };

  const PLANTILLA_CAPACIDAD = `
<Grid ColumnDefinitions="Auto,*" ColumnSpacing="0.012" Margin="0.014,0.007">
  <Border Grid.Column="0" Width="0.012" Height="0.012" CornerRadius="0.006"
          Background="{Binding color}" VerticalAlignment="Center"/>
  <StackPanel Grid.Column="1" Orientation="Vertical" Spacing="0.001">
    <TextBlock Text="{Binding nombre}" FontSize="0.025" Foreground="${C.texto}"/>
    <TextBlock Text="{Binding detalle}" FontSize="0.019" Foreground="${C.textoTenue}"/>
  </StackPanel>
</Grid>`;

  const PLANTILLA_BIBLIOTECA = `
<Grid ColumnDefinitions="*,Auto" Margin="0.014,0.009">
  <TextBlock Grid.Column="0" Text="{Binding nombre}" FontSize="0.025" Foreground="${C.texto}"/>
  <TextBlock Grid.Column="1" Text="{Binding version}" FontSize="0.023" Foreground="${C.textoTenue}"/>
</Grid>`;

  const contenido =
    // La cabecera: el nombre grande, que es, y la version en una pastilla.
    `<Border Background="${C.superficieAlta}" CornerRadius="0.02" Padding="0.022,0.02">
      <StackPanel Orientation="Vertical" Spacing="0.006">
        <TextBlock Text="Luna" FontSize="0.07" Foreground="${C.texto}"/>
        <TextBlock Text="Un navegador web espacial: documentos en 3D, en escritorio y en VR."
                   FontSize="0.024" Foreground="${C.textoTenue}" TextWrapping="Wrap"/>
        <StackPanel Orientation="Horizontal" Spacing="0.008" Margin="0,0.008,0,0">
          <Border Background="${C.azul}" CornerRadius="0.012" Padding="0.012,0.005">
            <TextBlock Text="{Binding chip}" FontSize="0.02" Foreground="${C.texto}"/>
          </Border>
        </StackPanel>
      </StackPanel>
    </Border>` +
    K.encabezado("Esta compilación") +
    K.grupo(
      K.fila("Versión", null,
        `<TextBlock Grid.Column="1" Text="{Binding version}" FontSize="0.024" Foreground="${C.textoTenue}" VerticalAlignment="Center"/>`) +
      K.raya() +
      K.fila("Perfil", null,
        `<TextBlock Grid.Column="1" Text="{Binding perfil}" FontSize="0.024" Foreground="${C.textoTenue}" VerticalAlignment="Center"/>`) +
      K.raya() +
      K.fila("Sistema", null,
        `<TextBlock Grid.Column="1" Text="{Binding sistema}" FontSize="0.024" Foreground="${C.textoTenue}" VerticalAlignment="Center"/>`),
      null) +
    K.encabezado("Hecho con") +
    K.grupo(`<ItemsControl ItemsSource="{Binding bibliotecas}" ItemTemplate="biblioteca" Spacing="0"/>`, null) +
    K.encabezado("Qué sabe hacer") +
    K.grupo(`<ItemsControl ItemsSource="{Binding capacidades}" ItemTemplate="capacidad" Spacing="0"/>`, null) +
    K.encabezado("Dónde vive") +
    K.grupo(
      K.fila("Configuración", "{Binding rutaConfig}", "") +
      K.raya() +
      K.fila("Shell raíz", "{Binding urlRaiz}", ""),
      "Sólo para mirar: lo que se cambia está en General y en Personalizar.") +
    K.encabezado("Proyecto") +
    K.grupo(
      K.fila("Luna Browser", "por Sam",
        `<StackPanel Grid.Column="1" Orientation="Horizontal" Spacing="0.008" VerticalAlignment="Center">
           <Button Content="Demos" Background="${C.violeta}" FontSize="0.022" CornerRadius="0.009" Click="irDemos"/>
           <Button Content="Inicio" Background="${C.azul}" FontSize="0.022" CornerRadius="0.009" Click="irInicio"/>
         </StackPanel>`),
      null);

  const app = K.montar({
    datos: datos,
    plantillas: { capacidad: PLANTILLA_CAPACIDAD, biblioteca: PLANTILLA_BIBLIOTECA },
    contenido: contenido,
    manejadores: {
      irDemos: function () { K.navegar("luna://demos"); },
      irInicio: function () { K.navegar("luna://home"); },
    },
  });

  if (ajustes) ajustes.on(function (cfg) {
    if (!cfg) return;
    const a = cfg.about || {};
    datos.version = a.version ? "v" + a.version : "—";
    datos.perfil = a.perfil === "release" ? "release (optimizado)"
                 : a.perfil === "debug" ? "debug (sin optimizar)" : "—";
    datos.sistema = a.sistema ? a.sistema + " · " + (a.arquitectura || "?") : "—";
    datos.chip = (a.version ? "v" + a.version : "versión desconocida") + (a.perfil ? " · " + a.perfil : "");
    datos.bibliotecas = (Array.isArray(a.bibliotecas) ? a.bibliotecas : []).map(function (b) {
      return { nombre: String(b.nombre || ""), version: String(b.version || "") };
    });
    datos.rutaConfig = cfg.configPath || "—";
    datos.urlRaiz = cfg.rootUrl || "—";
    app.invalidar();
  });
  else console.error("[ajustes] falta dimention.settings: la sección se dibuja sin datos");
})();
