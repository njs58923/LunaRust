// GENERADO POR server_ui — NO EDITAR A MANO.
//
// Origen: server_ui/public/{nucleo.js,controles.js,interactivos.js,paneles.js,mas.js,host.js,superpuestos.js}
// Rehacer: cd server_ui && bun run build
//
// Un framework de interfaces declarativo sobre malla, en la línea de WPF: árbol
// declarativo, layout en dos pasadas (medir y acomodar), paneles que reparten el
// espacio. Lo usan las páginas internas de Luna; el proyecto entero, con sus
// demos y su banco de pruebas, está en server_ui/README.md y RENDERING.md.
//
// Cómo se carga, y por qué así:
//
//     <script>globalThis.UI_CFG = { base: "luna://", distancia: 1.6, C: {...} };</script>
//     <script src="luna://internal/ui.js"/>
//     <script>
//       // Los <script src> llegan asincrónicos y el motor NO garantiza el
//       // orden. Esperar es más barato que ordenar.
//       (function arranque() {
//         if (!globalThis.UI) return void requestAnimationFrame(arranque);
//         const app = new UI.Aplicacion({ ancho: 1.2, alto: 0.8 });
//         app.cargar('<TextBlock Text="hola"/>');
//         app.correr();
//       })();
//     </script>
//
// El `UI_CFG` va **en línea**: un script sin `src` no espera a la red y se
// evalúa antes que cualquier externo, y `nucleo.js` lee la configuración al
// cargarse. Al revés —la config en un archivo aparte— la paleta llegaría tarde
// y los colores saldrían con los valores de reserva, sin un solo aviso.

if (!globalThis.UI_CFG) {
  console.error("[ui] UI_CFG no está declarado. Va en un <script> EN LÍNEA " +
                "antes de este archivo; si no, la paleta cae a los valores de " +
                "reserva y los colores salen mal en silencio.");
}


