# Mouse en escritorio remoto

Luna detecta sesiones RDP de Windows mediante WTS y vuelve a consultar cada
dos segundos para reconocer conexiones a una aplicación ya abierta. Usa
`WTSIsRemoteSession`, con `WTSClientProtocolType` y `SM_REMOTESESSION` como
alternativas. Las APIs se consultan para la sesión del proceso; no se depende
de una variable de entorno capturada al iniciar.

En RDP, **clic derecho captura el cursor y activa la mirada; Escape lo libera**.
No hace falta mantener el botón presionado. El cursor queda visible y confinado
a la ventana. Cerca de un borde se solicita moverlo al centro para poder seguir
girando. El modo remoto calcula desplazamientos entre posiciones absolutas y
descarta `MouseMotion`, evitando sumar ambos canales.

El cursor debe permanecer visible en esta implementación: `winit` 0.30.5 en
Windows reduce el confinamiento a un solo píxel cuando se combina captura con
cursor oculto, incluso usando `Confined`. Eso impide leer desplazamientos
absolutos mediante `CursorMoved`. En local se conserva el cursor oculto porque
la cámara utiliza movimiento relativo. La prueba de captura verifica también
esta condición; no sustituye una prueba manual con el cliente RDP.

El recentrado no se cuenta como movimiento: se conserva la referencia anterior
hasta recibir su respuesta. Los paquetes anteriores que todavía estén en cola
siguen contando. Si Windows combina la respuesta con un movimiento, se toma ese
primer punto cercano al centro como nueva referencia (puede perderse ese pequeño
movimiento). Se permite una sola solicitud pendiente para evitar recentrados en
cada frame. Si el cliente ignora la solicitud, el giro puede detenerse al llegar
al borde; Escape permite recuperar el puntero.

El primer punto establece la referencia. Perder el foco, activar otra vez la
mirada, cambiar resolución/DPI o recibir un salto anormal reinicia esa
referencia, sin traducir el salto a un giro. Fuera de la ventana no gira.
La sensibilidad remota usa píxeles lógicos de ventana; puede sentirse distinta
según el escalado y la aceleración del cliente RDP.

En una sesión local se conserva el modo habitual: clic derecho activa la
mirada con cursor bloqueado y movimiento relativo; Escape la desactiva.

Para diagnóstico o clientes que Windows no identifica como remotos, se puede
forzar el modo antes de iniciar Luna:

```powershell
$env:LUNA_MOUSE_INPUT = 'absolute' # posiciones con captura y recentrado
# 'raw' fuerza el comportamiento local; 'auto' usa detección automática.
```

Esto no cambia el protocolo RDP ni puede recuperar movimientos que el cliente
no envíe. El giro continuo depende de que el cliente respete el recentrado.
La detección automática está dirigida a RDP; no identifica por
sí sola AnyDesk, RustDesk o Chrome Remote Desktop.

Referencias de implementación: [información de sesión WTS](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ne-wtsapi32-wts_info_class)
y [consulta de sesión](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtsquerysessioninformationw).
