# Un ejemplo completo

Un documento y su script, enteros y funcionando. Es el camino más corto para ver
cómo encajan las piezas de [documento.md](documento.md),
[permisos.md](permisos.md) y [javascript.md](javascript.md).

---

## El documento y su script

Dos archivos servidos desde `http://localhost:2052/`.

### `demo.hsml`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<hsml>
  <head>
    <name>Demo Luna</name>
  </head>

  <!-- Coma, no espacio. fetch_text habilita lectura del propio origen. -->
  <space resources="navigate_self,read_camera_pose,fetch_text">
    <group id="panel" z="-3">
      <text y="2.0" value="Demo Luna" size="0.28" color="#344C49"/>
      <text id="contador" y="1.72" value="toques: 0" size="0.09" color="#4E6560"/>

      <plane y="1.2" sx="2.2" sy="0.9" sz="1" color="#E9E4D2" border-radius="0.08"/>

      <box id="btn_girar" x="-0.6" y="1.2" z="0.05"
           sx="0.8" sy="0.3" sz="0.08"
           color="#405E55" touchable="true" border-radius="0.04"/>
      <text x="-0.6" y="1.2" z="0.1" value="Girar" size="0.08" color="#F4F1E5"/>

      <box id="btn_home" x="0.6" y="1.2" z="0.05"
           sx="0.8" sy="0.3" sz="0.08"
           color="#7A4A3A" touchable="true" border-radius="0.04"/>
      <text x="0.6" y="1.2" z="0.1" value="Inicio" size="0.08" color="#F4F1E5"/>
    </group>

    <sphere id="orbe" y="0.6" z="-2" s="0.25" color="#C9A227" touchable="true"/>

    <script src="demo.js"/>
  </space>
</hsml>
```

### `demo.js`

```js
const root = hiperspace.dimention;

const contador = root.getElementById('contador');
const orbe     = root.getElementById('orbe');
const btnGirar = root.getElementById('btn_girar');
const btnHome  = root.getElementById('btn_home');

let toques = 0;
let girando = false;

// --- Eventos -------------------------------------------------------------
orbe.addEventListener('toque', (e) => {
  toques += 1;
  contador.setAttribute('value', `toques: ${toques}`);
  console.log('impacto en', e.x.toFixed(2), e.y.toFixed(2), e.z.toFixed(2));

  // Persistimos entre visitas (mismo origen).
  localStorage.setItem('toques', String(toques));
});

btnGirar.addEventListener('toque', () => {
  girando = !girando;
});

btnHome.addEventListener('toque', () => {
  location.href = 'luna://home';   // necesita navigate_self
});

// --- Estado previo -------------------------------------------------------
toques = Number(localStorage.getItem('toques') || 0);
contador.setAttribute('value', `toques: ${toques}`);

// --- Nodos creados desde JS ---------------------------------------------
const satelites = [];
for (let i = 0; i < 6; i++) {
  const s = root.createElement('box');
  s.setAttribute('color', '#405E55');
  s.scale = { x: 0.06, y: 0.06, z: 0.06 };
  root.appendChild(s);
  satelites.push(s);
}

// --- Loop ----------------------------------------------------------------
let t = 0;
function frame() {
  if (girando) {
    t += 0.02;
    orbe.rotation.y = t;

    // Un solo op para los 6 satélites en vez de 6 escrituras sueltas.
    const batch = [];
    satelites.forEach((s, i) => {
      const a = t + (i * Math.PI * 2) / satelites.length;
      batch.push(
        s.nodeId,
        Math.cos(a) * 0.7,
        0.6 + Math.sin(a * 2) * 0.15,
        -2 + Math.sin(a) * 0.7,
        0, a, 0,
      );
    });
    root.setTransformBatch(batch);
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

// --- Pose del usuario (read_camera_pose) ---------------------------------
// Con read_camera_pose llega la posición y la orientación en cero; con
// read_hmd_pose vendría completa, pero una página remota no la recibe.
const pose = root.readViewerPose && root.readViewerPose();
if (pose) console.log('modo:', pose.mode, 'en', pose.px.toFixed(2), pose.pz.toFixed(2));

// --- Red -----------------------------------------------------------------
// Lectura del propio servidor; requiere fetch_text en el space.
fetch('./data.json')
  .then(r => r.json())
  .then(d => console.log('datos', d))
  .catch(e => console.warn('fetch falló', e));

// Lo que sí funciona desde un origen remoto: que el motor traiga el documento.
// Los <include> los resuelve el motor, no el JS, así que no piden permiso.
const inc = root.createElement('include');
inc.setAttribute('src', 'http://localhost:2052/pedazo.hsml?q=1');
root.getElementById('panel').appendChild(inc);
```

### Probarlo

```bash
docker compose up -d          # Caddy + Bun sirven el directorio en :2052
cargo run -p luna --profile release-fast
```

Dentro de Luna, escribí `http://localhost:2052/demo.hsml` en la barra de direcciones.
En escritorio: WASD + Q/E para moverse, Shift + W para correr hacia adelante,
mouse para mirar y click para `toque`. En VR, presionar el stick izquierdo
mientras se mueve activa la carrera.
En VR: gatillo derecho para `toque`.

---

Volver al [índice de guías](index.md).