// ──────────────────────────────────────────────────────────────────────
// nucleo.js
// ──────────────────────────────────────────────────────────────────────
// ui.js — un framework de interfaces declarativo, al estilo WPF, sobre malla.
//
// ┌───────────────────────────────────────────────────────────────────────────
// │ La apuesta
// └───────────────────────────────────────────────────────────────────────────
//
// WPF acomoda su árbol en **dos pasadas**: `Measure`, donde cada elemento dice
// cuánto querría medir dado el espacio disponible, y `Arrange`, donde el padre
// le asigna un rectángulo definitivo. Todo lo demás —paneles, alineaciones,
// márgenes, `Auto` y `*`— sale de esas dos.
//
// Ese modelo se puede traer entero a este motor, y la razón está medida:
// **rearmar una malla de 2000 rectángulos cuesta ~135 fps y una de 8000, ~74**
// (ver public/banco.js). O sea que el árbol visual se puede volcar de cero en
// cada pasada de layout. Sin diffing, sin árbol de nodos que mantener
// sincronizado: se mide, se acomoda, se emite. Inmediato.
//
// ┌───────────────────────────────────────────────────────────────────────────
// │ Las tres poblaciones
// └───────────────────────────────────────────────────────────────────────────
//
// No todo puede ser malla, y el reparto es forzado por el motor:
//
//   1. **Lo que se dibuja** → una malla. Fondos, bordes, rellenos, indicadores.
//      Un solo nodo, se rearma completo.
//   2. **El texto** → nodos `<text>`. No hay tipografía en malla. Se agrupan en
//      una pileta y se reciclan; nunca se crean dos veces.
//   3. **Lo que se toca** → nodos `<plane touchable>`. Una malla es UN nodo y
//      por lo tanto UN blanco: cada control interactivo necesita el suyo. Misma
//      pileta, mismo reciclado.
//
// ┌───────────────────────────────────────────────────────────────────────────
// │ El sistema de coordenadas
// └───────────────────────────────────────────────────────────────────────────
//
// Adentro se trabaja **como WPF**: origen arriba a la izquierda, `x` a la
// derecha, `y` hacia abajo, en metros. El motor es Y-arriba, así que hay una
// sola inversión y está en `emitir()`. Mezclar las dos convenciones en el código
// de layout es lo que vuelve ilegible cualquier port de WPF, y acá el precio de
// mantenerlas separadas es una resta.
//
// ┌───────────────────────────────────────────────────────────────────────────
// │ Sobre las curvas
// └───────────────────────────────────────────────────────────────────────────
//
// Hoy una esquina redondeada se teselan a mano en `Geometria`. Si la malla llega
// a soportar curvas, **lo único que cambia es ese objeto**: nadie más habla de
// triángulos. Los controles piden "un rectángulo con radio 2 cm" y no saben cómo
// se cumple.
(function () {
  "use strict";

  const CFG = globalThis.UI_CFG || {};
  const root = hiperspace.dimention;

  // ──────────────────────────────────────────────────────────────────────────
  // Utilidades
  // ──────────────────────────────────────────────────────────────────────────

  /** Un canal de sRGB a lineal.
   *
   *  Hace falta y no es cosmético. Los colores por vértice de una malla se
   *  entregan **en lineal**: con `material-unlit` prendido, pasar el 0,11 de un
   *  #1C1C26 tal cual da un gris claro en pantalla, porque el pipeline lo
   *  convierte a sRGB al escribir y lo sube a 0,37. Un panel casi negro salía
   *  color lavanda.
   *
   *  Con el material iluminado el error se disimulaba —la luz ambiente lo
   *  bajaba de vuelta, por casualidad— y por eso la primera versión parecía
   *  «apagada pero correcta». Estaba mal en los dos casos. */
  function aLineal(c) {
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  }

  function rgba(hex, alfa) {
    let s=String(hex||"#000000");
    if(s.toLowerCase()==="transparent")return [0,0,0,0];
    s=s.replace(/^#/,"");
    if(s.length===3||s.length===4)s=[...s].map(c=>c+c).join("");
    if(!/^(?:[a-f0-9]{6}|[a-f0-9]{8})$/i.test(s))return [0,0,0,1];
    const c=[0,2,4].map(i=>aLineal(parseInt(s.slice(i,i+2),16)/255));
    return [...c,(s.length===8?parseInt(s.slice(6,8),16)/255:1)*(alfa===undefined?1:alfa)];
  }

  /** Aclarar u oscurecer un color. Los estados —hover, presionado, deshabilitado—
   *  salen de acá y no de más propiedades: pedirle a cada control tres colores
   *  es pedirle al que escribe la interfaz que sea diseñador. */
  function aSRGB(c) {
    return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
  }

  function tinte(hex, k) {
    const c = rgba(hex);
    const f = function (x) {
      const y = Math.max(0, Math.min(255, Math.round(aSRGB(x) * 255 * k)));
      const s = y.toString(16);
      return s.length < 2 ? "0" + s : s;
    };
    return "#" + f(c[0]) + f(c[1]) + f(c[2]) + (c[3]<1?Math.round(c[3]*255).toString(16).padStart(2,"0"):"");
  }

  const NADA = Object.freeze({ i: 0, d: 0, s: 0, f: 0 });

  /** Un `Thickness`: uno, dos o cuatro números.
   *    "0.02"              → todos
   *    "0.02,0.01"         → horizontal, vertical
   *    "0.02,0.01,0,0.03"  → izq, arriba, der, abajo   (orden de WPF) */
  function grosor(v) {
    if (v === undefined || v === null || v === "") return NADA;
    if (typeof v === "number") return { i: v, d: v, s: v, f: v };
    const n = String(v).split(/[,\s]+/).filter(function (x) { return x !== ""; })
      .map(Number);
    if (n.some(function (x) { return !isFinite(x); })) return NADA;
    if (n.length === 1) return { i: n[0], d: n[0], s: n[0], f: n[0] };
    if (n.length === 2) return { i: n[0], s: n[0], d: n[1], f: n[1] };
    return { i: n[0], d: n[1], s: n[2], f: n[3] };
  }
  const anchoDe = function (g) { return g.i + g.s; };
  const altoDe = function (g) { return g.d + g.f; };

  function numero(v, def) {
    if (v === undefined || v === null || v === "") return def;
    if (v === "Auto" || v === "auto" || v === "NaN") return NaN;
    const n = Number(v);
    return isFinite(n) ? n : def;
  }

  function booleano(v, def) {
    if (v === undefined || v === null || v === "") return def;
    return v === true || v === "true" || v === "True" || v === "1";
  }

  // ──────────────────────────────────────────────────────────────────────────
  // El lector de marcado
  // ──────────────────────────────────────────────────────────────────────────
  // Un XML mínimo: elementos, atributos con comillas, cierre propio, comentarios
  // y texto suelto (que se toma como `Content`). No hay namespaces, entidades más
  // allá de las cinco básicas, ni CDATA. Es a propósito: lo que no está no se
  // puede usar mal, y un parser completo acá sería una tarde entera para
  // sostener características que la interfaz no necesita.
  const ENTIDADES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };

  function desescapar(s) {
    return s.replace(/&(#x?[0-9a-fA-F]+|[a-z]+);/g, function (todo, e) {
      if (e[0] === "#") {
        const n = e[1] === "x" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
        return isFinite(n) ? String.fromCharCode(n) : todo;
      }
      return ENTIDADES[e] !== undefined ? ENTIDADES[e] : todo;
    });
  }

  function leerMarcado(texto) {
    let i = 0;
    const raiz = { hijos: [] };
    const pila = [raiz];

    function tope() { return pila[pila.length - 1]; }

    while (i < texto.length) {
      const menor = texto.indexOf("<", i);
      if (menor < 0) break;

      const suelto = texto.slice(i, menor).trim();
      if (suelto && tope().atributos) {
        tope().texto = (tope().texto || "") + desescapar(suelto);
      }

      if (texto.startsWith("<!--", menor)) {
        i = texto.indexOf("-->", menor);
        i = i < 0 ? texto.length : i + 3;
        continue;
      }
      if (texto.startsWith("<?", menor)) {
        i = texto.indexOf("?>", menor);
        i = i < 0 ? texto.length : i + 2;
        continue;
      }

      const mayor = texto.indexOf(">", menor);
      if (mayor < 0) break;
      let cuerpo = texto.slice(menor + 1, mayor).trim();

      if (cuerpo[0] === "/") {
        if (pila.length > 1) pila.pop();
        i = mayor + 1;
        continue;
      }

      const propio = cuerpo.endsWith("/");
      if (propio) cuerpo = cuerpo.slice(0, -1).trim();

      const espacio = cuerpo.search(/\s/);
      const nombre = espacio < 0 ? cuerpo : cuerpo.slice(0, espacio);
      const resto = espacio < 0 ? "" : cuerpo.slice(espacio);

      const atributos = {};
      const re = /([\w.:-]+)\s*=\s*"([^"]*)"|([\w.:-]+)\s*=\s*'([^']*)'/g;
      let m;
      while ((m = re.exec(resto)) !== null) {
        atributos[m[1] || m[3]] = desescapar(m[2] !== undefined ? m[2] : m[4]);
      }

      const nodo = { nombre: nombre, atributos: atributos, hijos: [], texto: "" };
      tope().hijos.push(nodo);
      if (!propio) pila.push(nodo);
      i = mayor + 1;
    }
    return raiz.hijos[0] || null;
  }

  // ──────────────────────────────────────────────────────────────────────────
  // Geometría
  // ──────────────────────────────────────────────────────────────────────────
  // El único lugar del framework que sabe que existen los triángulos.
  //
  // Los controles declaran contornos y radios. Este backend integra la
  // teselación adaptativa de PathGeometry, el recorte y los buffers del panel.
  function Geometria() {
    this.pos = [];
    this.col = [];
    this.nor = [];
    this.idx = [];
    this.v = 0;
    /** Recorte vigente y su pila. Ver `recortar`. */
    this.clip = null;
    this.pilaClip = [];
    /** Se sube y baja para que lo que se dibuja después quede delante. Un
     *  framework de interfaces necesita orden de pintado, y sin alfa ni
     *  z-testing por capas la única herramienta es separar en z. */
    this.z = 0;
  }

  /** El rectángulo de recorte vigente, o null. Es una pila: un ScrollViewer
   *  dentro de otro tiene que quedarse con la intersección de los dos.
   *
   *  No hay tijera de GPU acá, así que el recorte se hace **en la geometría**:
   *  cada rectángulo se intersecta antes de emitirse. Funciona perfecto para lo
   *  que es rectangular, que es casi todo, y tiene un límite honesto: una
   *  esquina redondeada cortada por el borde pierde su redondeo del lado
   *  cortado, porque el abanico ya no se puede recortar sin retesela. Se
   *  documenta y se sigue: en una lista con scroll, el elemento que asoma por
   *  el borde se corta recto y nadie lo nota. */
  Geometria.prototype.recortar = function (x, y, w, h) {
    const a = this.clip;
    const n = a ? {
      x: Math.max(a.x, x), y: Math.max(a.y, y),
      x1: Math.min(a.x1, x + w), y1: Math.min(a.y1, y + h),
    } : { x: x, y: y, x1: x + w, y1: y + h };
    this.pilaClip.push(this.clip);
    this.clip = n;
  };

  Geometria.prototype.restaurar = function () {
    this.clip = this.pilaClip.length ? this.pilaClip.pop() : null;
  };

  /** Intersectar un rectángulo con el recorte. Devuelve null si no queda nada,
   *  y eso es lo que apaga el dibujo entero de un elemento fuera de vista. */
  Geometria.prototype.aplicarClip = function (x, y, w, h) {
    const c = this.clip;
    if (!c) return { x: x, y: y, w: w, h: h, entero: true };
    const x0 = Math.max(x, c.x), y0 = Math.max(y, c.y);
    const x1 = Math.min(x + w, c.x1), y1 = Math.min(y + h, c.y1);
    if (!(x1 > x0 && y1 > y0)) return null;
    const entero = x0 === x && y0 === y && x1 === x + w && y1 === y + h;
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0, entero: entero };
  };

  /** ¿Está este punto adentro del recorte? Lo usan los textos, que no se pueden
   *  cortar por la mitad: o se dibujan o no. */
  Geometria.prototype.puntoVisible = function (x, y) {
    const c = this.clip;
    return !c || (x >= c.x && x <= c.x1 && y >= c.y && y <= c.y1);
  };

  Geometria.prototype.reiniciar = function () {
    this.pos.length = 0;
    this.col.length = 0;
    this.nor.length = 0;
    this.idx.length = 0;
    this.v = 0;
    this.z = 0;
    this.clip = null;
    this.pilaClip.length = 0;
  };

  Geometria.prototype.vertice = function (x, y, c) {
    // Acá y sólo acá se invierte la Y: adentro se piensa como WPF —hacia
    // abajo—, el motor dibuja hacia arriba.
    this.pos.push(x, -y, this.z);
    this.nor.push(0, 0, 1);
    this.col.push(c[0], c[1], c[2], c[3]);
    return this.v++;
  };

  /** El orden de los índices, y por qué es el que parece al revés.
   *
   *  Adentro se piensa con Y hacia abajo, como WPF. `vertice()` la invierte al
   *  emitir. Eso **da vuelta el sentido de giro**: un triángulo que en la hoja
   *  se recorre en sentido antihorario, en el mundo queda horario, y el motor
   *  lo descarta por mirar para atrás.
   *
   *  Costó una tarde la primera vez y no da ningún error: la malla se crea, los
   *  triángulos se cuentan, y no se ve nada. Por eso el orden se centraliza acá
   *  y nadie más arma índices a mano. */
  Geometria.prototype.triangulo = function (a, b, c) {
    this.idx.push(a, c, b);
  };

  /** Un rectángulo. `x, y` es su esquina superior izquierda. */
  Geometria.prototype.rect = function (x, y, w, h, color) {
    if (w <= 0 || h <= 0) return;
    const r = this.aplicarClip(x, y, w, h);
    if (!r) return;
    x = r.x; y = r.y; w = r.w; h = r.h;
    const c = rgba(color);
    const a = this.vertice(x, y, c);
    const b = this.vertice(x + w, y, c);
    const d = this.vertice(x + w, y + h, c);
    const e = this.vertice(x, y + h, c);
    this.triangulo(a, b, d);
    this.triangulo(a, d, e);
    // (el sentido lo corrige `triangulo`; acá se escribe como se lee en la hoja)
  };

  const curveCache=new Map();
  const CURVE_TOLERANCE=0.00005;
  function roundedCommands(w,h,r) {
    r=Math.max(0,Math.min(r,w/2,h/2));const k=r*0.5522847498307936;
    return [{op:"moveTo",x:r,y:0},{op:"lineTo",x:w-r,y:0},
      {op:"bezierTo",c1x:w-r+k,c1y:0,c2x:w,c2y:r-k,x:w,y:r},
      {op:"lineTo",x:w,y:h-r},{op:"bezierTo",c1x:w,c1y:h-r+k,c2x:w-r+k,c2y:h,x:w-r,y:h},
      {op:"lineTo",x:r,y:h},{op:"bezierTo",c1x:r-k,c1y:h,c2x:0,c2y:h-r+k,x:0,y:h-r},
      {op:"lineTo",x:0,y:r},{op:"bezierTo",c1x:0,c1y:r-k,c2x:r-k,c2y:0,x:r,y:0},{op:"close"}];
  }
  function roundedGeometry(w,h,r) {
    r=Math.max(0,Math.min(r,w/2,h/2));const key=[w,h,r].join(',');
    if(curveCache.has(key))return curveCache.get(key);
    let geometry;
    if(globalThis.PathGeometry && r>0)geometry=PathGeometry.tessellate(roundedCommands(w,h,r),{tolerance:CURVE_TOLERANCE});
    else {
      // Compatibility path: adaptive arcs, not a fixed polygon count.
      const points=[];
      if(r<=0)points.push([0,0],[w,0],[w,h],[0,h]);
      else {
        const steps=Math.max(2,Math.min(64,Math.ceil((Math.PI/2)/(2*Math.acos(Math.max(-1,1-CURVE_TOLERANCE/r))))));
        for(const [x,y,a] of [[w-r,r,-Math.PI/2],[w-r,h-r,0],[r,h-r,Math.PI/2],[r,r,Math.PI]])
          for(let i=0;i<=steps;i++)points.push([x+Math.cos(a+i/steps*Math.PI/2)*r,y+Math.sin(a+i/steps*Math.PI/2)*r]);
      }
      const indices=[];for(let i=1;i+1<points.length;i++)indices.push(0,i,i+1);
      geometry={positions:points,indices,contours:[points],closed:[true]};
    }
    if(curveCache.size>=128)curveCache.delete(curveCache.keys().next().value);
    curveCache.set(key,geometry);return geometry;
  }
  function clipPolygon(polygon,clip) {
    if(!clip)return polygon;
    for(const [axis,bound,sign] of [[0,clip.x,1],[0,clip.x1,-1],[1,clip.y,1],[1,clip.y1,-1]]) {
      const out=[];
      for(let i=0;i<polygon.length;i++){
        const a=polygon[i],b=polygon[(i+1)%polygon.length],ia=(a[axis]-bound)*sign>=0,ib=(b[axis]-bound)*sign>=0;
        if(ia)out.push(a);
        if(ia!==ib){const t=(bound-a[axis])/(b[axis]-a[axis]);out.push([a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])]);}
      }
      polygon=out;
    }
    return polygon;
  }
  Geometria.prototype.path = function(geometry,x,y,color) {
    const c=rgba(color);
    if(!this.clip){
      const vertices=geometry.positions.map(p=>this.vertice(p[0]+x,p[1]+y,c));
      for(let i=0;i<geometry.indices.length;i+=3)this.triangulo(vertices[geometry.indices[i]],vertices[geometry.indices[i+1]],vertices[geometry.indices[i+2]]);
      return;
    }
    for(let i=0;i<geometry.indices.length;i+=3){
      const polygon=clipPolygon(geometry.indices.slice(i,i+3).map(j=>[geometry.positions[j][0]+x,geometry.positions[j][1]+y]),this.clip);
      for(let j=1;j+1<polygon.length;j++){
        const a=polygon[0],b=polygon[j],d=polygon[j+1];
        if(Math.abs((b[0]-a[0])*(d[1]-a[1])-(b[1]-a[1])*(d[0]-a[0]))<1e-12)continue;
        this.triangulo(this.vertice(...a,c),this.vertice(...b,c),this.vertice(...d,c));
      }
    }
  };
  Geometria.prototype.rectRedondeado = function(x,y,w,h,r,color) {
    if(!(w>0&&h>0)||!this.aplicarClip(x,y,w,h))return;
    if(r<=0)return this.rect(x,y,w,h,color);
    this.path(roundedGeometry(w,h,r),x,y,color);
  };


  /** Un marco: cuatro rectángulos, no un rectángulo grande con otro encima.
   *  Dibujar el relleno arriba del borde obliga a otro nivel de z por cada
   *  control anidado, y el z se agota. */
  Geometria.prototype.marco = function (x, y, w, h, g, color) {
    if (g.d > 0) this.rect(x, y, w, g.d, color);
    if (g.f > 0) this.rect(x, y + h - g.f, w, g.f, color);
    const yi = y + g.d, hi = h - g.d - g.f;
    if (g.i > 0) this.rect(x, yi, g.i, hi, color);
    if (g.s > 0) this.rect(x + w - g.s, yi, g.s, hi, color);
  };

  // Closed convex extrusion. Clip the contour first, then generate front,
  // back and side faces with independent normals (no rounded side artifacts).
  Geometria.prototype.solidRect = function(x,y,w,h,radius,depth,color) {
    if(!(w>0 && h>0 && depth>0))return;
    const polygon=clipPolygon(roundedGeometry(w,h,radius).contours[0].map(p=>[p[0]+x,p[1]+y]),this.clip);
    if(polygon.length<3)return;
    const c=rgba(color),front=this.z,back=front-depth,self=this;
    function vertex(p,z,n,k){const id=self.v++;self.pos.push(p[0],-p[1],z);self.nor.push(...n);self.col.push(c[0]*k,c[1]*k,c[2]*k,c[3]);return id;}
    const top=polygon.map(p=>vertex(p,front,[0,0,1],1));
    const bottom=polygon.map(p=>vertex(p,back,[0,0,-1],0.6));
    for(let i=1;i+1<polygon.length;i++){
      this.idx.push(top[0],top[i+1],top[i],bottom[0],bottom[i],bottom[i+1]);
    }
    for(let i=0;i<polygon.length;i++){
      const a=polygon[i],b=polygon[(i+1)%polygon.length],dx=b[0]-a[0],dy=b[1]-a[1],length=Math.hypot(dx,dy);
      if(length<1e-9)continue;
      const normal=[dy/length,dx/length,0],shade=0.65+0.1*Math.abs(normal[0]);
      const v0=vertex(a,back,normal,shade),v1=vertex(b,back,normal,shade),v2=vertex(b,front,normal,shade),v3=vertex(a,front,normal,shade);
      this.idx.push(v0,v2,v1,v0,v3,v2);
    }
  };

  Geometria.prototype.malla = function () {
    return {
      positions: new Float32Array(this.pos),
      indices: new Uint32Array(this.idx),
      normals: new Float32Array(this.nor),
      colors: new Float32Array(this.col),
    };
  };

  // ──────────────────────────────────────────────────────────────────────────
  // Piletas de nodos
  // ──────────────────────────────────────────────────────────────────────────
  // Textos y blancos táctiles no pueden ir en la malla, así que son nodos. Se
  // crean una vez y se reciclan: en una pasada de layout se piden los que hagan
  // falta y los sobrantes se estacionan fuera de vista.
  //
  // Estacionar en vez de borrar es a propósito. `remove()` seguido de
  // `createElement` en el mismo cuadro es trabajo puro para el motor, y una
  // interfaz que muestra y esconde una lista lo haría sesenta veces por segundo.
  const LEJOS = -9000;

  function Pileta(padre, etiqueta, preparar) {
    this.padre = padre;
    this.etiqueta = etiqueta;
    this.preparar = preparar;
    this.items = [];
    this.usados = 0;
  }

  Pileta.prototype.pedir = function () {
    if (this.usados < this.items.length) return this.items[this.usados++];
    const el = root.createElement(this.etiqueta);
    const item = { el: el, alTocar: null };
    if (this.preparar) this.preparar(item);
    this.padre.appendChild(el);
    this.items.push(item);
    this.usados++;
    return item;
  };

  Pileta.prototype.comenzar = function () { this.usados = 0; };

  Pileta.prototype.terminar = function () {
    for (let i = this.usados; i < this.items.length; i++) {
      const it = this.items[i];
      if (!it.estacionado) {
        it.el.setAttribute("y", String(LEJOS));
        it.alTocar = null;
        // Al estacionar, el dueño deja de estar señalado: si no, un control que
        // dejó de dibujarse mientras el puntero estaba encima se quedaría
        // resaltado para siempre, porque su `pointerleave` nunca va a llegar.
        if (it.duenio) it.duenio.encima = false;
        it.duenio = null;
        it.estacionado = true;
      }
    }
  };

  // ──────────────────────────────────────────────────────────────────────────
  // Enlaces de datos
  // ──────────────────────────────────────────────────────────────────────────
  // La marca de la casa de WPF: el marcado no dice el valor, dice **de dónde
  // sacarlo**.
  //
  //   <TextBlock Text="{Binding titulo}"/>
  //   <Slider Value="{Binding potencia, Mode=TwoWay}"/>
  //
  // Sin esto, cambiar un número obliga a rearmar el árbol entero —que en este
  // framework es barato, pero pierde el estado de los controles y es la clase de
  // cosa que se nota cuando la interfaz crece.
  //
  // Es deliberadamente chico: una ruta con puntos, dos modos y un formato. No
  // hay convertidores, ni enlaces a elementos, ni notificación por propiedad. El
  // ciclo se cierra invalidando, no observando: es más tosco y es mucho menos
  // código que puede estar mal.
  const RE_ENLACE = /^\{Binding\s*([^,}]*)((?:,[^}]*)*)\}$/;

  function leerEnlace(texto) {
    const m = RE_ENLACE.exec(String(texto).trim());
    if (!m) return null;
    const enlace = { ruta: m[1].trim(), modo: "OneWay", formato: null };
    const extras = (m[2] || "").split(",");
    for (let i = 0; i < extras.length; i++) {
      const par = extras[i].split("=");
      if (par.length !== 2) continue;
      const k = par[0].trim(), v = par[1].trim();
      if (k === "Mode") enlace.modo = v;
      else if (k === "Format") enlace.formato = v;
    }
    return enlace;
  }

  /** Leer "a.b.c" de un objeto. Una ruta vacía es el objeto entero, como el
   *  `{Binding}` pelado de WPF. */
  function leerRuta(obj, ruta) {
    if (!ruta) return obj;
    const partes = ruta.split(".");
    let v = obj;
    for (let i = 0; i < partes.length && v != null; i++) v = v[partes[i]];
    return v;
  }

  function escribirRuta(obj, ruta, valor) {
    if (!ruta || obj == null) return;
    const partes = ruta.split(".");
    let o = obj;
    for (let i = 0; i < partes.length - 1; i++) {
      if (o[partes[i]] == null) o[partes[i]] = {};
      o = o[partes[i]];
    }
    o[partes[partes.length - 1]] = valor;
  }

  /** `Format` con un solo hueco. Lo mínimo para no tener que concatenar en el
   *  código lo que en realidad es presentación:
   *
   *      <TextBlock Text="{Binding potencia, Format=%v %}"/>
   *
   *  El hueco es `%v` y no `{0}` como en WPF por una razón boba pero real: la
   *  expresión de enlace termina en la primera llave de cierre, así que un
   *  `{0}` adentro la cortaría por la mitad. Se acepta igual por si alguien lo
   *  escribe por costumbre y el enlace no lleva más opciones. */
  function formatear(valor, formato) {
    if (!formato) return valor;
    return String(formato).replace("%v", String(valor)).replace("{0}", String(valor));
  }

  // ──────────────────────────────────────────────────────────────────────────
  // El elemento base
  // ──────────────────────────────────────────────────────────────────────────
  // `Measure` devuelve el tamaño deseado **incluyendo el margen**, y `Arrange`
  // recibe el rectángulo con el margen adentro: es la convención de WPF y evita
  // que cada panel tenga que acordarse de sumar o restar.

  function Elemento(atributos) {
    const a = atributos || {};
    this.attr = a;
    this.hijos = [];
    this.padre = null;
    this.nombre = a.Name || null;
    this.renderPanel = a.RenderPanel === "true";
    this.depth = Math.max(0,Math.min(1,numero(a.Depth,0)));
    this.elevation = Math.max(-1,Math.min(1,numero(a.Elevation,0)));

    this.margen = grosor(a.Margin);
    /** El globo de ayuda. Vive en Elemento y no en Interactivo porque el
     *  atributo se declara en el marcado sobre cualquier cosa; quien decide si
     *  se muestra es el hover, que solo tienen los interactivos. */
    this.ayuda = a.ToolTip || null;
    this.anchoPedido = numero(a.Width, NaN);
    this.altoPedido = numero(a.Height, NaN);
    this.anchoMin = numero(a.MinWidth, 0);
    this.altoMin = numero(a.MinHeight, 0);
    this.anchoMax = numero(a.MaxWidth, Infinity);
    this.altoMax = numero(a.MaxHeight, Infinity);
    this.alinH = a.HorizontalAlignment || "Stretch";
    this.alinV = a.VerticalAlignment || "Stretch";
    this.visible = (a.Visibility || "Visible") !== "Collapsed";
    this.opaco = (a.Visibility || "Visible") === "Visible";

    /** Los enlaces declarados en los atributos, ya leídos. */
    this.enlaces = null;
    for (const k in a) {
      const e = leerEnlace(a[k]);
      if (e) (this.enlaces || (this.enlaces = [])).push({ prop: k, e: e });
    }

    /** Lo que llena `Measure` y consume `Arrange`. */
    this.deseado = { w: 0, h: 0 };
    /** Lo que llena `Arrange` y consume `emitir`. */
    this.caja = { x: 0, y: 0, w: 0, h: 0 };
  }

  /** De atributo de XAML al campo de JavaScript. Cada control declara el suyo;
   *  sin esto un enlace tendría que saber cómo se llama por dentro cada cosa. */
  Elemento.prototype.mapa = {};

  /** Traer los valores enlazados desde el contexto de datos. Se corre antes de
   *  medir, así que el layout ya ve los valores nuevos en la misma pasada. */
  Elemento.prototype.aplicarEnlaces = function (datos, app) {
    // La aplicación baja con los datos y no se guarda al inflar: un mismo árbol
    // podría montarse en dos aplicaciones, y sobre todo las copias que crea un
    // ItemsControl no existen todavía cuando se infla.
    this.app = app;
    if (this.enlaces) {
      for (let i = 0; i < this.enlaces.length; i++) {
        const en = this.enlaces[i];
        const campo = this.mapa[en.prop] || ({Depth:"depth",Elevation:"elevation"})[en.prop];
        if (!campo) continue;
        let v = leerRuta(datos, en.e.ruta);
        if (v === undefined) continue;
        if (en.e.formato) v = formatear(v, en.e.formato);
        this[campo] = v;
      }
    }
    for (let i = 0; i < this.hijos.length; i++) {
      this.hijos[i].aplicarEnlaces(datos, app);
    }
  };

  /** Devolver un valor al origen. Sólo para los enlaces TwoWay, que son los que
   *  tienen sentido en un control que el visitante puede mover. */
  Elemento.prototype.devolver = function (datos, prop, valor) {
    if (!this.enlaces) return false;
    for (let i = 0; i < this.enlaces.length; i++) {
      const en = this.enlaces[i];
      if (en.prop === prop && en.e.modo === "TwoWay") {
        escribirRuta(datos, en.e.ruta, valor);
        return true;
      }
    }
    return false;
  };

  Elemento.prototype.agregar = function (hijo) {
    hijo.padre = this;
    this.hijos.push(hijo);
    return this;
  };

  /** Restringe una medida a lo pedido por Width/MinWidth/MaxWidth. */
  Elemento.prototype.limitar = function (w, h) {
    let nw = isFinite(this.anchoPedido) ? this.anchoPedido : w;
    let nh = isFinite(this.altoPedido) ? this.altoPedido : h;
    nw = Math.max(this.anchoMin, Math.min(this.anchoMax, nw));
    nh = Math.max(this.altoMin, Math.min(this.altoMax, nh));
    return { w: nw, h: nh };
  };

  Elemento.prototype.medir = function (dispW, dispH) {
    if (!this.visible) { this.deseado = { w: 0, h: 0 }; return this.deseado; }
    const mw = anchoDe(this.margen), mh = altoDe(this.margen);
    const c = this.limitar(Math.max(0, dispW - mw), Math.max(0, dispH - mh));
    const propio = this.medirContenido(c.w, c.h);
    const f = this.limitar(propio.w, propio.h);
    this.deseado = { w: f.w + mw, h: f.h + mh };
    return this.deseado;
  };

  /** Lo que cada elemento redefine: cuánto mide su contenido. */
  Elemento.prototype.medirContenido = function () { return { w: 0, h: 0 }; };

  Elemento.prototype.acomodar = function (x, y, w, h) {
    if (!this.visible) { this.caja = { x: x, y: y, w: 0, h: 0 }; return; }
    const m = this.margen;
    let cx = x + m.i, cy = y + m.d;
    let cw = Math.max(0, w - anchoDe(m)), ch = Math.max(0, h - altoDe(m));

    // La alineación decide si el elemento estira o se planta en su tamaño.
    const d = this.limitar(
      isFinite(this.anchoPedido) ? this.anchoPedido : this.deseado.w - anchoDe(m),
      isFinite(this.altoPedido) ? this.altoPedido : this.deseado.h - altoDe(m));

    if (this.alinH !== "Stretch" || isFinite(this.anchoPedido)) {
      const aw = Math.min(cw, d.w);
      if (this.alinH === "Center") cx += (cw - aw) / 2;
      else if (this.alinH === "Right") cx += cw - aw;
      cw = aw;
    }
    if (this.alinV !== "Stretch" || isFinite(this.altoPedido)) {
      const ah = Math.min(ch, d.h);
      if (this.alinV === "Center") cy += (ch - ah) / 2;
      else if (this.alinV === "Bottom") cy += ch - ah;
      ch = ah;
    }

    this.caja = { x: cx, y: cy, w: cw, h: ch };
    this.acomodarContenido(cx, cy, cw, ch);
  };

  Elemento.prototype.acomodarContenido = function () {};

  Elemento.prototype.emitir = function (ctx) {
    if (!this.visible || !this.opaco) return;
    const self=this;
    const depth=Math.max(0,Math.min(1,numero(this.depth,0)));
    const lift=Math.max(-1,Math.min(1,numero(this.elevation,0)))+depth;
    const baseZ=ctx.g.z;
    ctx.g.z=baseZ+lift;
    function paint(){
      const b=self.caja;
      if(depth>0 && (self.fondo || self.brocha)) {
        ctx.g.solidRect(b.x,b.y,b.w,b.h,self.radio||0,depth,self.fondo||self.brocha);
        ctx.subir();
      }
      self.emitirContenido(ctx);
    }
    try {
      if(this.renderPanel && ctx.app && ctx.app._renderPanel)ctx.app._renderPanel(this,ctx,paint);
      else paint();
    } finally {ctx.g.z=baseZ;}

  };

  Elemento.prototype.emitirContenido = function(ctx) {
    this.dibujar(ctx);for(const child of this.hijos)child.emitir(ctx);
  };
  Elemento.prototype.invalidateRender = function () {if(this.app)this.app.invalidateRender(this);};
  Elemento.prototype.invalidateMeasure = function () {if(this.app)this.app.invalidateMeasure();};
  Elemento.prototype.invalidateArrange = function () {if(this.app)this.app.invalidateArrange();};

  Elemento.prototype.dibujar = function () {};

  /** El dato de la fila a la que pertenece este elemento, si está adentro de la
   *  plantilla de un ItemsControl. Sube por los padres hasta la copia, que es la
   *  que lo tiene: el que recibe el toque suele ser un botón enterrado tres
   *  niveles más abajo.
   *
   *  Es lo que en WPF resuelve el DataContext heredado. Acá el contexto no se
   *  hereda —los enlaces se aplican de arriba hacia abajo y no se guardan—, así
   *  que se busca hacia arriba en vez de propagarse hacia abajo. */
  Elemento.prototype.itemDeLista = function () {
    let el = this;
    while (el) {
      if (el.datosItem !== undefined) return el.datosItem;
      el = el.padre;
    }
    return null;
  };

  /** Buscar por `Name`, como `FindName` de WPF. */
  Elemento.prototype.buscar = function (nombre) {
    if (this.nombre === nombre) return this;
    for (let i = 0; i < this.hijos.length; i++) {
      const r = this.hijos[i].buscar(nombre);
      if (r) return r;
    }
    return null;
  };

  function heredar(Hijo, Padre) {
    Hijo.prototype = Object.create(Padre.prototype);
    Hijo.prototype.constructor = Hijo;
    return Hijo;
  }

  globalThis.UI = {
    rgba: rgba,
    aLineal: aLineal,
    aSRGB: aSRGB,
    tinte: tinte,
    grosor: grosor,
    numero: numero,
    booleano: booleano,
    leerMarcado: leerMarcado,
    leerEnlace: leerEnlace,
    leerRuta: leerRuta,
    escribirRuta: escribirRuta,
    formatear: formatear,
    Geometria: Geometria,
    roundedCommands,roundedGeometry,
    Pileta: Pileta,
    Elemento: Elemento,
    heredar: heredar,
    anchoDe: anchoDe,
    altoDe: altoDe,
    LEJOS: LEJOS,
    root: root,
    CFG: CFG,
    tipos: {},
  };
})();


