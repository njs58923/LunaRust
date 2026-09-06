# Entorno de páginas internas

`luna://environment` sirve `crates/luna/src/web/environment.hsml`: cielo, islote,
rocas y vegetación estática. `luna://home` solo contiene su navegación.

El root API monta el entorno una vez dentro de un grupo propio, hermano de las
pestañas. No es una pestaña ni una app: cerrar o reemplazar la página spatial y
cambiar entre shell de escritorio y VR no lo desmonta. Tampoco cambia la pose
del usuario. Al volver desde una página externa reutiliza el mismo include.

La lista `nativePages` en `web/internal/root_api.js` identifica los documentos
nativos que usan este fondo: home, demos, settings, about, cache-stats, error/404,
scale_demo, fire_demo, target_demo y range_demo. Una app aditiva o embedded no
activa el entorno sobre una página externa. La ruta environment puede visitarse
sola y no activa otra copia del fondo.

El mantenimiento consulta únicamente los wrappers del pequeño registro de
pestañas, sin recorrer los elementos de las demos. Lee el `src` actual del
include, porque `location.href` reemplaza el documento sin llamar a mountSpace.
Solo escribe visibilidad cuando cambia entre contenido nativo y externo.

## Convenciones de contenido

- El nivel del terreno es y=0. No se traslada el espacio raíz para elevar paneles:
  las demos que reciben poses de mandos conservan sus coordenadas mundiales.
- Los paneles estáticos se agrupan a una altura legible; los proyectiles y
  blancos dinámicos siguen siendo hijos del espacio sin ese desplazamiento.
- Las páginas no añaden otro suelo. En Scale Demo, Toggle Guides controla sus
  referencias de distancia; no permite ocultar el terreno compartido.
- Los proyectiles se retiran al bajar al nivel del suelo; los blancos y las
  primitivas animadas se mantienen sobre él. Esto no añade físicas al navegador.
- El cielo abarca las distancias usadas por Scale Demo. Los objetos lejanos
  pueden flotar fuera del islote: este sigue siendo un placeholder pequeño.
- Los documentos HTTP/HTTPS y los archivos de pruebas externos no se modifican.

Las pruebas del root API cubren reutilización, navegación mediante include,
cambio de shell, salida a una página externa y apps embedded sobre contenido
externo.
