# Recuperación desktop / VR

Luna conserva la preferencia `RenderMode` y la reconcilia con el estado real de
OpenXR. No depende exclusivamente de eventos de cambio ni crea sesiones desde
dos sistemas diferentes.

- Available + VR: pedir creación, con reintento cada 2 segundos si falla.
- Ready + VR: comenzar, incluso si se eligió VR cuando ya estaba en Ready.
- Running + desktop: solicitar salida. Nunca pedir salida desde Ready/Idle,
  donde xrRequestExitSession puede devolver SESSION_NOT_RUNNING.
- Stopping: finalizar. Idle: esperar al runtime. Exiting: destruir. Si el
  runtime pide salida sin reinicio, volver a desktop; si es pérdida recuperable,
  conservar la intención de volver a VR.
- La cámara desktop permanece activa mientras VR no llegue a Running o no deba
  dibujar (should_render=false). Los mandos se ocultan durante ese intervalo.
- Las cámaras XR se apagan al abandonar la sesión o cuando should_render es false.

La creación fallida no ejecuta XrSessionCreated ni publica su evento. La creación
exitosa pasa a Idle; la destrucción reinicia el flag de sesión comenzada. Antes
de recrear, se espera la confirmación de limpieza del mundo de render para no
pisar recursos de una sesión anterior en renderizado paralelo.

Los espacios internos de cabeza/manos y los mandos de Luna se eliminan al
destruir la sesión. No se elimina XrTrackingRoot: conserva la posición del
jugador. Los action sets siguen perteneciendo a la instancia OpenXR y se vuelven
a adjuntar a cada sesión. Una pausa Idle no necesita duplicar mandos ni espacios.

Un error en wait_frame inicia recuperación de sesión y apaga las cámaras XR.
Los errores al comenzar, finalizar o solicitar salida se registran sin panic;
la reconciliación vuelve a intentarlo. Los fallos temporales de locate_views
conservan la última pose en vez de terminar el proceso.

## Límites

Esto recupera sesiones sobre una instancia OpenXR ya inicializada. Si al arrancar
falló init_xr (runtime ausente o sin sistema disponible), el plugin instaló el
RenderPlugin de desktop y no conserva una instancia/dispositivo compatible para
XR. Esa ruta aún requiere reiniciar con el runtime disponible; no se cambia el
dispositivo gráfico de una app en marcha. El sample sessions usa el mismo init.

La pérdida completa de la instancia y todos los errores posibles del swapchain
no quedan resueltos por la recuperación de sesión. No se considera que una prueba
sin visor certifique reconexiones reales de SteamVR/Quest ni suspensión del SO.

## Validación

Pruebas automatizadas: alternar desde Ready, throttling de reintentos, ciclo
Stopping/Idle/Ready, salida desde Running, limpieza repetida de mandos/rig y
bloqueo del frame loop en Idle/Exiting. Compilación:

```sh
cargo check -p luna -p bevy_mod_openxr -p bevy_xr_utils --tests -j 1
cargo test -p luna --bin luna xr_lifecycle_tests -j 1
cargo test -p luna -p bevy_mod_openxr -p bevy_xr_utils --lib lifecycle_tests -j 1
```

Con visor: activar VR desde desktop; apagar/encender mandos; quitar/poner visor;
alternar a desktop durante Idle y regresar; repetir 10 ciclos. Comprobar un solo
mando por mano, rayos/clicks, locomoción, persistencia del tracking root y cámara
desktop utilizable al salir. Probar conexión tardía con runtime ya inicializado
por separado del arranque sin runtime.

Referencia: https://registry.khronos.org/OpenXR/specs/1.0/man/html/xrRequestExitSession.html