// ──────────────────────────────────────────────────────────────────────
// controles.js
// ──────────────────────────────────────────────────────────────────────
// controles.js — los elementos del framework.
//
// Cada uno implementa como mucho tres cosas:
//
//   medirContenido(dispW, dispH) → {w, h}   cuánto querría medir
//   acomodarContenido(x, y, w, h)           dónde van sus hijos
//   dibujar(ctx)                            qué emite
//
// El resto —márgenes, alineación, Width/Height, Min/Max— lo resuelve `Elemento`
// una sola vez para todos. Es la parte de WPF que más rinde copiar: los paneles
// terminan siendo diez líneas porque no repiten nada.
(function () {
  "use strict";

  const U = globalThis.UI;
  const heredar = U.heredar;
  const num = U.numero;
  const bool = U.booleano;
  const grosor = U.grosor;
  const anchoDe = U.anchoDe;
  const altoDe = U.altoDe;

  const C = (U.CFG && U.CFG.C) || {};

  // Shared native layout: measurement and glyph drawing consume the same result.
  const layouts = new Map();
  function medirTexto(texto, tam, ancho) {
    const text = String(texto === undefined || texto === null ? "" : texto);
    const key = JSON.stringify([text,tam,ancho || 0]);
    if (layouts.has(key)) return layouts.get(key);
    const result = globalThis.TextLayout
      ? TextLayout.create(text,tam,ancho || 0)
      : {w:text.length*tam*0.6,h:tam}; // Compatibility with older engine builds.
    if (layouts.size >= 256) layouts.delete(layouts.keys().next().value);
    layouts.set(key,result);
    return result;
  }
  U.medirTexto = medirTexto;

  // ── TextBlock ─────────────────────────────────────────────────────────────
  function TextBlock(a) {
    U.Elemento.call(this, a);
    this.texto = a.Text !== undefined ? a.Text : (a.__texto || "");
    this.tam = num(a.FontSize, 0.05);
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.alineacion = a.TextAlignment || "Left";
    this.wrap = a.TextWrapping === "Wrap";
  }
  heredar(TextBlock, U.Elemento);

  TextBlock.prototype.medirContenido = function (width) {
    this.textLayout=medirTexto(this.texto,this.tam,this.wrap && Number.isFinite(width) && width>0 ? width : 0);
    return this.textLayout;
  };

  TextBlock.prototype.dibujar = function (ctx) {
    if (this.texto === "" || this.texto === null) return;
    const b = this.caja;
    const m = this.textLayout || medirTexto(this.texto, this.tam);
    // El `<text>` del motor se centra en su punto, así que alinear a izquierda
    // es correr medio ancho estimado. De ahí que la estimación importe.
    let cx = b.x + b.w / 2;
    if (this.alineacion === "Left") cx = b.x + m.w / 2;
    else if (this.alineacion === "Right") cx = b.x + b.w - m.w / 2;
    ctx.texto(cx, b.y + b.h / 2, this.texto, this.tam, this.color,m);
  };
  /** Qué atributo de XAML escribe qué campo. Es lo único que un enlace necesita
   *  saber de un control. */
  TextBlock.prototype.mapa = {
    Text: "texto", FontSize: "tam", Foreground: "color",
    TextAlignment: "alineacion",
  };
  U.tipos.TextBlock = TextBlock;

  // ── Border ────────────────────────────────────────────────────────────────
  // El decorador de WPF: un hijo, un fondo, un marco y un relleno interno.
  // Casi todos los controles con aspecto se arman con esto adentro.
  function Border(a) {
    U.Elemento.call(this, a);
    this.fondo = a.Background || null;
    this.brocha = a.BorderBrush || null;
    this.grosorBorde = grosor(a.BorderThickness);
    this.radio = num(a.CornerRadius, 0);
    this.relleno = grosor(a.Padding);
  }
  heredar(Border, U.Elemento);

  Border.prototype.interior = function () {
    const g = this.grosorBorde, p = this.relleno;
    return {
      i: g.i + p.i, d: g.d + p.d, s: g.s + p.s, f: g.f + p.f,
    };
  };

  Border.prototype.medirContenido = function (w, h) {
    const t = this.interior();
    const hw = anchoDe(t), hh = altoDe(t);
    let mw = 0, mh = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      const d = this.hijos[i].medir(Math.max(0, w - hw), Math.max(0, h - hh));
      mw = Math.max(mw, d.w);
      mh = Math.max(mh, d.h);
    }
    return { w: mw + hw, h: mh + hh };
  };

  Border.prototype.acomodarContenido = function (x, y, w, h) {
    const t = this.interior();
    const ix = x + t.i, iy = y + t.d;
    const iw = Math.max(0, w - anchoDe(t)), ih = Math.max(0, h - altoDe(t));
    for (let i = 0; i < this.hijos.length; i++) this.hijos[i].acomodar(ix, iy, iw, ih);
  };

  Border.prototype.dibujar = function (ctx) {
    const b = this.caja;
    if (this.brocha && (this.grosorBorde.i || this.grosorBorde.d ||
                        this.grosorBorde.s || this.grosorBorde.f)) {
      if (this.radio > 0) ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, this.radio, this.brocha);
      else ctx.g.marco(b.x, b.y, b.w, b.h, this.grosorBorde, this.brocha);
    }
    if (this.fondo) {
      const g = this.brocha ? this.grosorBorde : { i: 0, d: 0, s: 0, f: 0 };
      const x = b.x + g.i, y = b.y + g.d;
      const w = b.w - anchoDe(g), h = b.h - altoDe(g);
      ctx.subir();
      if (this.radio > 0) {
        ctx.g.rectRedondeado(x, y, w, h, Math.max(0, this.radio - g.i), this.fondo);
      } else {
        ctx.g.rect(x, y, w, h, this.fondo);
      }
    }
    ctx.subir();
  };
  Border.prototype.mapa = {
    Background: "fondo", BorderBrush: "brocha", CornerRadius: "radio",
  };
  U.tipos.Border = Border;

  // ── StackPanel ────────────────────────────────────────────────────────────
  function StackPanel(a) {
    U.Elemento.call(this, a);
    this.horizontal = (a.Orientation || "Vertical") === "Horizontal";
    this.espaciado = num(a.Spacing, 0);
    this.fondo = a.Background || null;
  }
  heredar(StackPanel, U.Elemento);

  StackPanel.prototype.medirContenido = function (w, h) {
    let a = 0, t = 0, n = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      // En el eje del apilado el espacio es infinito: eso es lo que hace que un
      // StackPanel no comprima a sus hijos, y es la diferencia con un Grid.
      const d = this.horizontal ? c.medir(Infinity, h) : c.medir(w, Infinity);
      if (this.horizontal) { t += d.w; a = Math.max(a, d.h); }
      else { t += d.h; a = Math.max(a, d.w); }
      n++;
    }
    if (n > 1) t += this.espaciado * (n - 1);
    return this.horizontal ? { w: t, h: a } : { w: a, h: t };
  };

  StackPanel.prototype.acomodarContenido = function (x, y, w, h) {
    let p = this.horizontal ? x : y;
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      if (this.horizontal) {
        c.acomodar(p, y, c.deseado.w, h);
        p += c.deseado.w + this.espaciado;
      } else {
        c.acomodar(x, p, w, c.deseado.h);
        p += c.deseado.h + this.espaciado;
      }
    }
  };

  StackPanel.prototype.dibujar = function (ctx) {
    if (this.fondo) {
      const b = this.caja;
      ctx.g.rect(b.x, b.y, b.w, b.h, this.fondo);
      ctx.subir();
    }
  };
  StackPanel.prototype.mapa = { Background: "fondo", Spacing: "espaciado" };
  U.tipos.StackPanel = StackPanel;

  // ── Grid ──────────────────────────────────────────────────────────────────
  // Filas y columnas con las tres formas de WPF:
  //   0.2   fijo, en metros
  //   Auto  lo que pida el contenido
  //   *     reparte lo que sobra;  2*  pesa el doble
  //
  // Se declara compacto —`ColumnDefinitions="Auto,*,0.2"`— en vez de con los
  // elementos anidados de XAML: es la misma información sin cuatro líneas de
  // ceremonia por columna.
  function leerDefiniciones(txt) {
    const out = [];
    const partes = String(txt || "*").split(/[,\s]+/).filter(function (s) { return s; });
    for (let i = 0; i < partes.length; i++) {
      const p = partes[i];
      if (p === "Auto" || p === "auto") out.push({ tipo: "auto", peso: 0, medida: 0 });
      else if (p.endsWith("*")) {
        const w = p.length === 1 ? 1 : Number(p.slice(0, -1));
        out.push({ tipo: "estrella", peso: isFinite(w) ? w : 1, medida: 0 });
      } else {
        const v = Number(p);
        out.push({ tipo: "fijo", peso: 0, medida: isFinite(v) ? v : 0 });
      }
    }
    return out;
  }

  function Grid(a) {
    U.Elemento.call(this, a);
    this.cols = leerDefiniciones(a.ColumnDefinitions || "*");
    this.filas = leerDefiniciones(a.RowDefinitions || "*");
    this.sepCol = num(a.ColumnSpacing, 0);
    this.sepFila = num(a.RowSpacing, 0);
    this.fondo = a.Background || null;
  }
  heredar(Grid, U.Elemento);

  function celdaDe(hijo) {
    return {
      f: Math.max(0, Math.trunc(num(hijo.attr["Grid.Row"], 0))),
      c: Math.max(0, Math.trunc(num(hijo.attr["Grid.Column"], 0))),
      df: Math.max(1, Math.trunc(num(hijo.attr["Grid.RowSpan"], 1))),
      dc: Math.max(1, Math.trunc(num(hijo.attr["Grid.ColumnSpan"], 1))),
    };
  }

  /** Reparto de una dimensión. Fijos primero, después Auto con lo que pidan sus
   *  hijos, y lo que sobra se divide entre las estrellas según su peso. Es
   *  exactamente el orden de WPF, y el orden importa: si las estrellas se
   *  resolvieran antes, los Auto no tendrían contra qué medirse. */
  function repartir(defs, total, sep, pedidos) {
    const n = defs.length;
    const tam = new Array(n);
    let usado = sep * Math.max(0, n - 1);
    let pesos = 0;

    for (let i = 0; i < n; i++) {
      const d = defs[i];
      if (d.tipo === "fijo") { tam[i] = d.medida; usado += tam[i]; }
      else if (d.tipo === "auto") { tam[i] = pedidos[i] || 0; usado += tam[i]; }
      else { tam[i] = 0; pesos += d.peso; }
    }
    const sobra = Math.max(0, total - usado);
    if (pesos > 0) {
      for (let i = 0; i < n; i++) {
        if (defs[i].tipo === "estrella") tam[i] = (sobra * defs[i].peso) / pesos;
      }
    }
    return tam;
  }

  Grid.prototype.medirContenido = function (w, h) {
    const pedidoCol = new Array(this.cols.length).fill(0);
    const pedidoFila = new Array(this.filas.length).fill(0);

    // Primera vuelta: sólo para saber cuánto piden los Auto. Los que ocupan más
    // de una celda no aportan —repartir su pedido entre celdas es ambiguo y WPF
    // tampoco lo hace de forma útil.
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      const p = celdaDe(c);
      const d = c.medir(Infinity, Infinity);
      if (p.dc === 1 && p.c < pedidoCol.length) {
        pedidoCol[p.c] = Math.max(pedidoCol[p.c], d.w);
      }
      if (p.df === 1 && p.f < pedidoFila.length) {
        pedidoFila[p.f] = Math.max(pedidoFila[p.f], d.h);
      }
    }

    // **Lo que la grilla pide no es lo que la grilla ocupa.**
    //
    // Una fila `*` ocupa lo que sobre, pero *pedir* todo lo disponible es otra
    // cosa: metida arriba de un DockPanel, una grilla con una fila `*` se comía
    // el panel entero y no quedaba lugar para nada más. Pasó, y se veía como si
    // el encabezado hubiera devorado la lista.
    //
    // Así que para el tamaño deseado, una `*` vale **lo que pide su contenido**,
    // igual que una Auto. El reparto de verdad —donde `*` se queda con el
    // sobrante— pasa en `acomodarContenido`, con el ancho definitivo.
    let tw = this.sepCol * Math.max(0, this.cols.length - 1);
    let th = this.sepFila * Math.max(0, this.filas.length - 1);
    for (let i = 0; i < this.cols.length; i++) {
      tw += this.cols[i].tipo === "fijo" ? this.cols[i].medida : (pedidoCol[i] || 0);
    }
    for (let i = 0; i < this.filas.length; i++) {
      th += this.filas[i].tipo === "fijo" ? this.filas[i].medida : (pedidoFila[i] || 0);
    }
    return { w: tw, h: th };
  };

  Grid.prototype.acomodarContenido = function (x, y, w, h) {
    // Rehacer el reparto con el tamaño definitivo: en `medir` el disponible pudo
    // ser Infinity —dentro de un StackPanel lo es— y las estrellas habrían
    // quedado en cero.
    const pedidoCol = new Array(this.cols.length).fill(0);
    const pedidoFila = new Array(this.filas.length).fill(0);
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      const p = celdaDe(c);
      if (p.dc === 1 && p.c < pedidoCol.length) {
        pedidoCol[p.c] = Math.max(pedidoCol[p.c], c.deseado.w);
      }
      if (p.df === 1 && p.f < pedidoFila.length) {
        pedidoFila[p.f] = Math.max(pedidoFila[p.f], c.deseado.h);
      }
    }
    const anchos = repartir(this.cols, w, this.sepCol, pedidoCol);
    const altos = repartir(this.filas, h, this.sepFila, pedidoFila);

    const xs = [], ys = [];
    let px = x;
    for (let i = 0; i < anchos.length; i++) { xs.push(px); px += anchos[i] + this.sepCol; }
    let py = y;
    for (let i = 0; i < altos.length; i++) { ys.push(py); py += altos[i] + this.sepFila; }

    this.anchoCols = anchos;
    this.altoFilas = altos;
    this.xs = xs;
    this.ys = ys;

    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      const p = celdaDe(c);
      const ci = Math.min(p.c, anchos.length - 1);
      const fi = Math.min(p.f, altos.length - 1);
      let cw = 0, ch = 0;
      for (let k = 0; k < p.dc && ci + k < anchos.length; k++) {
        cw += anchos[ci + k] + (k > 0 ? this.sepCol : 0);
      }
      for (let k = 0; k < p.df && fi + k < altos.length; k++) {
        ch += altos[fi + k] + (k > 0 ? this.sepFila : 0);
      }
      c.acomodar(xs[ci] || x, ys[fi] || y, cw, ch);
    }
  };

  Grid.prototype.dibujar = function (ctx) {
    if (this.fondo) {
      const b = this.caja;
      ctx.g.rect(b.x, b.y, b.w, b.h, this.fondo);
      ctx.subir();
    }
  };
  Grid.prototype.mapa = { Background: "fondo" };
  U.tipos.Grid = Grid;

  // ── Canvas ────────────────────────────────────────────────────────────────
  function Canvas(a) {
    U.Elemento.call(this, a);
    this.fondo = a.Background || null;
  }
  heredar(Canvas, U.Elemento);

  Canvas.prototype.medirContenido = function () {
    for (let i = 0; i < this.hijos.length; i++) this.hijos[i].medir(Infinity, Infinity);
    return { w: 0, h: 0 };
  };

  Canvas.prototype.acomodarContenido = function (x, y) {
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      c.acomodar(x + num(c.attr["Canvas.Left"], 0), y + num(c.attr["Canvas.Top"], 0),
                 c.deseado.w, c.deseado.h);
    }
  };

  Canvas.prototype.dibujar = function (ctx) {
    if (this.fondo) {
      const b = this.caja;
      ctx.g.rect(b.x, b.y, b.w, b.h, this.fondo);
      ctx.subir();
    }
  };
  U.tipos.Canvas = Canvas;

  U.leerDefiniciones = leerDefiniciones;
  U.repartir = repartir;
  U.celdaDe = celdaDe;
  function RenderPanel(a){Border.call(this,a);this.renderPanel=true;}
  heredar(RenderPanel,Border);U.tipos.RenderPanel=RenderPanel;

  function readPath(data){
    if(typeof data!=="string"||data.length>16384)throw new TypeError("Path Data exceeds 16 KiB");
    const re=/[MLQCZmlqcz]|[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?/g;
    const tokens=[];let end=0,m;
    while((m=re.exec(data))){if(!/^[\s,]*$/.test(data.slice(end,m.index)))throw Error("Path supports M L Q C Z commands");tokens.push(m[0]);end=re.lastIndex;}
    if(!/^[\s,]*$/.test(data.slice(end)))throw Error("Invalid Path Data");
    const out=[];let i=0,command=null,x=0,y=0,startX=0,startY=0;
    while(i<tokens.length){
      if(/^[a-z]$/i.test(tokens[i]))command=tokens[i++];
      if(!command)throw Error("Path requires a command");
      const upper=command.toUpperCase(),relative=command!==upper,count={M:2,L:2,Q:4,C:6,Z:0}[upper];
      if(upper==="Z"){out.push({op:"close"});x=startX;y=startY;command=null;}
      else {
        const n=tokens.slice(i,i+count).map(Number);if(n.length!==count||n.some(v=>!Number.isFinite(v)))throw Error("Incomplete Path command");i+=count;
        if(relative)for(let j=0;j<n.length;j+=2){n[j]+=x;n[j+1]+=y;}
        x=n[n.length-2];y=n[n.length-1];
        if(upper==="M"){out.push({op:"moveTo",x,y});startX=x;startY=y;command=relative?"l":"L";}
        else if(upper==="L")out.push({op:"lineTo",x,y});
        else if(upper==="Q")out.push({op:"quadraticTo",cx:n[0],cy:n[1],x,y});
        else out.push({op:"bezierTo",c1x:n[0],c1y:n[1],c2x:n[2],c2y:n[3],x,y});
      }
      if(out.length>128)throw Error("Path exceeds 128 commands");
    }
    return out;
  }
  function Path(a){
    U.Elemento.call(this,a);this.data=a.Data||"";this.fill=a.Fill||null;this.stroke=a.Stroke||null;
    this.strokeWidth=num(a.StrokeThickness,0.002);this.tolerance=num(a.Tolerance,0.00005);
    this.nonZero=a.FillRule==="NonZero";this.stretch=a.Stretch||"Uniform";
  }
  heredar(Path,U.Elemento);
  Path.prototype.commands=function(){return readPath(this.data);};
  Path.prototype.prepare=function(){
    if(!globalThis.PathGeometry)throw Error("Path/Ellipse require a Luna build with PathGeometry");
    const commands=this.commands(),key=JSON.stringify([commands,this.strokeWidth,this.tolerance,this.nonZero,!!this.fill,!!this.stroke]);
    if(key===this._pathKey)return;
    this._fill=this.fill?PathGeometry.tessellate(commands,{tolerance:this.tolerance,nonZero:this.nonZero}):null;
    this._stroke=this.stroke&&this.strokeWidth>0?PathGeometry.tessellate(commands,{tolerance:this.tolerance,strokeWidth:this.strokeWidth}):null;
    const points=[...(this._fill?this._fill.positions:[]),...(this._stroke?this._stroke.positions:[])];
    let x=Infinity,y=Infinity,x1=-Infinity,y1=-Infinity;
    for(const p of points){x=Math.min(x,p[0]);y=Math.min(y,p[1]);x1=Math.max(x1,p[0]);y1=Math.max(y1,p[1]);}
    this._bounds=points.length?{x,y,w:x1-x,h:y1-y}:{x:0,y:0,w:0,h:0};
    this._pathKey=key;
  };
  Path.prototype.medirContenido=function(){this.prepare();return this._bounds;};
  Path.prototype.dibujar=function(ctx){
    this.prepare();const b=this.caja,n=this._bounds;if(!(n.w>0&&n.h>0))return;
    let sx=this.stretch==="None"?1:b.w/n.w,sy=this.stretch==="None"?1:b.h/n.h;
    if(this.stretch==="Uniform")sx=sy=Math.min(sx,sy);
    const x=b.x+(b.w-n.w*sx)/2,y=b.y+(b.h-n.h*sy)/2;
    for(const [geometry,color] of [[this._fill,this.fill],[this._stroke,this.stroke]])if(geometry){
      const transformed={indices:geometry.indices,positions:geometry.positions.map(p=>[(p[0]-n.x)*sx,(p[1]-n.y)*sy])};
      ctx.g.path(transformed,x,y,color);ctx.subir();
    }
  };
  Path.prototype.mapa={Data:"data",Fill:"fill",Stroke:"stroke",StrokeThickness:"strokeWidth"};
  function Ellipse(a){Path.call(this,a);}
  heredar(Ellipse,Path);
  Ellipse.prototype.commands=function(){
    const w=Number.isFinite(this.anchoPedido)?this.anchoPedido:0.1,h=Number.isFinite(this.altoPedido)?this.altoPedido:0.1;
    const x=w/2,y=h/2,k=0.5522847498307936;
    return [{op:"moveTo",x:w,y},
      {op:"bezierTo",c1x:w,c1y:y+y*k,c2x:x+x*k,c2y:h,x,y:h},
      {op:"bezierTo",c1x:x-x*k,c1y:h,c2x:0,c2y:y+y*k,x:0,y},
      {op:"bezierTo",c1x:0,c1y:y-y*k,c2x:x-x*k,c2y:0,x,y:0},
      {op:"bezierTo",c1x:x+x*k,c1y:0,c2x:w,c2y:y-y*k,x:w,y},{op:"close"}];
  };
  U.tipos.Path=Path;U.tipos.Ellipse=Ellipse;U.readPath=readPath;
})();


