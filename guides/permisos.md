# Permisos

Qué puede hacer un documento lo decide su atributo `resources`, y lo que un
`<include>` hereda de su padre.

---

## Los bundles

`<space resources="a,b,c">` declara qué capacidades pide el documento.
**Separadas por coma, sin espacios**: ver la advertencia 1 arriba. Bundles
reales (`permissions.rs`), los que un documento normal puede pedir:

| Bundle | Habilita |
|---|---|
| `navigate_self` | cambiar la URL del propio espacio (`location.href`) |
| `navigate_global` | navegar el shell entero |
| `fetch_text` | `fetch()` GET/HEAD con respuestas HTTP reales |
| `fetch_http` | `fetch()` con métodos, cabeceras y cuerpo de texto/JSON/formulario |
| `skybox` | montar `<skybox>` |
| `spawn` | aplicar una aparición por carga del documento espacial principal |
| `read_pose_stream` | eventos `posemove` de `<posezone>` |
| `read_hmd_pose` | `dimention.readViewerPose()` con pose completa (posición + hacia dónde mira) |
| `read_camera_pose` | `dimention.readViewerPose()` **sólo con posición**; la orientación viene en cero. Anda en escritorio y en VR |
| `capture_frame` | `dimention.captureFrame(nombre)` → PNG del frame. Elevado |
| `desktop_camera_control` | control de cámara en escritorio |
| `vr_locomotion` | locomoción con mandos |

> **Ojo con esta tabla**: lista lo que un documento *puede pedir*, no lo que va a
> *recibir*. Lo efectivo es la intersección con lo que el shell concede según el
> tipo de página. Una `spatial` remota hoy recibe `navigate_self`,
> `read_pose_stream`, `read_camera_pose`, `skybox`, `fetch_text`, `fetch_http` y `spawn` como
> concesiones predeterminadas. Los dos permisos fetch y `spawn` requieren además una
> solicitud explícita del documento. El shell puede limitar esas concesiones.

Elevados — **sólo** para páginas nativas o apps montadas por el shell trusted; un
origen remoto que los pida dispara UX de consentimiento o simplemente no los recibe:

`root`, `manage_tabs`, `ux_embed`, `read_system_input`,
`mount_root_space`, `unmount_root_space`, `update_root_space`, `list_root_spaces`.

Algunos bundles auto-inyectan su script:

| Bundle | Script inyectado | API que aparece |
|---|---|---|
| `root` | `luna://internal/root_api.js` | `dimension.luna.*` |
| `read_camera_pose` | `luna://internal/viewer_pose_api.js` | `dimention.readViewerPose()` (sin orientación) |
| `manage_tabs` | `luna://internal/tabs_api.js` | `dimention.tabs.*` |
| `ux_embed` | `luna://internal/embedded_api.js` | `dimention.embedded.*` |
| `read_hmd_pose` | `luna://internal/viewer_pose_api.js` | `dimention.readViewerPose()` |

### Herencia por `include`

`spawn` tiene una restricción adicional: sólo se usan marcadores del documento
principal de un montaje `spatial` visible. Los includes anidados, los espacios
anidados y las apps no pueden definir la aparición, aunque tengan el permiso
o compartan origen. Ver [Puntos de aparición](llegada.md).

Un `<include>` **no propaga permisos** por defecto: el contenido hijo arranca con
capacidades vacías. Si el `include` declara `resources`, eso genera un *entry grant*
para el documento hijo, siempre intersecado con lo que el padre ya tenía
(no se puede escalar privilegios desde un include).

---

---

Volver al [índice de guías](index.md).
