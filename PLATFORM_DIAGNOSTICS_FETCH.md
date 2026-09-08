# Diagnósticos, logs MCP y fetch de mismo origen

## Lectura de red

Una página espacial remota puede declarar `<space resources="fetch_text">`.
El shell normal ofrece esa capacidad, pero no se activa si el documento no la
pide. Un `include` debe recibir `resources="fetch_text"` de un padre que ya lo
tenga; el documento incluido también debe pedirlo. Un montaje con `grants: []`
conserva ese techo vacío. Las apps de otros kinds conservan sus grants anteriores.

```js
fetch('./api/state')
  .then(response => response.json())
  .then(state => console.log(state))
  .catch(error => console.error(error.message));
```

El host resuelve rutas relativas contra el documento propietario y comprueba
protocolo, host y puerto. Se rechazan URLs con credenciales y protocolos ajenos
a HTTP(S). Las redirecciones se validan antes de seguirlas, con máximo 5 saltos;
la respuesta tiene un límite de 8 MiB. Se conservan los límites de concurrencia
y timeouts del servicio IO. Los servicios nativos de origen luna conservan su
camino previo; esto no concede capacidades adicionales a páginas remotas.

`fetch_text` permite GET/HEAD. La ampliación `fetch_http` permite métodos de
escritura, cuerpo y cabeceras. Ambos devuelven estados HTTP reales: 4xx/5xx
resuelven con `ok: false`. Ver [Fetch HTTP](guides/red.md) para contrato y límites.

## Logs MCP

`luna_logs` acepta `limit` (1–300, default 100), `tabId`, `spaceId`, `level`
(`info`, `warn`, `error`), `pattern` (regex Rust, máximo 512 bytes) y `after`.
Sin cursor devuelve las últimas entradas; con cursor pagina hacia adelante en
orden de secuencia. Usar `nextCursor` para continuar. La respuesta indica `hasMore`
y `oldestAvailable`; puede haber huecos por retención o limpieza de logs.

La consola de las páginas registra timestamp, tab, espacio e identidad de runtime
al recibirse, para conservar la atribución después de una navegación. Los logs
del host sin contexto siguen accesibles en la consulta global.

Hay hasta 300 entradas por espacio y 3000 en total. Los mensajes se truncan a
4096 bytes más el marcador de truncamiento. El filtro regex se ejecuta en Rust,
con límites de compilación; nunca se evalúa como JavaScript. Solo está disponible
a través del bridge MCP local existente y su activación habitual.

## Diagnósticos básicos

- Recursos desconocidos y capacidades denegadas: al reconstruir políticas,
  deduplicados por entidad y mensaje durante su vida.
- IDs duplicados dentro de un mismo espacio del documento y meta-transformaciones
  ignoradas: al cargar XML, con hasta 64 avisos por documento.
- No cambia el parser de recursos: el separador sigue siendo la coma.

No se recorre toda la escena por cada frame para validar. No se añadió un esquema
general de atributos ni comprobación de IDs creados después desde JavaScript.