// ──────────────────────────────────────────────────────────────────────
// interactivos.js
// ──────────────────────────────────────────────────────────────────────
// interactivos.js — los controles que responden al toque.
//
// No hay hover en el motor: sólo llega `toque`, y llega **con el punto de
// impacto en coordenadas del mundo** (x, y, z). Eso decide el lenguaje de
// interacción de todo el framework:
//
//   - no existe el estado "el puntero está encima", así que ningún control
//     puede depender de él;
//   - sí existe "acaba de ser tocado", que dura lo que uno quiera;
//   - y existe la posición, así que un control puede saber *dónde* lo tocaron.
//     Sin eso, un Slider sería dos botones de más y menos.
//
// El destello al presionar reemplaza al hover. Sin realimentación, un botón
// flotando en el aire no se siente apretado, y el destello es la única señal
// disponible.
(function () {
  "use strict";

  const U = globalThis.UI;
  const heredar = U.heredar;
  const num = U.numero;
  const bool = U.booleano;
  const grosor = U.grosor;
  const anchoDe = U.anchoDe;
  const altoDe = U.altoDe;
  const medirTexto = U.medirTexto;
  const C = (U.CFG && U.CFG.C) || {};

  const MS_DESTELLO = 170;

  /** Base de todo lo que se toca. Guarda cuándo fue el último toque y expone
   *  `destello()`: cuánto de encendido está ahora mismo, de 1 a 0. */
  function Interactivo(a) {
    U.Elemento.call(this, a);
    this.habilitado = bool(a.IsEnabled, true);
    this.tocadoEn = 0;
    /** Lo pone y lo saca el host desde `pointerenter` / `pointerleave`. */
    this.encima = false;
  }
  heredar(Interactivo, U.Elemento);

  /** Cuánto hay que aclarar el fondo ahora mismo: el destello del toque, que
   *  decae, más un escalón fijo mientras el puntero esté encima.
   *
   *  Son dos señales distintas y por eso se suman en vez de elegir una: el hover
   *  dice "esto se puede tocar" y el destello dice "te escuché". */
  Interactivo.prototype.realce = function () {
    return (this.encima ? 0.18 : 0) + 0.5 * this.destello();
  };

  Interactivo.prototype.destello = function () {
    if (!this.tocadoEn) return 0;
    const t = (Date.now() - this.tocadoEn) / MS_DESTELLO;
    return t >= 1 ? 0 : 1 - t;
  };

  /** Resolver un nombre de manejador contra los que registró la aplicación.
   *  Es el Click="aceptar" de XAML: el marcado nombra, el código provee. */
  Interactivo.prototype.manejador = function (app, nombre) {
    if (!nombre) return null;
    const h = app && app.manejadores ? app.manejadores[nombre] : null;
    return typeof h === "function" ? h : null;
  };

  Interactivo.prototype.marcarToque = function (app) {
    this.tocadoEn = Date.now();
    app.animar(MS_DESTELLO + 40,this);
  };

  // ── Button ────────────────────────────────────────────────────────────────
  function Button(a) {
    Interactivo.call(this, a);
    this.contenido = a.Content !== undefined ? a.Content : (a.__texto || "");
    this.tam = num(a.FontSize, 0.034);
    this.fondo = a.Background || C.azul || "#0A84FF";
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.radio = num(a.CornerRadius, 0.012);
    this.relleno = grosor(a.Padding || "0.024,0.014");
    this.alClick = a.Click || null;
  }
  heredar(Button, Interactivo);

  Button.prototype.medirContenido = function () {
    const m = medirTexto(this.contenido, this.tam);
    return { w: m.w + anchoDe(this.relleno), h: m.h + altoDe(this.relleno) };
  };

  Button.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const d = this.destello();
    const self = this;

    // **El blanco táctil va primero, debajo de todo lo que dibuja el control.**
    // Es un plano opaco: emitido al final tapaba el relleno del Slider y la
    // casilla del CheckBox. Que quede abajo no le quita sensibilidad —el rayo
    // busca el nodo tocable más cercano y la malla no es tocable— así que el
    // orden visual y el orden de impacto son independientes.
    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, this.fondo, function (app) {
        self.marcarToque(app);
        const h = self.manejador(app, self.alClick);
        if (h) h(self, app);
      }, this);
      ctx.subir();
    }
    // El destello aclara el fondo y decae solo. El color del estado se **deriva**
    // en vez de pedirse: exigirle tres colores a quien escribe la interfaz es
    // pedirle que además sea diseñador.
    const k = this.realce();
    const fondo = !this.habilitado ? U.tinte(this.fondo, 0.45)
                : k > 0 ? U.tinte(this.fondo, 1 + k)
                : this.fondo;
    ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, this.radio, fondo);
    ctx.subir();
    ctx.texto(b.x + b.w / 2, b.y + b.h / 2, this.contenido, this.tam,
              this.habilitado ? this.color : U.tinte(this.color, 0.55));
    ctx.subir();
  };
  Button.prototype.mapa = {
    Content: "contenido", Background: "fondo", Foreground: "color",
    FontSize: "tam", IsEnabled: "habilitado",
  };
  U.tipos.Button = Button;

  // ── CheckBox ──────────────────────────────────────────────────────────────
  function CheckBox(a) {
    Interactivo.call(this, a);
    this.marcado = bool(a.IsChecked, false);
    this.contenido = a.Content !== undefined ? a.Content : (a.__texto || "");
    this.tam = num(a.FontSize, 0.032);
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.acento = a.Accent || C.verde || "#30D158";
    this.alCambiar = a.Changed || null;
    this.lado = num(a.BoxSize, 0.042);
    this.hueco = num(a.Spacing, 0.014);
  }
  heredar(CheckBox, Interactivo);

  CheckBox.prototype.medirContenido = function () {
    const m = medirTexto(this.contenido, this.tam);
    return { w: this.lado + this.hueco + m.w, h: Math.max(this.lado, m.h) };
  };

  CheckBox.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const l = this.lado;
    const cy = b.y + (b.h - l) / 2;
    const d = this.destello();
    const marco = this.marcado ? this.acento : (C.borde || "#5A5A70");
    const k = this.realce();
    const brillo = k > 0 ? U.tinte(marco, 1 + k) : marco;
    const self = this;

    // El blanco cubre la casilla **y su etiqueta**: en WPF un CheckBox se activa
    // tocando el texto, y a un metro y medio de distancia una casilla de 4 cm
    // sola sería un blanco cruel. Va primero, por lo mismo que en Button.
    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, C.superficie || "#1C1C26", function (app) {
        self.marcado = !self.marcado;
        self.devolver(app.datos, "IsChecked", self.marcado);
        self.marcarToque(app);
        const h = self.manejador(app, self.alCambiar);
        if (h) h(self, app);
        app.invalidar();
      }, this);
      ctx.subir();
    }

    ctx.g.rectRedondeado(b.x, cy, l, l, 0.008, brillo);
    ctx.subir();
    if (this.marcado) {
      // La marca es un cuadrado adentro y no un tilde: un tilde en malla son dos
      // rectángulos girados y a 3 cm no se distingue de una mancha.
      ctx.g.rectRedondeado(b.x + l * 0.26, cy + l * 0.26, l * 0.48, l * 0.48,
                           0.004, C.fondo || "#08080C");
    } else {
      ctx.g.rectRedondeado(b.x + 0.005, cy + 0.005, l - 0.01, l - 0.01,
                           0.005, C.superficie || "#1C1C26");
    }
    ctx.subir();
    if (this.contenido) {
      const m = medirTexto(this.contenido, this.tam);
      ctx.texto(b.x + l + this.hueco + m.w / 2, b.y + b.h / 2,
                this.contenido, this.tam, this.color);
    }
    ctx.subir();
  };
  CheckBox.prototype.mapa = {
    IsChecked: "marcado", Content: "contenido", Foreground: "color",
    Accent: "acento", IsEnabled: "habilitado",
  };
  U.tipos.CheckBox = CheckBox;

  // ── ProgressBar ───────────────────────────────────────────────────────────
  function ProgressBar(a) {
    U.Elemento.call(this, a);
    this.min = num(a.Minimum, 0);
    this.max = num(a.Maximum, 1);
    this.valor = num(a.Value, 0);
    this.fondo = a.Background || C.superficieAlta || "#2A2A38";
    this.relleno = a.Foreground || C.azul || "#0A84FF";
    this.radio = num(a.CornerRadius, 0.01);
  }
  heredar(ProgressBar, U.Elemento);

  ProgressBar.prototype.fraccion = function () {
    const r = this.max - this.min;
    if (!(r > 0)) return 0;
    return Math.max(0, Math.min(1, (this.valor - this.min) / r));
  };

  ProgressBar.prototype.medirContenido = function () {
    return { w: 0.2, h: 0.024 };
  };

  ProgressBar.prototype.dibujar = function (ctx) {
    const b = this.caja;
    ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, this.radio, this.fondo);
    ctx.subir();
    const w = b.w * this.fraccion();
    if (w > 0.001) {
      ctx.g.rectRedondeado(b.x, b.y, Math.max(w, this.radio * 2), b.h,
                           this.radio, this.relleno);
    }
    ctx.subir();
  };
  ProgressBar.prototype.mapa = {
    Value: "valor", Minimum: "min", Maximum: "max",
    Foreground: "relleno", Background: "fondo",
  };
  U.tipos.ProgressBar = ProgressBar;

  // ── Slider ────────────────────────────────────────────────────────────────
  // El único control que usa la posición del toque. El host traduce el punto de
  // impacto del mundo a coordenadas de la hoja y se lo pasa al manejador.
  function Slider(a) {
    Interactivo.call(this, a);
    this.min = num(a.Minimum, 0);
    this.max = num(a.Maximum, 1);
    this.valor = num(a.Value, 0);
    this.paso = num(a.Step, 0);
    this.pista = a.Background || C.superficieAlta || "#2A2A38";
    this.relleno = a.Foreground || C.azul || "#0A84FF";
    this.perilla = a.ThumbBrush || C.texto || "#FFFFFF";
    this.altoPista = num(a.TrackHeight, 0.016);
    this.ladoPerilla = num(a.ThumbSize, 0.046);
    this.alCambiar = a.ValueChanged || null;
  }
  heredar(Slider, Interactivo);

  Slider.prototype.fraccion = function () {
    const r = this.max - this.min;
    if (!(r > 0)) return 0;
    return Math.max(0, Math.min(1, (this.valor - this.min) / r));
  };

  Slider.prototype.medirContenido = function () {
    return { w: 0.28, h: Math.max(this.ladoPerilla, this.altoPista) };
  };

  Slider.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const util = Math.max(0.001, b.w - this.ladoPerilla);
    const f = this.fraccion();
    const cy = b.y + b.h / 2;
    const x0 = b.x + this.ladoPerilla / 2;
    const self = this;

    // Primero el blanco, como en los demás. Acá importa el doble: es un plano
    // opaco del ancho entero del control, y emitido al final tapaba la pista, el
    // relleno y la perilla, dejando un rectángulo gris.
    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, this.pista, function (app, hoja) {
        if (hoja) {
          const t = (hoja.x - x0) / util;
          let v = self.min + Math.max(0, Math.min(1, t)) * (self.max - self.min);
          if (self.paso > 0) v = Math.round(v / self.paso) * self.paso;
          self.valor = v;
          // TwoWay: el control mueve el dato, no sólo se pinta con él.
          self.devolver(app.datos, "Value", v);
        }
        self.marcarToque(app);
        const h = self.manejador(app, self.alCambiar);
        if (h) h(self, app);
        app.invalidar();
      }, this);
      ctx.subir();
    }

    ctx.g.rectRedondeado(x0, cy - this.altoPista / 2, util, this.altoPista,
                         this.altoPista / 2, this.pista);
    ctx.subir();
    if (f > 0) {
      ctx.g.rectRedondeado(x0, cy - this.altoPista / 2,
                           Math.max(util * f, this.altoPista), this.altoPista,
                           this.altoPista / 2, this.relleno);
    }
    ctx.subir();

    // La perilla crece con el hover en vez de sólo cambiar de color: en un
    // deslizador, lo que uno quiere saber antes de tocar es dónde está el
    // agarre, y el tamaño lo dice mejor que el brillo.
    const d = this.destello();
    const lp = this.ladoPerilla * (this.encima ? 1.18 : 1);
    ctx.g.rectRedondeado(x0 + util * f - lp / 2, cy - lp / 2, lp, lp, lp / 2,
                         d > 0 ? U.tinte(this.perilla, 1 - 0.3 * d) : this.perilla);
    ctx.subir();
  };
  Slider.prototype.mapa = {
    Value: "valor", Minimum: "min", Maximum: "max",
    Foreground: "relleno", IsEnabled: "habilitado",
  };
  U.tipos.Slider = Slider;

  U.Interactivo = Interactivo;
  U.MS_DESTELLO = MS_DESTELLO;
})();


// ──────────────────────────────────────────────────────────────────────
// paneles.js
// ──────────────────────────────────────────────────────────────────────
// paneles.js — los contenedores que faltaban, y las listas.
//
// WrapPanel y DockPanel son dos formas más de repartir espacio; ItemsControl y
// ScrollViewer son otra cosa: son los que hacen que una interfaz pueda mostrar
// **datos** y no sólo controles.
(function () {
  "use strict";

  const U = globalThis.UI;
  const heredar = U.heredar;
  const num = U.numero;
  const grosor = U.grosor;
  const anchoDe = U.anchoDe;
  const altoDe = U.altoDe;
  const C = (U.CFG && U.CFG.C) || {};

  // ── WrapPanel ─────────────────────────────────────────────────────────────
  // Apila en una dirección y salta de renglón cuando no entra. La diferencia
  // con StackPanel es una sola línea de lógica y un mundo de uso: es el panel de
  // las etiquetas, los chips y las galerías.
  function WrapPanel(a) {
    U.Elemento.call(this, a);
    this.horizontal = (a.Orientation || "Horizontal") === "Horizontal";
    this.sepX = num(a.ColumnSpacing, num(a.Spacing, 0));
    this.sepY = num(a.RowSpacing, num(a.Spacing, 0));
    this.fondo = a.Background || null;
  }
  heredar(WrapPanel, U.Elemento);

  /** El acomodo se calcula igual al medir y al acomodar; la única diferencia es
   *  si se escribe el resultado. Tenerlo una vez evita que las dos versiones se
   *  separen con el tiempo, que es como se rompen los paneles. */
  WrapPanel.prototype.repartir = function (w, h, escribir, x0, y0) {
    let px = 0, py = 0, altoLinea = 0, maxAncho = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      const d = escribir ? c.deseado : c.medir(w, h);
      if (px > 0 && px + d.w > w) {
        px = 0;
        py += altoLinea + this.sepY;
        altoLinea = 0;
      }
      if (escribir) c.acomodar(x0 + px, y0 + py, d.w, d.h);
      px += d.w + this.sepX;
      altoLinea = Math.max(altoLinea, d.h);
      maxAncho = Math.max(maxAncho, px - this.sepX);
    }
    return { w: maxAncho, h: py + altoLinea };
  };

  WrapPanel.prototype.medirContenido = function (w, h) {
    return this.repartir(isFinite(w) ? w : 1e6, h, false, 0, 0);
  };

  WrapPanel.prototype.acomodarContenido = function (x, y, w, h) {
    this.repartir(w, h, true, x, y);
  };

  WrapPanel.prototype.dibujar = function (ctx) {
    if (this.fondo) {
      const b = this.caja;
      ctx.g.rect(b.x, b.y, b.w, b.h, this.fondo);
      ctx.subir();
    }
  };
  WrapPanel.prototype.mapa = { Background: "fondo" };
  U.tipos.WrapPanel = WrapPanel;

  // ── DockPanel ─────────────────────────────────────────────────────────────
  // Cada hijo se pega a un borde y se queda con una franja entera; el último
  // llena lo que sobra. Es el panel de las ventanas: barra arriba, estado abajo,
  // contenido en el medio.
  function DockPanel(a) {
    U.Elemento.call(this, a);
    this.ultimoLlena = a.LastChildFill !== "false";
    this.fondo = a.Background || null;
  }
  heredar(DockPanel, U.Elemento);

  DockPanel.prototype.medirContenido = function (w, h) {
    let usadoW = 0, usadoH = 0;
    const vis = this.hijos.filter(function (c) { return c.visible; });
    for (let i = 0; i < vis.length; i++) {
      const c = vis[i];
      const ultimo = this.ultimoLlena && i === vis.length - 1;
      const d = c.medir(Math.max(0, w - usadoW), Math.max(0, h - usadoH));
      if (ultimo) { usadoW += d.w; usadoH += d.h; continue; }
      const lado = c.attr["DockPanel.Dock"] || "Top";
      if (lado === "Left" || lado === "Right") usadoW += d.w;
      else usadoH += d.h;
    }
    return { w: usadoW, h: usadoH };
  };

  DockPanel.prototype.acomodarContenido = function (x, y, w, h) {
    let ix = x, iy = y, iw = w, ih = h;
    const vis = this.hijos.filter(function (c) { return c.visible; });
    for (let i = 0; i < vis.length; i++) {
      const c = vis[i];
      if (this.ultimoLlena && i === vis.length - 1) {
        c.acomodar(ix, iy, iw, ih);
        return;
      }
      const lado = c.attr["DockPanel.Dock"] || "Top";
      const d = c.deseado;
      if (lado === "Left") {
        c.acomodar(ix, iy, Math.min(d.w, iw), ih);
        ix += Math.min(d.w, iw); iw -= Math.min(d.w, iw);
      } else if (lado === "Right") {
        c.acomodar(ix + iw - Math.min(d.w, iw), iy, Math.min(d.w, iw), ih);
        iw -= Math.min(d.w, iw);
      } else if (lado === "Bottom") {
        c.acomodar(ix, iy + ih - Math.min(d.h, ih), iw, Math.min(d.h, ih));
        ih -= Math.min(d.h, ih);
      } else {
        c.acomodar(ix, iy, iw, Math.min(d.h, ih));
        iy += Math.min(d.h, ih); ih -= Math.min(d.h, ih);
      }
    }
  };

  DockPanel.prototype.dibujar = function (ctx) {
    if (this.fondo) {
      const b = this.caja;
      ctx.g.rect(b.x, b.y, b.w, b.h, this.fondo);
      ctx.subir();
    }
  };
  DockPanel.prototype.mapa = { Background: "fondo" };
  U.tipos.DockPanel = DockPanel;

  // ── Separator ─────────────────────────────────────────────────────────────
  function Separator(a) {
    U.Elemento.call(this, a);
    this.color = a.Background || C.borde || "#5A5A70";
    this.gruesoLinea = num(a.Thickness, 0.002);
  }
  heredar(Separator, U.Elemento);
  Separator.prototype.medirContenido = function () {
    return { w: 0, h: this.gruesoLinea };
  };
  Separator.prototype.dibujar = function (ctx) {
    const b = this.caja;
    ctx.g.rect(b.x, b.y + (b.h - this.gruesoLinea) / 2, b.w, this.gruesoLinea, this.color);
    ctx.subir();
  };
  U.tipos.Separator = Separator;

  // ── ItemsControl ──────────────────────────────────────────────────────────
  // Una lista: un arreglo de datos y una plantilla que se repite.
  //
  //   <ItemsControl ItemsSource="{Binding filas}" ItemTemplate="fila"/>
  //   app.plantillas.fila = "<Border>...<TextBlock Text='{Binding nombre}'/></Border>";
  //
  // Cada copia de la plantilla recibe **su** elemento del arreglo como contexto
  // de datos, así que adentro los enlaces hablan del item y no del modelo
  // entero. Es lo mismo que hace WPF y es lo que evita tener que numerar los
  // enlaces a mano.
  //
  // Las copias se **reciclan**: al cambiar el arreglo se ajusta la cantidad, no
  // se rehacen. Un árbol nuevo por cuadro sería gratis de dibujar pero perdería
  // el estado de los controles de cada fila.
  function ItemsControl(a) {
    U.Elemento.call(this, a);
    this.plantilla = a.ItemTemplate || null;
    this.fuente = null;
    this.espaciado = num(a.Spacing, 0);
    this.horizontal = (a.Orientation || "Vertical") === "Horizontal";
    /** El `ItemsPanelTemplate` de WPF, en su versión chica: con qué panel se
     *  acomodan las copias. Apilar y envolver son los dos casos que se usan; el
     *  resto se consigue metiendo la lista adentro del panel que uno quiera. */
    this.envolver = (a.ItemsPanel || "") === "Wrap";
    /** Virtualización: medir, acomodar y enlazar **sólo lo que se ve**.
     *
     *  Está medido por qué hace falta. Con 200 filas una pasada completa costaba
     *  3,3 ms; con 800, 12,5 — exactamente cuatro veces, o sea lineal en las
     *  filas **totales** y no en las visibles, aunque en pantalla entren trece.
     *  Se pagaba medir, acomodar y resolver enlaces de setecientas ochenta y
     *  siete filas que nadie iba a mirar.
     *
     *  Supone que **todas las filas miden igual**, que es cierto cuando salen de
     *  la misma plantilla y es lo que permite saber dónde está la número 700 sin
     *  haber tocado las 699 anteriores. Con filas de alto variable habría que
     *  medirlas todas y no habría nada que ganar. */
    this.virtualizar = (a.Virtualize || "true") !== "false";
    this.ventana = null;
    this.altoFila = 0;
    this.desde = 0;
    this.hasta = 0;
  }
  heredar(ItemsControl, U.Elemento);
  ItemsControl.prototype.mapa = { ItemsSource: "fuente" };

  ItemsControl.prototype.aplicarEnlaces = function (datos, app) {
    this.app = app;
    // Primero resolver la propia fuente...
    if (this.enlaces) {
      for (let i = 0; i < this.enlaces.length; i++) {
        const en = this.enlaces[i];
        const campo = this.mapa[en.prop];
        if (campo) {
          const v = U.leerRuta(datos, en.e.ruta);
          if (v !== undefined) this[campo] = v;
        }
      }
    }
    // ...después ajustar la cantidad de copias...
    const lista = this.fuente && this.fuente.length ? this.fuente : [];
    const marcado = this.app && this.app.plantillas
      ? this.app.plantillas[this.plantilla] : null;
    this.marcadoFila = marcado;
    if (marcado && !this.virtualizar) {
      while (this.hijos.length < lista.length) {
        const h = U.inflar(U.leerMarcado(marcado), app ? app.estilos : null);
        if (!h) break;
        this.agregar(h);
      }
      while (this.hijos.length > lista.length) this.hijos.pop();
    }
    this.lista = lista;
    this.datosPadre = datos;

    // ...y recién entonces bajar, cada uno con su propio contexto.
    //
    // Cuando virtualiza, esto **no** pasa acá: enlazar setecientas filas para
    // mostrar trece es justamente el costo que se quiere evitar. Se enlaza en
    // `medirContenido`, que corre después de que el ScrollViewer dijo qué franja
    // se ve, y sólo para esa franja.
    if (this.virtualizar) return;
    for (let i = 0; i < this.hijos.length; i++) {
      const item = lista[i] !== undefined ? lista[i] : datos;
      // Cada copia se queda con **su** dato. Los enlaces ya lo usan y lo tiran;
      // guardarlo es lo que permite que un control adentro de la plantilla sepa
      // de qué fila es cuando lo tocan. Sin esto, un manejador recibe el botón y
      // no tiene forma de saber a qué elemento del arreglo corresponde.
      this.hijos[i].datosItem = item;
      this.hijos[i].aplicarEnlaces(item, app);
    }
  };

  /** Asegurar que haya `n` contenedores en la pileta.
   *
   *  Virtualizando, `hijos` deja de ser "una copia por dato" y pasa a ser **una
   *  pileta del tamaño de la ventana**: quince contenedores alcanzan para una
   *  lista de cuatro mil. El contenedor `k` muestra el dato `desde + k`, y al
   *  desplazarse los mismos quince cambian de dato.
   *
   *  Reciclar así es correcto **porque todo el estado de una fila viene de sus
   *  enlaces**. Si una fila guardara algo por su cuenta —una casilla marcada que
   *  no está en los datos— al desplazarse aparecería en la fila equivocada. Es
   *  la misma trampa que tiene el reciclado de contenedores en WPF, y la regla
   *  es la misma: en una plantilla de fila, el estado va en el dato.
   */
  ItemsControl.prototype.asegurarPileta = function (n, app) {
    if (!this.marcadoFila) return;
    while (this.hijos.length < n) {
      const h = U.inflar(U.leerMarcado(this.marcadoFila), app ? app.estilos : null);
      if (!h) break;
      this.agregar(h);
    }
  };

  /** Qué índices caen dentro de la ventana que puso el padre, con dos filas de
   *  colchón a cada lado para que un desplazamiento chico no deje un hueco. */
  ItemsControl.prototype.rango = function () {
    const n = this.lista ? this.lista.length : 0;
    const paso = this.altoFila + this.espaciado;
    if (!this.ventana || !(paso > 0)) return { desde: 0, hasta: n };
    const desde = Math.max(0, Math.floor(this.ventana.y0 / paso) - 2);
    const hasta = Math.min(n, Math.ceil(this.ventana.y1 / paso) + 2);
    return { desde: desde, hasta: Math.max(desde, hasta) };
  };

  /** Cuando envuelve, se delega en la misma cuenta que usa WrapPanel: tener dos
   *  copias del algoritmo es tener dos algoritmos que se separan. */
  ItemsControl.prototype.repartir = WrapPanel.prototype.repartir;

  ItemsControl.prototype.medirContenido = function (w, h) {
    if (this.virtualizar && !this.envolver && !this.horizontal) {
      const n = this.lista ? this.lista.length : 0;
      if (!n) { this.desde = this.hasta = 0; return { w: 0, h: 0 }; }

      // Una fila de muestra para saber cuánto mide una. Es la única que se mide
      // sin estar en la ventana, y sin ella no habría con qué calcular dónde
      // empieza la número 700.
      this.asegurarPileta(1, this.app);
      if (!this.hijos.length) { this.desde = this.hasta = 0; return { w: 0, h: 0 }; }
      this.hijos[0].datosItem = this.lista[0];
      this.hijos[0].aplicarEnlaces(this.lista[0], this.app);
      this.altoFila = this.hijos[0].medir(w, Infinity).h;

      const r = this.rango();
      this.desde = r.desde;
      this.hasta = r.hasta;
      this.asegurarPileta(r.hasta - r.desde, this.app);
      for (let k = 0; k + r.desde < r.hasta; k++) {
        const c = this.hijos[k];
        if (!c) break;
        // Reciclando, el contenedor `k` cambia de dato al desplazar: el dato se
        // reasigna en cada pasada, nunca se hereda del que estaba antes.
        c.datosItem = this.lista[r.desde + k];
        c.aplicarEnlaces(this.lista[r.desde + k], this.app);
        c.medir(w, Infinity);
      }
      const paso = this.altoFila + this.espaciado;
      return { w: w, h: Math.max(0, n * paso - this.espaciado) };
    }
    if (this.envolver) {
      this.sepX = this.espaciado;
      this.sepY = this.espaciado;
      return this.repartir(isFinite(w) ? w : 1e6, h, false, 0, 0);
    }
    let a = 0, t = 0, n = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      const d = this.horizontal ? c.medir(Infinity, h) : c.medir(w, Infinity);
      if (this.horizontal) { t += d.w; a = Math.max(a, d.h); }
      else { t += d.h; a = Math.max(a, d.w); }
      n++;
    }
    if (n > 1) t += this.espaciado * (n - 1);
    return this.horizontal ? { w: t, h: a } : { w: a, h: t };
  };

  ItemsControl.prototype.acomodarContenido = function (x, y, w, h) {
    if (this.virtualizar && !this.envolver && !this.horizontal) {
      const paso = this.altoFila + this.espaciado;
      for (let k = 0; k + this.desde < this.hasta; k++) {
        const c = this.hijos[k];
        if (!c) break;
        c.acomodar(x, y + (this.desde + k) * paso, w, this.altoFila);
      }
      return;
    }
    if (this.envolver) return void this.repartir(w, h, true, x, y);
    let p = this.horizontal ? x : y;
    for (let i = 0; i < this.hijos.length; i++) {
      const c = this.hijos[i];
      if (!c.visible) continue;
      if (this.horizontal) {
        c.acomodar(p, y, c.deseado.w, h);
        p += c.deseado.w + this.espaciado;
      } else {
        c.acomodar(x, p, w, c.deseado.h);
        p += c.deseado.h + this.espaciado;
      }
    }
  };
  ItemsControl.prototype.emitirContenido = function (ctx) {
    if (!this.visible || !this.opaco) return;
    this.dibujar(ctx);
    if (this.virtualizar && !this.envolver && !this.horizontal) {
      for (let k = 0; k + this.desde < this.hasta; k++) {
        if (!this.hijos[k]) break;
        this.hijos[k].emitir(ctx);
      }
      return;
    }
    for (let i = 0; i < this.hijos.length; i++) this.hijos[i].emitir(ctx);
  };
  U.tipos.ItemsControl = ItemsControl;

  // ── ScrollViewer ──────────────────────────────────────────────────────────
  // Un hijo más alto que su caja, y una barra para moverlo.
  //
  // Sin tijera de GPU, el recorte lo hace la geometría: `ctx.recortar()` mete un
  // rectángulo en la pila y todo lo que se emita adentro se intersecta contra
  // él. Lo que queda afuera no se dibuja **ni se puede tocar**, que es la mitad
  // que se olvida y deja botones invisibles activos.
  //
  // No hay arrastre —el motor manda toques sueltos, no gestos— así que se mueve
  // tocando la barra, como en un scrollbar clásico.
  function ScrollViewer(a) {
    U.Elemento.call(this, a);
    this.desplazamiento = num(a.Offset, 0);
    this.anchoBarra = num(a.ScrollBarWidth, 0.022);
    this.fondoBarra = a.ScrollBarBackground || C.superficieAlta || "#2A2A38";
    this.perilla = a.ScrollBarBrush || C.borde || "#5A5A70";
    this.altoContenido = 0;
  }
  heredar(ScrollViewer, U.Elemento);

  ScrollViewer.prototype.medirContenido = function (w, h) {
    const wc = Math.max(0, w - this.anchoBarra);
    let alto = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      // Decirle al hijo qué franja se va a ver, **antes** de que se mida. Es lo
      // que hace posible que una lista virtualizada sepa qué filas tocar; sin
      // esto tendría que medirlas todas para recién después descubrir cuáles
      // estaban a la vista.
      this.hijos[i].ventana = {
        y0: this.desplazamiento,
        y1: this.desplazamiento + (isFinite(h) ? h : 1e6),
      };
      // Altura infinita: el hijo mide lo que necesita, y de esa diferencia con
      // la caja sale cuánto hay para desplazar.
      const d = this.hijos[i].medir(wc, Infinity);
      alto = Math.max(alto, d.h);
    }
    this.altoContenido = alto;
    return { w: w, h: isFinite(h) ? h : alto };
  };

  ScrollViewer.prototype.acomodarContenido = function (x, y, w, h) {
    const wc = Math.max(0, w - this.anchoBarra);
    this.sobra = Math.max(0, this.altoContenido - h);
    this.desplazamiento = Math.max(0, Math.min(this.sobra, this.desplazamiento));
    for (let i = 0; i < this.hijos.length; i++) {
      this.hijos[i].acomodar(x, y - this.desplazamiento, wc,
                             Math.max(h, this.altoContenido));
    }
  };

  ScrollViewer.prototype.emitirContenido = function (ctx) {
    if (!this.visible || !this.opaco) return;
    const b = this.caja;
    ctx.recortar(b.x, b.y, Math.max(0, b.w - this.anchoBarra), b.h);
    for (let i = 0; i < this.hijos.length; i++) this.hijos[i].emitir(ctx);
    ctx.restaurar();
    this.dibujarBarra(ctx);
  };

  ScrollViewer.prototype.dibujarBarra = function (ctx) {
    const b = this.caja;
    if (!(this.sobra > 0)) return;
    const bx = b.x + b.w - this.anchoBarra;
    const self = this;

    ctx.blanco(bx, b.y, this.anchoBarra, b.h, this.fondoBarra, function (app, hoja) {
      if (!hoja) return;
      // El toque manda la perilla a donde se tocó, con la perilla centrada. Es
      // lo que hace un scrollbar cuando le tocás la pista.
      const t = (hoja.y - b.y) / Math.max(0.001, b.h);
      self.desplazamiento = Math.max(0, Math.min(1, t)) * self.sobra;
      app.invalidar();
    });
    ctx.subir();

    ctx.g.rectRedondeado(bx + 0.003, b.y, this.anchoBarra - 0.006, b.h,
                         (this.anchoBarra - 0.006) / 2, this.fondoBarra);
    ctx.subir();

    const visible = b.h / Math.max(0.001, this.altoContenido);
    const altoPerilla = Math.max(0.05, b.h * visible);
    const t = this.sobra > 0 ? this.desplazamiento / this.sobra : 0;
    ctx.g.rectRedondeado(bx + 0.003, b.y + t * (b.h - altoPerilla),
                         this.anchoBarra - 0.006, altoPerilla,
                         (this.anchoBarra - 0.006) / 2, this.perilla);
    ctx.subir();
  };
  ScrollViewer.prototype.mapa = { Offset: "desplazamiento" };
  U.tipos.ScrollViewer = ScrollViewer;
})();


// ──────────────────────────────────────────────────────────────────────
// mas.js
// ──────────────────────────────────────────────────────────────────────
// mas.js — el resto del juego base.
//
// Image, RadioButton, ToggleSwitch y TabControl. Ninguno inventa nada nuevo del
// motor: son composiciones de lo que ya hay. Que se puedan escribir en cincuenta
// líneas cada uno es la prueba de que el núcleo estaba bien repartido.
(function () {
  "use strict";

  const U = globalThis.UI;
  const heredar = U.heredar;
  const num = U.numero;
  const bool = U.booleano;
  const grosor = U.grosor;
  const anchoDe = U.anchoDe;
  const altoDe = U.altoDe;
  const medirTexto = U.medirTexto;
  const C = (U.CFG && U.CFG.C) || {};

  // ── Image ─────────────────────────────────────────────────────────────────
  // Lo único que no puede ser malla además del texto. Un `<image>` es un plano
  // texturado: `fit` viene en `contain` y `unlit` en true, y **`color` tiñe** —
  // multiplica, al revés que una textura en modo overlay, donde el color lo pone
  // el PNG. Es la misma incoherencia que ya tenía el motor y acá sólo se hereda.
  function Image(a) {
    U.Elemento.call(this, a);
    this.fuente = a.Source || "";
    this.ajuste = a.Stretch || "contain";
    this.tinte = a.Foreground || null;
    this.anchoNatural = num(a.NaturalWidth, 0.1);
    this.altoNatural = num(a.NaturalHeight, 0.1);
  }
  heredar(Image, U.Elemento);
  Image.prototype.mapa = { Source: "fuente", Foreground: "tinte" };

  Image.prototype.medirContenido = function () {
    // El motor no dice cuánto mide una imagen hasta que la bajó, y el layout no
    // puede esperar. Así que el tamaño se declara; si no se declara, se asume
    // cuadrada de 10 cm. Es la misma clase de deuda que la métrica de texto.
    return { w: this.anchoNatural, h: this.altoNatural };
  };

  Image.prototype.dibujar = function (ctx) {
    if (!this.fuente) return;
    const b = this.caja;
    ctx.imagen(b.x, b.y, b.w, b.h, this.fuente, this.ajuste, this.tinte);
    ctx.subir();
  };
  U.tipos.Image = Image;

  // ── RadioButton ───────────────────────────────────────────────────────────
  // Como el CheckBox pero excluyente: al prenderse apaga a los de su grupo.
  // El grupo se resuelve recorriendo el árbol desde la raíz, no con un registro
  // global: dos aplicaciones con el mismo nombre de grupo no tienen por qué
  // pisarse.
  function RadioButton(a) {
    U.Interactivo.call(this, a);
    this.marcado = bool(a.IsChecked, false);
    this.grupo = a.Group || "";
    this.contenido = a.Content !== undefined ? a.Content : (a.__texto || "");
    this.tam = num(a.FontSize, 0.032);
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.acento = a.Accent || C.azul || "#0A84FF";
    this.alCambiar = a.Changed || null;
    this.lado = num(a.BoxSize, 0.042);
    this.hueco = num(a.Spacing, 0.014);
    this.valor = a.Value !== undefined ? a.Value : this.contenido;
  }
  heredar(RadioButton, U.Interactivo);
  RadioButton.prototype.mapa = {
    IsChecked: "marcado", Content: "contenido", IsEnabled: "habilitado",
  };

  RadioButton.prototype.medirContenido = function () {
    const m = medirTexto(this.contenido, this.tam);
    return { w: this.lado + this.hueco + m.w, h: Math.max(this.lado, m.h) };
  };

  /** Apagar a los hermanos del mismo grupo. */
  RadioButton.prototype.excluir = function (nodo) {
    if (nodo !== this && nodo instanceof RadioButton && nodo.grupo === this.grupo) {
      nodo.marcado = false;
    }
    for (let i = 0; i < nodo.hijos.length; i++) this.excluir(nodo.hijos[i]);
  };

  RadioButton.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const l = this.lado;
    const cy = b.y + (b.h - l) / 2;
    const aro = this.marcado ? this.acento : (C.borde || "#5A5A70");
    const k = this.realce();
    const brillo = k > 0 ? U.tinte(aro, 1 + k) : aro;
    const self = this;

    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, C.superficie || "#1C1C26", function (app) {
        if (app.raiz) self.excluir(app.raiz);
        self.marcado = true;
        self.devolver(app.datos, "Value", self.valor);
        self.marcarToque(app);
        const h = self.manejador(app, self.alCambiar);
        if (h) h(self, app);
        app.invalidar();
      }, this);
      ctx.subir();
    }

    // Redondo, no cuadrado: es lo único que lo distingue de una casilla a
    // distancia, y esa distinción es la que dice "elegí uno" en vez de "marcá
    // los que quieras".
    ctx.g.rectRedondeado(b.x, cy, l, l, l / 2, brillo);
    ctx.subir();
    ctx.g.rectRedondeado(b.x + 0.005, cy + 0.005, l - 0.01, l - 0.01,
                         (l - 0.01) / 2, C.superficie || "#1C1C26");
    ctx.subir();
    if (this.marcado) {
      ctx.g.rectRedondeado(b.x + l * 0.28, cy + l * 0.28, l * 0.44, l * 0.44,
                           l * 0.22, this.acento);
    }
    ctx.subir();
    if (this.contenido) {
      const m = medirTexto(this.contenido, this.tam);
      ctx.texto(b.x + l + this.hueco + m.w / 2, b.y + b.h / 2,
                this.contenido, this.tam, this.color);
    }
    ctx.subir();
  };
  U.tipos.RadioButton = RadioButton;

  // ── ToggleSwitch ──────────────────────────────────────────────────────────
  // La misma semántica que un CheckBox con otra forma. Existe porque en un panel
  // a metro y medio, una llave que se corre se lee de un vistazo y una casilla
  // de cuatro centímetros hay que buscarla.
  function ToggleSwitch(a) {
    U.Interactivo.call(this, a);
    this.marcado = bool(a.IsChecked, false);
    this.contenido = a.Content !== undefined ? a.Content : (a.__texto || "");
    this.tam = num(a.FontSize, 0.032);
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.acento = a.Accent || C.verde || "#30D158";
    this.apagado = a.OffBrush || C.superficieAlta || "#2A2A38";
    this.alCambiar = a.Changed || null;
    this.ancho = num(a.SwitchWidth, 0.075);
    this.alto = num(a.SwitchHeight, 0.038);
    this.hueco = num(a.Spacing, 0.016);
  }
  heredar(ToggleSwitch, U.Interactivo);
  ToggleSwitch.prototype.mapa = {
    IsChecked: "marcado", Content: "contenido", IsEnabled: "habilitado",
  };

  ToggleSwitch.prototype.medirContenido = function () {
    const m = medirTexto(this.contenido, this.tam);
    return {
      w: this.ancho + (this.contenido ? this.hueco + m.w : 0),
      h: Math.max(this.alto, m.h),
    };
  };

  ToggleSwitch.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const cy = b.y + (b.h - this.alto) / 2;
    const pista = this.marcado ? this.acento : this.apagado;
    const self = this;

    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, C.superficie || "#1C1C26", function (app) {
        self.marcado = !self.marcado;
        self.devolver(app.datos, "IsChecked", self.marcado);
        self.marcarToque(app);
        const h = self.manejador(app, self.alCambiar);
        if (h) h(self, app);
        app.invalidar();
      }, this);
      ctx.subir();
    }

    ctx.g.rectRedondeado(b.x, cy, this.ancho, this.alto, this.alto / 2,
                         this.realce() > 0
                           ? U.tinte(pista, 1 + 0.8 * this.realce()) : pista);
    ctx.subir();
    const r = this.alto - 0.008;
    const px = this.marcado ? b.x + this.ancho - r - 0.004 : b.x + 0.004;
    ctx.g.rectRedondeado(px, cy + 0.004, r, r, r / 2, C.texto || "#FFFFFF");
    ctx.subir();
    if (this.contenido) {
      const m = medirTexto(this.contenido, this.tam);
      ctx.texto(b.x + this.ancho + this.hueco + m.w / 2, b.y + b.h / 2,
                this.contenido, this.tam, this.color);
    }
    ctx.subir();
  };
  U.tipos.ToggleSwitch = ToggleSwitch;

  // ── TabControl ────────────────────────────────────────────────────────────
  // Una tira de solapas arriba y un solo hijo visible abajo. No dibuja el
  // contenido de las que no están elegidas: **no las mide siquiera**, así que
  // una solapa con una lista de mil filas no cuesta nada mientras esté cerrada.
  function TabItem(a) {
    U.Elemento.call(this, a);
    this.encabezado = a.Header || "";
  }
  heredar(TabItem, U.Elemento);
  TabItem.prototype.mapa = { Header: "encabezado" };

  TabItem.prototype.medirContenido = function (w, h) {
    let mw = 0, mh = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      const d = this.hijos[i].medir(w, h);
      mw = Math.max(mw, d.w);
      mh = Math.max(mh, d.h);
    }
    return { w: mw, h: mh };
  };
  TabItem.prototype.acomodarContenido = function (x, y, w, h) {
    for (let i = 0; i < this.hijos.length; i++) this.hijos[i].acomodar(x, y, w, h);
  };
  U.tipos.TabItem = TabItem;

  function TabControl(a) {
    U.Interactivo.call(this, a);
    this.elegida = Math.max(0, Math.trunc(num(a.SelectedIndex, 0)));
    this.tam = num(a.FontSize, 0.03);
    this.acento = a.Accent || C.azul || "#0A84FF";
    this.fondoSolapa = a.TabBackground || C.superficieAlta || "#2A2A38";
    this.relleno = grosor(a.TabPadding || "0.018,0.01");
    this.hueco = num(a.TabSpacing, 0.006);
    this.alCambiar = a.SelectionChanged || null;
  }
  heredar(TabControl, U.Interactivo);
  TabControl.prototype.mapa = { SelectedIndex: "elegida" };

  TabControl.prototype.altoTira = function () {
    return this.tam * 1.30 + altoDe(this.relleno);
  };

  TabControl.prototype.medirContenido = function (w, h) {
    const tira = this.altoTira();
    let anchoTira = 0;
    for (let i = 0; i < this.hijos.length; i++) {
      anchoTira += medirTexto(this.hijos[i].encabezado, this.tam).w +
                   anchoDe(this.relleno) + this.hueco;
    }
    const activa = this.hijos[this.elegida];
    const d = activa ? activa.medir(w, Math.max(0, h - tira)) : { w: 0, h: 0 };
    return { w: Math.max(anchoTira, d.w), h: tira + d.h };
  };

  TabControl.prototype.acomodarContenido = function (x, y, w, h) {
    const tira = this.altoTira();
    for (let i = 0; i < this.hijos.length; i++) {
      // Las cerradas se acomodan en una caja vacía: no se dibujan y sus blancos
      // no existen. Es más simple que una lista aparte y no se puede olvidar.
      if (i === this.elegida) this.hijos[i].acomodar(x, y + tira, w, Math.max(0, h - tira));
      else this.hijos[i].acomodar(x, y + tira, 0, 0);
    }
  };

  TabControl.prototype.emitirContenido = function (ctx) {
    if (!this.visible || !this.opaco) return;
    const b = this.caja;
    const tira = this.altoTira();
    let px = b.x;
    const self = this;

    for (let i = 0; i < this.hijos.length; i++) {
      const t = this.hijos[i];
      const m = medirTexto(t.encabezado, this.tam);
      const w = m.w + anchoDe(this.relleno);
      const elegida = i === this.elegida;
      const idx = i;

      ctx.blanco(px, b.y, w, tira, this.fondoSolapa, function (app) {
        self.elegida = idx;
        self.marcarToque(app);
        const h = self.manejador(app, self.alCambiar);
        if (h) h(self, app);
        app.invalidar();
      // El dueño del blanco es **la solapa**, no el TabControl: si fuera el
      // control entero, pasar el puntero por una resaltaría las cinco.
      }, t);
      ctx.subir();
      const fondoSolapa = elegida ? this.acento
                        : t.encima ? U.tinte(this.fondoSolapa, 1.5)
                        : this.fondoSolapa;
      ctx.g.rectRedondeado(px, b.y, w, tira, 0.008, fondoSolapa);
      ctx.subir();
      ctx.texto(px + w / 2, b.y + tira / 2, t.encabezado, this.tam,
                elegida ? (C.fondo || "#08080C") : (C.textoTenue || "#A8A8BC"));
      ctx.subir();
      px += w + this.hueco;
    }

    const activa = this.hijos[this.elegida];
    if (activa) activa.emitir(ctx);
  };
  U.tipos.TabControl = TabControl;
})();


// ──────────────────────────────────────────────────────────────────────
// host.js
// ──────────────────────────────────────────────────────────────────────
// host.js — el que hace correr todo.
//
// Toma el marcado, arma el árbol, corre las dos pasadas y vuelca el resultado:
// una malla, unos textos y unos blancos táctiles. También decide **cuándo**
// hacerlo, que es la mitad del trabajo de un framework de interfaces.
//
// La política es: no redibujar por cuadro. El banco dice que se puede
// —2000 rectángulos por cuadro a 135 fps— pero poder no es deber: una interfaz
// quieta no tiene por qué gastar nada. Se redibuja cuando algo la invalida, y
// las animaciones piden cuadros mientras duran.
(function () {
  "use strict";

  const U = globalThis.UI;
  const root = U.root;

  // Technical separation inside a control, independent of row count and of
  // the explicit physical Depth/Elevation. Siblings restore their entry plane.
  const PASO_Z = 0.0001;

  /** Inflar el árbol desde el marcado ya leído.
   *
   *  `estilos` es el `Style TargetType` de WPF, en su versión más chica que
   *  todavía sirve: un juego de atributos por tipo de elemento, que se aplica
   *  **sólo donde el marcado no dijo nada**. Sin triggers, sin herencia de
   *  estilos, sin recursos. Alcanza para que una aplicación no repita
   *  `FontSize="0.032" Foreground="#A8A8BC"` en veinte líneas. */
  function inflar(nodo, estilos) {
    if (!nodo) return null;
    const Tipo = U.tipos[nodo.nombre];
    if (!Tipo) {
      console.log("[ui] elemento desconocido: " + nodo.nombre);
      return null;
    }
    const a = {};
    const estilo = estilos ? estilos[nodo.nombre] : null;
    if (estilo) for (const k in estilo) a[k] = estilo[k];
    for (const k in nodo.atributos) a[k] = nodo.atributos[k];
    if (nodo.texto) a.__texto = nodo.texto;
    const el = new Tipo(a);
    for (let i = 0; i < nodo.hijos.length; i++) {
      const h = inflar(nodo.hijos[i], estilos);
      if (h) el.agregar(h);
    }
    if (el.alInflar) el.alInflar();
    return el;
  }
  U.inflar = inflar;

  function Aplicacion(op) {
    const o = op || {};
    this.ancho = o.ancho || 1.4;
    this.alto = o.alto || 0.95;
    /** Corrimiento lateral del panel. Existe desde que hubo una sala con mas de
     *  un panel: sin esto todos nacen centrados en x=0 y se pisan. */
    this.x = o.x !== undefined ? o.x : 0;
    this.y = o.y !== undefined ? o.y : 1.62;
    this.z = o.z !== undefined ? o.z : -(U.CFG.distancia || 1.6);
    /** El contexto de datos, como el DataContext de WPF: el objeto contra el
     *  que se resuelven los `{Binding}`. */
    this.datos = o.datos || {};
    /** Plantillas con nombre, para los ItemsControl. Es el `DataTemplate` de
     *  XAML sin el ceremonial de los recursos. */
    this.plantillas = o.plantillas || {};
    /** Estilos por tipo de elemento. Ver `inflar`. */
    this.estilos = o.estilos || null;
    this.manejadores = o.manejadores || {};

    this.g = new U.Geometria();
    this.malla = null;
    this._textBatches = new Map();
    this._panels = new Map();
    this._renderVersion = 1;
    this._animations = new Map();
    this.nodoMalla = root.getElementById(o.malla || "ui_malla");
    this.contenedor = root.getElementById(o.nodos || "ui_nodos");

    const self = this;
    this.textos = new U.Pileta(this.contenedor, "text");
    /** Las imágenes son la tercera población de nodos, por la misma razón que
     *  los textos: un `<image>` es un plano texturado y no hay forma de meter
     *  una textura en la malla de la interfaz. */
    this.imagenes = new U.Pileta(this.contenedor, "image", function (item) {
      item.el.setAttribute("material-alpha", "blend");
      item.el.setAttribute("touchable", "false");
      item.el.addEventListener("load",function(){self.invalidar();});
    });
    this.blancos = new U.Pileta(this.contenedor, "plane", function (item) {
      item.el.setAttribute("touchable", "true");
      item.el.setAttribute("material-alpha","blend");
      item.el.setAttribute("color","#00000000");

      // **Hover.** El motor lo tiene desde `feat(input): add HTML-like hover`:
      // los nodos `touchable` reciben `pointerenter` / `pointerleave` sin que
      // nadie apriete nada, con el cursor en escritorio y con los rayos de los
      // mandos en VR (ver HOVER.md del motor).
      //
      // El framework se escribió antes suponiendo que no existía, y el destello
      // al presionar estaba puesto justamente para tapar esa ausencia. Ahora el
      // destello dice "te escuché" y el hover dice "esto se puede tocar", que es
      // el reparto de siempre y el que la gente ya conoce.
      //
      // El dueño se reasigna en cada pasada, así que se guarda **en el ítem** y
      // no se captura: capturarlo dejaría el resaltado prendido en un control
      // que ya no está ahí.
      item.el.addEventListener("pointerenter", function () {
        if (item.duenio) { item.duenio.encima = true; self.invalidateRender(item.duenio); }
      });
      item.el.addEventListener("pointerleave", function () {
        if (item.duenio) { item.duenio.encima = false; self.invalidateRender(item.duenio); }
      });
      item.el.addEventListener("toque", function (e) {
        // El manejador se reasigna en cada pasada; el listener se pone una sola
        // vez. Si se agregara uno por pasada, cada redibujo dejaría oyentes
        // muertos escuchando el mismo nodo.
        //
        // El evento trae el punto de impacto **en coordenadas del mundo**. Como
        // el panel no rota, traducirlo a coordenadas de la hoja es una resta y
        // una inversión de signo. Es lo que hace posible un Slider.
        let hoja = null;
        if (e && Number.isFinite(e.localX) && Number.isFinite(e.localY) && item.bounds) {
          const b=item.bounds;hoja={x:b.x+(e.localX+0.5)*b.w,y:b.y+(0.5-e.localY)*b.h};
        } else if (e && isFinite(e.x) && isFinite(e.y)) {
          hoja = { x: e.x - self.origenX, y: self.origenY - e.y };
        }
        if (item.alTocar) item.alTocar(self, hoja);
      });
    });

    // El panel se coloca con los nodos contenedores, no sumando el offset a cada
    // vértice: el layout queda en 2D puro y mover el panel es mover dos nodos.
    const x0 = this.x - this.ancho / 2;
    const y0 = this.y + this.alto / 2;
    /** Origen de la hoja en el mundo. Lo necesita la traducción del punto de
     *  impacto; guardarlo evita recalcularlo en cada toque. */
    this.origenX = x0;
    this.origenY = y0;
    for (const n of [this.nodoMalla, this.contenedor]) {
      if (!n) continue;
      n.setAttribute("x", String(x0));
      n.setAttribute("y", String(y0));
      n.setAttribute("z", String(this.z));
    }

    this.raiz = null;
    this.sucio = true;
    this.animando = 0;
    this.pasadas = 0;
    this._frame = null;
    this._running = false;
    this._layoutDirty = true;
    this._arrangeDirty = true;
  }

  Aplicacion.prototype.cargar = function (marcado) {
    this.raiz = inflar(U.leerMarcado(marcado), this.estilos);
    if (this.raiz) {this.raiz.app = this;this.raiz.renderPanel=true;}
    this.invalidar();
    return this.raiz;
  };

  Aplicacion.prototype.buscar = function (nombre) {
    return this.raiz ? this.raiz.buscar(nombre) : null;
  };

  /** Pedir un redibujo. Barato de llamar de más: sólo prende una bandera. */
  Aplicacion.prototype.invalidar = function () {
    this._layoutDirty = true;
    this._arrangeDirty = true;
    this.invalidateRender();
  };
  Aplicacion.prototype.invalidateMeasure = Aplicacion.prototype.invalidar;
  Aplicacion.prototype.invalidateArrange = function() {
    this._arrangeDirty=true;this.invalidateRender();
  };
  Aplicacion.prototype._markRender = function (element) {
    if (!element) {this._renderVersion++;return;}
    for(let el=element;el;el=el.padre) if(el._panelState) el._panelState.dirty=true;
  };
  Aplicacion.prototype.invalidateRender = function (element) {
    this._markRender(element);
    this.sucio = true;
    this._programar();
  };
  Aplicacion.prototype._programar = function () {
    if (!this._running || this._frame !== null) return;
    const self = this;
    this._frame = requestAnimationFrame(function () {
      self._frame = null;
      if (!self._running) return;
      // Clear before rendering: an invalidation during render must survive.
      self.sucio = false;
      for (const [element,until] of self._animations) {
        self._markRender(element);
        if(Date.now()>=until)self._animations.delete(element);
      }
      self.pasada(true);
      if (self.sucio || Date.now() < self.animando) self._programar();
    });
  };

  /** Pedir cuadros continuos durante `ms`. Lo usan las animaciones de los
   *  controles: mientras dure, el bucle redibuja aunque nadie invalide. */
  Aplicacion.prototype.animar = function (ms, element) {
    const hasta = Date.now() + (ms || 250);
    if (hasta > this.animando) this.animando = hasta;
    this._animations.set(element || null,hasta);
    this.invalidateRender(element);
  };

  Aplicacion.prototype.pasada = function (incremental) {
    if (!incremental) {this._layoutDirty = true;this._arrangeDirty=true;this._renderVersion++;}
    if (!this.raiz) return;
    const t0 = Date.now();
    this._frontmost=0;

    if (this._layoutDirty) {
      this._layoutDirty = false;
      this.raiz.aplicarEnlaces(this.datos, this);
      this.raiz.medir(this.ancho, this.alto);
      this._arrangeDirty=true;
    }
    if(this._arrangeDirty){
      this._arrangeDirty=false;
      this.raiz.acomodar(0, 0, this.ancho, this.alto);
    }

    for(const panel of this._panels.values())panel.seen=false;
    // 3. Emitir.
    this.g.reiniciar();
    for (const batch of this._textBatches.values()) batch.begin();
    this.textos.comenzar();
    this.imagenes.comenzar();
    this.blancos.comenzar();

    const self = this;
    const ctx = {
      get g() {return self.g;},
      app: this,
      subir: function () { self.g.z += PASO_Z;self._frontmost=Math.max(self._frontmost,self.g.z+PASO_Z*2); },
      recortar: function (x, y, w, h) { self.g.recortar(x, y, w, h); },
      restaurar: function () { self.g.restaurar(); },
      texto: function (x, y, valor, tam, color, preparedLayout) {
        self._frontmost=Math.max(self._frontmost,self.g.z+PASO_Z*2);
        const layout = preparedLayout || U.medirTexto(valor,tam);
        if (layout.glyphs) {
          for (const glyph of layout.glyphs) {
            const source = "luna://ui-font/" + glyph.page;
            let batch = self._textBatches.get(source);
            if (!batch) { batch = new U.TexturedBatch(self.contenedor,source); self._textBatches.set(source,batch); }
            batch.quad(x-layout.w/2+glyph.x,y-layout.h/2+glyph.y,glyph.w,glyph.h,
              glyph.u,glyph.v,glyph.uw,glyph.vh,color,self.g.z+PASO_Z*2,self.g);
          }
          return;
        }
        // Un `<text>` es un nodo y no se puede cortar por la mitad: o entra o no
        // se dibuja. Se decide por su centro, que es donde el motor lo ancla.
        if (!self.g.puntoVisible(x, y)) return;
        const it = self.textos.pedir();
        it.estacionado = false;
        it.el.setAttribute("value", String(valor));
        it.el.setAttribute("size", String(tam));
        it.el.setAttribute("color", color);
        it.el.setAttribute("x", String(x));
        it.el.setAttribute("y", String(-y));
        it.el.setAttribute("z", String(self.g.z + PASO_Z * 2));
      },
      imagen: function (x, y, w, h, src, ajuste, tinte) {
        if (w <= 0 || h <= 0) return;
        const it = self.imagenes.pedir();
        it.estacionado = false;
        it.el.setAttribute("src", src);
        const nw=it.el.naturalWidth, nh=it.el.naturalHeight;
        let u=0,v=0,uw=1,vh=1;
        if(nw>0 && nh>0){
          const ratio=(nw/nh)/(w/h);
          if((ajuste||"contain")==="contain"){
            const rw=ratio<1?w*ratio:w,rh=ratio>1?h/ratio:h;
            x+=(w-rw)/2;y+=(h-rh)/2;w=rw;h=rh;
          } else if(ajuste==="cover"){
            uw=Math.min(1,1/ratio);vh=Math.min(1,ratio);u=(1-uw)/2;v=(1-vh)/2;
          }
        }
        const r=self.g.aplicarClip(x,y,w,h);
        if(!r){it.el.setAttribute("visible","false");return;}
        it.el.setAttribute("visible","true");
        it.el.setAttribute("fit","stretch");
        it.el.setAttribute("texture-region",[u+(r.x-x)/w*uw,v+(r.y-y)/h*vh,r.w/w*uw,r.h/h*vh].join(","));
        it.el.setAttribute("color",tinte||"#FFFFFF");
        it.el.setAttribute("sx",String(r.w));it.el.setAttribute("sy",String(r.h));
        it.el.setAttribute("x",String(r.x+r.w/2));it.el.setAttribute("y",String(-(r.y+r.h/2)));
        it.el.setAttribute("z",String(self.g.z+PASO_Z*2));
      },
      blanco: function (x, y, w, h, color, alTocar, duenio) {
        if (w <= 0 || h <= 0) return;
        // El blanco también se recorta: un botón que se fue de la lista no tiene
        // que seguir siendo tocable donde ya no se ve.
        const r = self.g.aplicarClip(x, y, w, h);
        if (!r) return;
        x = r.x; y = r.y; w = r.w; h = r.h;
        const it = self.blancos.pedir();
        it.estacionado = false;
        it.alTocar = alTocar;
        it.bounds={x,y,w,h};
        it.duenio = duenio || null;
        it.el.setAttribute("sx", String(w));
        it.el.setAttribute("sy", String(h));
        it.el.setAttribute("color", "#00000000");
        it.el.setAttribute("x", String(x + w / 2));
        it.el.setAttribute("y", String(-(y + h / 2)));
        it.el.setAttribute("z", String(self.g.z + PASO_Z));
      },
    };

    this.raiz.emitir(ctx);

    // Las capas superpuestas, al final y a proposito: emitir despues es estar
    // mas adelante, tanto para la vista como para el rayo del toque. Ver
    // superpuestos.js. El `if` es porque los bancos corren sin esa pieza.
    if (this.revisarGlobos) this.revisarGlobos();
    if (this.emitirCapas) {this.g.z=Math.max(this.g.z,this._frontmost);this.emitirCapas(ctx);}

    for (const batch of this._textBatches.values()) batch.commit();
    this.textos.terminar();
    this.imagenes.terminar();
    this.blancos.terminar();

    const d = this.g.malla();
    if(d.indices.length){
      this.nodoMalla.setAttribute("material-alpha",transparent(d.colors)?"blend":"opaque");
      this.nodoMalla.setAttribute("visible","true");
      if (!this.malla) {this.malla=MeshResource.create(d);this.nodoMalla.src=this.malla.src;}
      else this.malla.update(d);
    } else if(this.nodoMalla) this.nodoMalla.setAttribute("visible","false");
    for(const [element,panel] of this._panels){
      if(!panel.seen){this._disposePanel(panel);element._panelState=null;this._panels.delete(element);}
    }

    this.pasadas++;
    this.ultimoCosto = Date.now() - t0;
    this.ultimosTriangulos = d.indices.length / 3;
    for(const panel of this._panels.values()){this.ultimosTriangulos+=panel.geometry.idx.length/3;for(const batch of panel.batches.values())this.ultimosTriangulos+=batch.indices.length/3;}
  };

  // Explicit geometry boundary. The root, dialogs and large independent groups
  // can have their own mesh without recording/counting draw operations.
  Aplicacion.prototype._renderPanel = function (element,ctx,emit) {
    let panel=this._panels.get(element);
    if(!panel){
      const node=root.createElement("model");node.setAttribute("material-unlit","true");
      node.setAttribute("touchable","false");this.contenedor.appendChild(node);
      panel={node,geometry:new U.Geometria(),resource:null,dirty:true,version:0,children:new Set(),
        texts:new U.Pileta(this.contenedor,"text"),images:new U.Pileta(this.contenedor,"image",this.imagenes.preparar),
        hits:new U.Pileta(this.contenedor,"plane",this.blancos.preparar),batches:new Map(),renders:0};
      this._panels.set(element,panel);element._panelState=panel;
    }
    if(this._activePanel)this._activePanel.children.add(panel);
    function seen(p){p.seen=true;for(const child of p.children)seen(child);}
    panel.seen=true;
    const start=this.g.z, clip=JSON.stringify(this.g.clip), box=JSON.stringify(element.caja);
    if(!panel.dirty && panel.version===this._renderVersion && panel.start===start && panel.clip===clip && panel.box===box){
      seen(panel);this.g.z=panel.end;this._frontmost=Math.max(this._frontmost,panel.frontmost);return;
    }
    const saved=[this.g,this.textos,this.imagenes,this.blancos,this._textBatches,this._activePanel];
    panel.geometry.reiniciar();panel.geometry.z=start;panel.geometry.clip=this.g.clip;
    this.g=panel.geometry;this.textos=panel.texts;this.imagenes=panel.images;this.blancos=panel.hits;
    this._textBatches=panel.batches;this._activePanel=panel;panel.children.clear();
    panel.dirty=false;panel.version=this._renderVersion;panel.start=start;panel.clip=clip;panel.box=box;
    this.textos.comenzar();this.imagenes.comenzar();this.blancos.comenzar();
    for(const batch of this._textBatches.values())batch.begin();
    try {
      emit();panel.end=this.g.z;panel.frontmost=this._frontmost;
      for(const batch of this._textBatches.values())batch.commit();
      this.textos.terminar();this.imagenes.terminar();this.blancos.terminar();
      const d=this.g.malla();
      panel.node.setAttribute("material-alpha",transparent(d.colors)?"blend":"opaque");
      panel.node.setAttribute("visible",d.indices.length?"true":"false");
      if(d.indices.length){
        if(panel.resource)panel.resource.update(d);
        else {panel.resource=MeshResource.create(d);panel.node.src=panel.resource.src;}
      }
      panel.renders++;
    } catch(error){panel.dirty=true;throw error;}
    finally {[this.g,this.textos,this.imagenes,this.blancos,this._textBatches,this._activePanel]=saved;}
    this.g.z=panel.end;
  };
  Aplicacion.prototype._disposePanel = function (panel) {
    if(panel.resource)panel.resource.dispose();panel.node.remove();
    for(const pool of [panel.texts,panel.images,panel.hits])for(const item of pool.items){if(item.duenio)item.duenio.encima=false;item.duenio=null;item.alTocar=null;item.el.remove();}
    for(const batch of panel.batches.values())batch.dispose();
  };
  Aplicacion.prototype.dispose = function () {
    this.detener();for(const [element,panel] of this._panels){this._disposePanel(panel);element._panelState=null;}this._panels.clear();
    this._animations.clear();this.animando=0;this.capas=[];this.globoDuenio=null;
    for(const batch of this._textBatches.values())batch.dispose();this._textBatches.clear();
    if(this.malla){this.malla.dispose();this.malla=null;}
    for(const pool of [this.textos,this.imagenes,this.blancos]){for(const item of pool.items)item.el.remove();pool.items.length=0;pool.usados=0;}
    this.raiz=null;
  };

  Aplicacion.prototype.correr = function () {
    this._running = true;
    if (this.sucio || Date.now() < this.animando) this._programar();
    return this;
  };
  Aplicacion.prototype.detener = function () {
    this._running = false;
    if (this._frame !== null) cancelAnimationFrame(this._frame);
    this._frame = null;
    if (this._globoTimer !== undefined) clearTimeout(this._globoTimer);
    this._globoTimer = undefined;
    return this;
  };

  // A batch owns one mesh/material, not one node per glyph. Rectangular clips
  // modify both positions and UVs so partially visible glyphs remain correct.
  function transparent(colors){for(let i=3;i<colors.length;i+=4)if(colors[i]<1)return true;return false;}
  function TexturedBatch(parent,source) {
    this.node = root.createElement("model");
    this.node.setAttribute("texture",source);
    this.node.setAttribute("texture-fit","stretch");
    this.node.setAttribute("material-alpha","blend");
    this.node.setAttribute("material-unlit","true");
    this.node.setAttribute("touchable","false");
    parent.appendChild(this.node); this.resource=null; this.begin();
  }
  TexturedBatch.prototype.begin = function () {
    this.positions=[];this.indices=[];this.normals=[];this.uvs=[];this.colors=[];
  };
  TexturedBatch.prototype.quad = function (x,y,w,h,u,v,uw,vh,color,z,geometry) {
    const r=geometry.aplicarClip(x,y,w,h); if (!r || w<=0 || h<=0) return;
    const u0=u+(r.x-x)/w*uw, v0=v+(r.y-y)/h*vh;
    const u1=u+(r.x+r.w-x)/w*uw, v1=v+(r.y+r.h-y)/h*vh;
    const n=this.positions.length/3, c=U.rgba(color);
    this.positions.push(r.x,-r.y,z,r.x+r.w,-r.y,z,r.x+r.w,-r.y-r.h,z,r.x,-r.y-r.h,z);
    this.indices.push(n,n+2,n+1,n,n+3,n+2);
    this.uvs.push(u0,v0,u1,v0,u1,v1,u0,v1);
    for(let i=0;i<4;i++){this.normals.push(0,0,1);this.colors.push(...c);}
  };
  TexturedBatch.prototype.commit = function () {
    if (!this.indices.length) { this.node.setAttribute("visible","false"); return; }
    this.node.setAttribute("visible","true");
    const data={positions:this.positions,indices:this.indices,normals:this.normals,uvs:this.uvs,colors:this.colors};
    if(this.resource) this.resource.update(data);
    else {this.resource=MeshResource.create(data);this.node.src=this.resource.src;}
  };
  TexturedBatch.prototype.dispose = function () {if(this.resource)this.resource.dispose();this.node.remove();};
  U.TexturedBatch=TexturedBatch;

  U.Aplicacion = Aplicacion;
  U.PASO_Z = PASO_Z;
})();


// ──────────────────────────────────────────────────────────────────────
// superpuestos.js
// ──────────────────────────────────────────────────────────────────────
// superpuestos.js — la capa de arriba: desplegables, globos y diálogos.
//
// Todo lo que había hasta acá vive **adentro** del rectángulo de la aplicación y
// se acomoda con sus hermanos. Un desplegable no: se abre encima, tapa lo que
// haya debajo, y si no entra hacia abajo se abre hacia arriba. Un globo de ayuda
// tampoco: aparece al lado del control y no empuja a nadie.
//
// En WPF eso es el `Popup`, que no es un control sino **otra ventana**. Acá no
// hay ventanas, pero hay algo equivalente y más barato: la pasada emite la malla
// en orden y `g.z` sube a medida que avanza, así que **emitir después es estar
// más adelante**. Una capa superpuesta es simplemente lo último que se emite,
// con un salto de z que la despega de todo el resto.
//
// Eso alcanza también para el toque, y no por casualidad: el rayo se queda con
// el nodo tocable más cercano, así que el blanco de un desplegable abierto le
// gana al del control que quedó debajo. No hay que desactivar nada.
//
// Lo que **no** se puede hacer, medido: un velo semitransparente. Un `<plane>`
// toma un color plano y `material-alpha` mira el alfa de la textura, que acá no
// existe. Así que un diálogo modal oscurece con un rectángulo **opaco** —que es
// como se ve un modal de todos modos— y un desplegable no pone velo: se cierra
// eligiendo, o volviendo a tocar el control.
(function () {
  "use strict";

  const U = globalThis.UI;
  const heredar = U.heredar;
  const num = U.numero;
  const grosor = U.grosor;
  const anchoDe = U.anchoDe;
  const altoDe = U.altoDe;
  const medirTexto = U.medirTexto;
  const C = (U.CFG && U.CFG.C) || {};

  /** El salto de z entre capas. Muy por encima de PASO_Z, que separa las partes
   *  de un mismo control: una capa tiene que despegarse de **todo** el árbol de
   *  abajo, no del último rectángulo emitido. */
  const SALTO_CAPA = 0.02;

  /** `marco` toma un grosor de los cuatro lados, no un numero. */
  const GROSOR_BORDE = grosor("0.003");
  const GROSOR_GLOBO = grosor("0.002");

  // ── El registro de capas ──────────────────────────────────────────────────

  /** Abrir una capa.
   *
   *  @param el     un árbol de elementos ya inflado
   *  @param op.x,y esquina superior izquierda pedida, en coordenadas de la
   *                aplicación. Se corrige para que la capa no se salga.
   *  @param op.ancho,alto  el espacio máximo que se le ofrece al medir
   *  @param op.modal       oscurecer y tragarse los toques de abajo
   *  @param op.duenio      quién la abrió, para poder cerrarla por dueño
   *  @param op.alCerrar    aviso, una sola vez
   *  @return un identificador para `cerrar` */
  U.Aplicacion.prototype.superponer = function (el, op) {
    const o = op || {};
    if (!this.capas) this.capas = [];
    el.app = this;
    this.capaSiguiente = (this.capaSiguiente || 0) + 1;
    const capa = {
      id: this.capaSiguiente,
      el: el,
      x: o.x || 0,
      y: o.y || 0,
      ancho: o.ancho || this.ancho,
      alto: o.alto || this.alto,
      modal: !!o.modal,
      centrar: !!o.centrar,
      alturaAncla: o.alturaAncla,
      duenio: o.duenio || null,
      alCerrar: o.alCerrar || null,
    };
    this.capas.push(capa);
    this.invalidar();
    return capa.id;
  };

  /** Cerrar por id, por dueño, o todas. */
  U.Aplicacion.prototype.cerrarCapa = function (que) {
    if (!this.capas || !this.capas.length) return false;
    const quedan = [];
    let cerradas = 0;
    for (let i = 0; i < this.capas.length; i++) {
      const c = this.capas[i];
      const coincide = que === undefined || que === null ? true
                     : typeof que === "number" ? c.id === que
                     : c.duenio === que;
      if (!coincide) { quedan.push(c); continue; }
      cerradas++;
      if (c.alCerrar) c.alCerrar();
    }
    this.capas = quedan;
    if (cerradas) this.invalidar();
    return cerradas > 0;
  };

  U.Aplicacion.prototype.tieneCapa = function (duenio) {
    if (!this.capas) return false;
    for (let i = 0; i < this.capas.length; i++) {
      if (this.capas[i].duenio === duenio) return true;
    }
    return false;
  };

  /** Lo que llama el host después de emitir el árbol principal.
   *
   *  Cada capa se mide y se acomoda **acá**, no en la pasada de layout: su
   *  posición depende de dónde quedó el control que la abrió, y eso recién se
   *  sabe cuando el árbol ya se acomodó. */
  U.Aplicacion.prototype.emitirCapas = function (ctx) {
    // Se crea acá y no en el constructor para no tocar host.js, que no tiene por
    // qué saber que existen las capas. Después de la primera pasada el arreglo
    // está siempre, que es lo que esperan los que lo consultan.
    if (!this.capas) this.capas = [];
    if (!this.capas.length) return;

    for (let i = 0; i < this.capas.length; i++) {
      const c = this.capas[i];
      const el = c.el;

      // Los enlaces tambien valen en una capa: un dialogo que muestra un dato
      // del modelo es el caso normal, no la excepcion.
      el.aplicarEnlaces(this.datos, this);
      el.medir(c.ancho, c.alto);
      const w = Math.min(el.deseado.w, c.ancho);
      const h = Math.min(el.deseado.h, c.alto);

      // Que no se salga del panel. Primero se prueba tal cual; si no entra hacia
      // abajo, se vuelca hacia arriba del ancla — que es lo que hace cualquier
      // desplegable cerca del borde inferior de una pantalla.
      let x = Math.max(0, Math.min(c.x, this.ancho - w));
      let y = c.y;
      if (c.centrar) {
        // El centrado se resuelve aca y no al abrir: el tamanio de la capa sale
        // de medirla, y medirla necesita el espacio que recien se conoce ahora.
        x = Math.max(0, (this.ancho - w) / 2);
        y = Math.max(0, (this.alto - h) / 2);
      } else if (y + h > this.alto) {
        const arriba = (c.alturaAncla !== undefined ? c.y - c.alturaAncla : c.y) - h;
        y = arriba >= 0 ? arriba : Math.max(0, this.alto - h);
      }

      ctx.g.z += SALTO_CAPA;

      if (c.modal) {
        // El velo. Opaco, porque no hay alfa: oscurece el fondo con un
        // rectángulo del color de la sala y se traga el toque.
        const self = this;
        const idCapa = c.id;
        ctx.g.rect(0, 0, this.ancho, this.alto, U.tinte(C.fondo || "#08080C", 0.85));
        ctx.blanco(0, 0, this.ancho, this.alto, U.tinte(C.fondo || "#08080C", 0.85),
                   function (app) { self.cerrarCapa(idCapa); void app; }, null);
        ctx.g.z += SALTO_CAPA;
      }

      el.acomodar(x, y, w, h);
      el.emitir(ctx);
    }
  };

  // ── ComboBox ──────────────────────────────────────────────────────────────
  //
  // El desplegable. Cerrado es un botón que dice qué está elegido; abierto es
  // una capa con una fila por opción.
  //
  // Las opciones se declaran como texto separado por `;` —`Items="Bajo;Medio;
  // Alto"`— o se enlazan a un arreglo de strings. No acepta plantillas todavía:
  // un ComboBox con plantilla es un ListBox con más pasos, y el ListBox no está.

  function ComboBox(a) {
    U.Interactivo.call(this, a);
    this.opciones = ComboBox.leerOpciones(a.Items);
    this.elegida = num(a.SelectedIndex, 0);
    this.tam = num(a.FontSize, 0.03);
    this.fondo = a.Background || C.superficieAlta || "#2A2A38";
    this.color = a.Foreground || C.texto || "#FFFFFF";
    this.acento = a.Accent || C.azul || "#0A84FF";
    this.radio = num(a.CornerRadius, 0.01);
    this.relleno = grosor(a.Padding || "0.016,0.01");
    this.alCambiar = a.SelectionChanged || null;
  }
  heredar(ComboBox, U.Interactivo);

  ComboBox.leerOpciones = function (v) {
    if (Array.isArray(v)) return v.map(String);
    if (typeof v === "string" && v.length) return v.split(";");
    return [];
  };

  /** El ancho es el de la opción más larga, no el de la elegida: si cambiara de
   *  tamaño al elegir, el panel entero se reacomodaría a cada selección. */
  ComboBox.prototype.medirContenido = function () {
    let w = 0;
    for (let i = 0; i < this.opciones.length; i++) {
      const m = medirTexto(this.opciones[i], this.tam);
      if (m.w > w) w = m.w;
    }
    return {
      w: w + anchoDe(this.relleno) + this.tam * 1.4,
      h: this.tam + altoDe(this.relleno),
    };
  };

  ComboBox.prototype.abierto = function () {
    return !!(this.app && this.app.tieneCapa(this));
  };

  ComboBox.prototype.alternar = function (app) {
    if (app.tieneCapa(this)) { app.cerrarCapa(this); return; }
    // Una sola capa de desplegable a la vez: dos abiertos se pisarían y el de
    // abajo quedaría inalcanzable.
    app.cerrarCapa();

    const lista = new ListaDeOpciones({
      opciones: this.opciones,
      elegida: this.elegida,
      tam: this.tam,
      fondo: this.fondo,
      color: this.color,
      acento: this.acento,
      radio: this.radio,
      relleno: this.relleno,
      Width: String(this.caja.w),
      duenio: this,
    });

    app.superponer(lista, {
      x: this.caja.x,
      y: this.caja.y + this.caja.h + 0.004,
      alturaAncla: this.caja.h + 0.008,
      ancho: app.ancho,
      alto: app.alto,
      duenio: this,
    });
  };

  ComboBox.prototype.elegir = function (i, app) {
    if (i === this.elegida) return;
    this.elegida = i;
    this.devolver(app.datos, "SelectedIndex", i);
    const h = this.manejador(app, this.alCambiar);
    if (h) h(this, app, this.opciones[i]);
  };

  ComboBox.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const self = this;

    if (this.habilitado) {
      ctx.blanco(b.x, b.y, b.w, b.h, this.fondo, function (app) {
        self.marcarToque(app);
        self.alternar(app);
      }, this);
      ctx.subir();
    }

    const k = this.realce();
    const fondo = !this.habilitado ? U.tinte(this.fondo, 0.45)
                : k > 0 ? U.tinte(this.fondo, 1 + k) : this.fondo;
    ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, this.radio, fondo);
    ctx.subir();

    // La flecha. Dos triángulos no: un chevrón es un triángulo, y como
    // `Geometria` sólo sabe de rectángulos, se arma con tres rectángulos
    // escalonados. A este tamaño se lee igual y no agrega API nueva.
    const t = this.tam * 0.5;
    const cx = b.x + b.w - anchoDe(this.relleno) / 2 - t / 2;
    const cy = b.y + b.h / 2;
    const abierto = this.abierto();
    for (let i = 0; i < 3; i++) {
      const paso = t / 3;
      const ancho = t - i * paso;
      const dy = abierto ? (2 - i) * paso * 0.5 : i * paso * 0.5;
      ctx.g.rect(cx - ancho / 2, cy - t * 0.25 + dy, ancho, paso * 0.6, this.color);
    }
    ctx.subir();

    const texto = this.opciones[this.elegida];
    if (texto !== undefined) {
      ctx.texto(b.x + this.relleno.i + medirTexto(texto, this.tam).w / 2,
                b.y + b.h / 2, texto, this.tam,
                this.habilitado ? this.color : U.tinte(this.color, 0.55));
      ctx.subir();
    }
  };

  ComboBox.prototype.mapa = {
    Items: "opciones", SelectedIndex: "elegida", Background: "fondo",
    Foreground: "color", IsEnabled: "habilitado",
  };
  U.tipos.ComboBox = ComboBox;

  // ── La lista que se abre ──────────────────────────────────────────────────
  // No es un tipo del marcado: la arma el ComboBox y vive sólo en la capa.

  function ListaDeOpciones(a) {
    U.Elemento.call(this, a);
    this.opciones = a.opciones;
    this.elegida = a.elegida;
    this.tam = a.tam;
    this.fondo = a.fondo;
    this.color = a.color;
    this.acento = a.acento;
    this.radio = a.radio;
    this.relleno = a.relleno;
    this.duenio = a.duenio;
  }
  heredar(ListaDeOpciones, U.Elemento);

  ListaDeOpciones.prototype.altoFila = function () {
    return this.tam + altoDe(this.relleno);
  };

  ListaDeOpciones.prototype.medirContenido = function () {
    let w = 0;
    for (let i = 0; i < this.opciones.length; i++) {
      const m = medirTexto(this.opciones[i], this.tam);
      if (m.w > w) w = m.w;
    }
    return {
      w: w + anchoDe(this.relleno),
      h: this.altoFila() * this.opciones.length + 0.008,
    };
  };

  ListaDeOpciones.prototype.dibujar = function (ctx) {
    const b = this.caja;
    const fila = this.altoFila();
    const self = this;

    ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, this.radio, U.tinte(this.fondo, 0.8));
    ctx.subir();
    ctx.g.marco(b.x, b.y, b.w, b.h, GROSOR_BORDE, this.acento);
    ctx.subir();

    for (let i = 0; i < this.opciones.length; i++) {
      const y = b.y + 0.004 + i * fila;
      const elegida = i === this.elegida;

      (function (indice) {
        ctx.blanco(b.x + 0.004, y, b.w - 0.008, fila,
                   elegida ? self.acento : self.fondo,
                   function (app) {
                     self.duenio.elegir(indice, app);
                     app.cerrarCapa(self.duenio);
                   }, null);
      })(i);
      ctx.subir();

      if (elegida) {
        ctx.g.rectRedondeado(b.x + 0.004, y, b.w - 0.008, fila, this.radio * 0.6, this.acento);
        ctx.subir();
      }
      ctx.texto(b.x + this.relleno.i + medirTexto(this.opciones[i], this.tam).w / 2,
                y + fila / 2, this.opciones[i], this.tam, this.color);
      ctx.subir();
    }
  };

  // ── ToolTip ───────────────────────────────────────────────────────────────
  //
  // Un globo de ayuda al pasar por encima. Existe **porque ahora hay hover**: el
  // motor manda `pointerenter` / `pointerleave` desde
  // `feat(input): add HTML-like hover`, y el host ya los usa para prender
  // `Interactivo.encima`. Sin eso esto no se podría hacer — un globo que
  // apareciera al tocar llegaría siempre tarde, porque el toque ya hizo lo suyo.
  //
  // No es un control sino un atributo de cualquier interactivo: `ToolTip="..."`.

  function Globo(a) {
    U.Elemento.call(this, a);
    this.texto = a.texto;
    this.tam = a.tam || 0.024;
    this.fondo = a.fondo || C.fondo || "#08080C";
    this.color = a.color || C.texto || "#FFFFFF";
    this.borde = a.borde || C.borde || "#5A5A70";
    this.relleno = grosor("0.012,0.007");
  }
  heredar(Globo, U.Elemento);

  Globo.prototype.medirContenido = function () {
    const m = medirTexto(this.texto, this.tam);
    return { w: m.w + anchoDe(this.relleno), h: m.h + altoDe(this.relleno) };
  };

  Globo.prototype.dibujar = function (ctx) {
    const b = this.caja;
    ctx.g.rectRedondeado(b.x, b.y, b.w, b.h, 0.006, this.fondo);
    ctx.subir();
    ctx.g.marco(b.x, b.y, b.w, b.h, GROSOR_GLOBO, this.borde);
    ctx.subir();
    ctx.texto(b.x + b.w / 2, b.y + b.h / 2, this.texto, this.tam, this.color);
    ctx.subir();
  };

  /** El seguimiento del hover. Se hace en la pasada y no con un evento porque el
   *  host reasigna los blancos en cada pasada: `encima` es del control, y el
   *  control es lo único estable entre pasada y pasada. */
  const MS_GLOBO = 350;

  U.Aplicacion.prototype.revisarGlobos = function () {
    const raiz = this.raiz;
    if (!raiz) return;
    const self = this;
    let encontrado = null;

    (function recorrer(el) {
      if (!el.visible) return;
      if (el.ayuda && el.encima) encontrado = el;
      for (let i = 0; i < el.hijos.length; i++) recorrer(el.hijos[i]);
    })(raiz);

    if (encontrado !== this.globoDuenio) {
      if (this.globoCapa) { this.cerrarCapa(this.globoCapa); this.globoCapa = 0; }
      this.globoDuenio = encontrado;
      this.globoDesde = encontrado ? Date.now() : 0;
      if (this._globoTimer !== undefined) clearTimeout(this._globoTimer);
      this._globoTimer = undefined;
      if (encontrado) this._globoTimer = setTimeout(function () {
        self._globoTimer = undefined;
        if (self.globoDuenio === encontrado && encontrado.encima) self.invalidateRender();
      }, MS_GLOBO);
    }

    // La demora existe para que pasar el puntero por encima de una fila de
    // botones no dispare seis globos en el camino.
    if (encontrado && !this.globoCapa && Date.now() - this.globoDesde >= MS_GLOBO) {
      const b = encontrado.caja;
      const globo = new Globo({ texto: encontrado.ayuda });
      this.globoCapa = this.superponer(globo, {
        x: b.x,
        y: b.y + b.h + 0.006,
        alturaAncla: b.h + 0.012,
        duenio: globo,
        alCerrar: function () { self.globoCapa = 0; },
      });
    }
  };

  U.Globo = Globo;
  U.ComboBox = ComboBox;
})();
