Reporte técnico: estrategia de runtime optimizado para avatares y lógica procedural

Proyecto: Luna / runtime social 3D
Objetivo del documento: dejar una referencia consolidada para implementar más adelante un sistema de avatares/lógica custom de alto rendimiento, evitando depender del historial de la conversación.
Fecha: 2026-04-13

1. Resumen ejecutivo

La estrategia más prometedora para escalar avatares y lógica custom en Luna es:

sacar a JS del hot path
manejar avatares con una entidad/runtime especializado en Rust
usar un sistema de scripting custom muy restringido solo para lógica matemática/procedural local
no sincronizar huesos completos por red salvo casos excepcionales
usar LOD de animación, budget por avatar y scheduler adaptativo
Conclusión principal

El mejor equilibrio entre rendimiento, control y extensibilidad no es:

JS controlando huesos por frame
ni binarios arbitrarios de usuarios
ni scripting libre estilo “puede hacer cualquier cosa”

Sino algo como:

un runtime procedural de avatar / instancia, muy capado, con inputs y outputs tipados, ejecutado fuera de JS y gobernado por Rust.

2. Contexto de rendimiento base

Se parte de una observación muy valiosa:

Escena base

En una escena de tamaño promedio tipo VRChat, el sistema actual logra aproximadamente:

400 FPS

Eso equivale a:

2.5 ms por frame
Presupuesto libre aproximado para avatares

Si la escena base ya consume ~2.5 ms:

Objetivo FPS	Presupuesto total/frame	Margen aprox. para avatares
120 FPS	8.3 ms	~5.8 ms
90 FPS	11.1 ms	~8.6 ms
60 FPS	16.6 ms	~14.1 ms

Interpretación: el entorno base no parece ser el problema; el cuello futuro estará casi totalmente en avatares, red, animación, scripting y materiales.

3. Escenarios evaluados

Se discutieron varios enfoques para manejar avatares.

Escenario 1 — <avatar/> especializado, manejado totalmente por Rust

Características:

entidad/runtime propio para avatares
JS se evita en lo posible
Rust controla:
red
animation state
interpolación
LOD
culling
blend de animaciones
scheduling
Evaluación

Este es el camino recomendado base.

Estimaciones
Todos cerca / todos importantes
FPS objetivo	Avatares estimados
120 FPS	30–60
90 FPS	45–80
60 FPS	70–130
Escena mixta con LOD serio
FPS objetivo	Avatares estimados
120 FPS	70–140
90 FPS	100–180
60 FPS	150–280
Conclusión

Es la mejor base práctica y la más segura para producción.

Escenario 2 — avatares manejados desde JS

Características:

JS instancia o controla el avatar
JS pasa posiciones/transformaciones de huesos o parámetros de animación
hay puente JS ↔ host ↔ render
Evaluación

Es claramente peor para escalar.

Problemas
marshalling entre runtimes
copias
validación
más jitter
peores picos
más trabajo por avatar en CPU
difícil de presupuestar fino
Estimaciones
Todos cerca / full-ish
FPS objetivo	Avatares estimados
120 FPS	8–18
90 FPS	12–25
60 FPS	20–45
Mixto con reducción de update rate
FPS objetivo	Avatares estimados
120 FPS	15–35
90 FPS	25–50
60 FPS	40–80
Conclusión

No recomendado como camino principal para avatares.

Escenario 3 — pipeline procedural nativo sin JS

Interpretación correcta del caso:

no es un “shader con socket” literal
la red entra a CPU
Rust decodifica inputs
un runtime/VM simple calcula parámetros
Rust/GPU aplican animación/skinning
JS queda fuera
Evaluación

Muy bueno, pero más complejo que el escenario 1.

Estimaciones
Todos cerca / full-ish
FPS objetivo	Avatares estimados
120 FPS	40–75
90 FPS	60–100
60 FPS	90–160
Mixto con LOD fuerte
FPS objetivo	Avatares estimados
120 FPS	90–170
90 FPS	130–240
60 FPS	200–350
Conclusión

Puede superar al escenario 1 si se implementa muy bien, pero requiere más ingeniería.

Escenario 4 — arquitectura híbrida óptima

Combinación recomendada:

Rust posee el runtime del avatar
JS solo participa en lógica de alto nivel si hace falta
red manda estado compacto, no huesos completos
LOD de animación
shadows budget fijo
materiales simples
Estimaciones
Cerca / importante
FPS objetivo	Avatares estimados
120 FPS	40–70
90 FPS	60–100
60 FPS	90–160
Mixto
FPS objetivo	Avatares estimados
120 FPS	100–180
90 FPS	140–240
60 FPS	220–350
Conclusión

Objetivo ideal a mediano plazo.

4. Nueva estrategia refinada: runtime procedural custom para avatares/instancias

La idea refinada fue:

no usar JS en el hot path
permitir cierta customización por usuario/avatar
pero mediante un runtime extremadamente controlado
más cercano a una Avatar VM / DSL / bytecode que a un lenguaje general
5. Terminología correcta

Aunque inicialmente se habló de “shader”, lo más preciso para esta idea sería llamarlo:

Avatar VM
Avatar DSL
Pose Program
Rig Logic Bytecode
Procedural Instance Runtime

Porque no es un shader GPU literal, sino:

un pequeño programa validado, acotado, determinista, que transforma inputs tipados en outputs tipados.

6. Conclusión principal sobre esta estrategia
Sí, es viable

Y puede acercarse mucho al rendimiento de un sistema nativo en Rust.

Pero solo si es hiperrestringida

Si se vuelve demasiado libre, se cae en el mismo problema estructural de plataformas UGC como VRChat:

presupuesto impredecible
picos
dificultad de profiling
difícil control del coste por avatar
degradación progresiva del frame time
7. Regla central de diseño

No hacer un lenguaje libre para avatares.
Hacer una VM/DSL acotada para control procedural de instancias.

8. Qué NO debe permitirse
Prohibido
acceso directo a sockets desde el script
acceso directo al mundo/escena
acceso a otras entidades arbitrarias
spawn/despawn
creación de materiales
modificación libre de atributos string
acceso a archivos
syscalls
threads
strings
heap o alloc libre
tablas/maps dinámicos sin límite
reflection
loops arbitrarios
recursión
imports arbitrarios
binarios nativos de usuarios
Conclusión sobre binarios nativos de usuario

Aunque “serían rápidos”, no se recomienda permitir:

.dll
.so
plugins nativos
código arbitrario compilado

Motivos:

seguridad
estabilidad
crashes
cheating
portabilidad
sandboxing difícil
pérdida total de control presupuestario
9. Qué SÍ puede permitirse
Inputs tipados, solo lectura

Ejemplos:

dt
time
lod
distance
root_pos
root_rot
velocity
clip_id
clip_phase
gesture_l
gesture_r
voice_level
look_target
flags
params[n]
net_channels[n]
Outputs tipados, alcance limitado

Ejemplos:

blend weights
look yaw / pitch
face weights
emote weights
additive rotations limitadas
material params limitados
visibility flags
attachment params
scalar/vector channels
10. Decisión clave: no dejar que el script “posea el esqueleto completo”

Se discutieron dos modelos.

Modelo A — el programa solo emite parámetros

Ejemplo:

locomotion weight
gesture blend
look offsets
facial weights
algunos offsets aditivos

Después:

Rust mezcla animaciones
Rust aplica constraints
Rust compone pose
GPU hace skinning
Evaluación

Es el mejor camino.

Modelo B — el programa emite transform de todos los huesos

Ejemplo:

posición/rotación de 60 huesos por frame
Evaluación

Puede funcionar, pero:

escala peor
requiere más validación
más coste de escritura y aplicación
más difícil presupuestar
Recomendación

Solo para casos especiales o cercanos, no como modo principal.

11. Ranking recomendado de runtimes
1. DSL / bytecode propio

Mejor opción para el hot path.

Ventajas:

determinista
sin GC
sin heap
fácil de validar
fácil de presupuestar
fácil de limitar por instrucciones

Desventaja:

hay que diseñarlo
2. WASM muy capado

Aceptable, si se controla duro:

imports limitados
memoria limitada
sin acceso arbitrario al host

Aun así, es más generalista de lo ideal.

3. Lua

Útil para scripting moderado, pero no ideal como núcleo del hot path.

Problemas:

dinámico
tablas
GC
menos control exacto del presupuesto
Recomendación

Si se usa Lua, que sea para:

eventos
emotes
triggers
lógica de alto nivel

No como núcleo de la lógica por-avatar por-frame.

4. Binarios nativos de usuarios

No recomendado.

12. “Es solo matemática” — aclaración importante

Se concluyó que aunque el programa solo haga “cálculo matemático”, eso igualmente puede volverse costoso por:

multiplicación por cantidad de avatares
picos, no solo promedio
acceso a memoria dispersa
demasiadas salidas
demasiadas ramas
composición libre de muchas pequeñas lógicas
crecimiento gradual del script hasta convertirse en un mini motor
Conclusión

Un simple timeout o “tiempo máximo” ayuda, pero no es suficiente como mecanismo principal.

13. Presupuesto estructural recomendado

Además del control por tiempo, debe existir un presupuesto estático/determinista.

Recomendaciones
máximo de instrucciones
máximo de registros
máximo de estado persistente
máximo de channels/huesos afectados
máximo de outputs
sin heap
sin loops arbitrarios
sin recursión
Ejemplo orientativo
128–512 instrucciones máximas
16–64 registros
1–4 KB de estado persistente
8–32 outputs principales
offsets aditivos en un subconjunto pequeño de huesos
14. Scheduler adaptativo recomendado

Se discutió que bajar la frecuencia de ejecución según coste y distancia sí es una buena idea, pero como segunda capa, no como único mecanismo.

Capas recomendadas
Capa 1 — presupuesto duro

El programa nunca puede exceder ciertos límites estructurales.

Capa 2 — scheduler adaptativo

Se ejecuta a distinta frecuencia según:

distancia
importancia visual
coste medido
tier del programa
presupuesto disponible del frame
Política sugerida por LOD
Cerca
60 Hz
Media distancia
30 Hz
Lejos
10–15 Hz
Muy lejos
5 Hz o congelado
Política por clase de coste
Clase A — muy barato
60 / 30 / 15
Clase B — medio
30 / 15 / 5
Clase C — caro
15 / 5 / freeze
15. Generalización: no solo humanoides

La estrategia no debe diseñarse solo para humanos.

Puede servir para:
humanoides
criaturas
robots
props interactivos
armas
mascotas
hologramas
UI 3D viva
efectos de movimiento procedural
elementos decorativos con lógica local
Conclusión

Lo correcto es pensar en esto como un:

runtime procedural genérico para instancias animadas/controladas

16. Estimaciones revisadas para la Avatar VM / runtime custom

Partiendo del baseline de 400 FPS y del modelo refinado:

Caso óptimo real

Características:

runtime custom muy restringido
inputs y outputs tipados
sin JS
Rust sigue poseyendo el sistema de animación
el programa solo modula parámetros, no reemplaza todo el solver
Todos cerca / importantes
FPS objetivo	Avatares estimados
120 FPS	35–65
90 FPS	50–90
60 FPS	80–150
Mixto con LOD fuerte
FPS objetivo	Avatares estimados
120 FPS	90–170
90 FPS	130–230
60 FPS	200–320
Conclusión

Se acerca mucho al rendimiento de un sistema nativo.

Caso medio

Características:

runtime custom, pero escribe más cosas
más salidas
más offsets de huesos
más estado
Todos cerca
FPS objetivo	Avatares estimados
120 FPS	25–50
90 FPS	35–70
60 FPS	55–110
Mixto
FPS objetivo	Avatares estimados
120 FPS	60–120
90 FPS	90–170
60 FPS	140–240
Caso permisivo/malo

Características:

demasiado flexible
demasiadas operaciones
demasiadas salidas
estructura tipo lenguaje general
loops/tablas/dinámica o equivalente
control demasiado libre
Todos cerca
FPS objetivo	Avatares estimados
120 FPS	10–25
90 FPS	15–35
60 FPS	25–60
Conclusión

Se acerca peligrosamente al coste de JS o al coste impredecible UGC clásico.

17. Evaluación del objetivo “60–120 mixto y permisivo”

Se concluyó que:

60–120 avatares en escena mixta con un sistema relativamente permisivo ya sería un muy buen resultado.

Interpretación práctica

Si el sistema logra:

avatares custom
cierta lógica custom
red
mezcla de cercanos y lejanos
sin materiales absurdos
con scheduler y LOD

y aun así sostiene:

60–120 mixto

entonces el resultado ya es muy respetable y probablemente superior a la experiencia habitual de plataformas más abiertas.

18. Recomendaciones de red
No sincronizar huesos completos por red

Evitar mandar:

matrices de 50–100 huesos por avatar por frame
Mejor mandar estado compacto

Ejemplo:

root_pos
root_rot
velocity
clip_id
normalized_time
playback_speed
gesture/emote bits
look target
flags

Después cada cliente:

reconstruye la pose localmente
aplica procedural local
hace interpolación
Conclusión

Esto da una mejora grande en escalabilidad.

19. Recomendaciones de render y animación
Recomendaciones base
materiales simples
pocos materiales por avatar
sin shaders arbitrarios
sombras solo para pocos avatares cercanos
sin transparencias costosas en masa
LOD de malla
LOD de animación
reducción de update rate por distancia
Recomendación clave

Si no hay LOD o presupuesto suficiente, el avatar debe simplificarse agresivamente o desaparecer de la escena.

20. Filosofía anti-VRChat adoptada

La conversación dejó una conclusión fuerte:

El problema de VRChat no es solo Unity o la GPU

El problema es la combinación de:

contenido de usuario arbitrario
materiales/shaders variados
transparencias
múltiples materiales por avatar
comportamiento no acotado
coste CPU fragmentado
difícil imposición de presupuestos
Estrategia propuesta para evitar eso
sistema más cerrado
budgets duros
capabilities
materiales simples
lógica custom restringida
JS fuera del hot path
runtime nativo/VM controlada
LOD obligatorio
scheduler adaptativo
21. Recomendación final de arquitectura
Capa 1 — declarativa

HSML / DOM / includes / scripts

Sirve para:

estructura
navegación
UI espacial
permisos
recursos
configuración de entidades
Capa 2 — runtime nativo

Rust / componentes tipados

Sirve para:

avatares
networking
interpolación
animación
budgets
LOD
culling
Capa 3 — runtime procedural custom

Avatar VM / DSL / bytecode restringido

Sirve para:

modular parámetros del avatar o instancia
expresividad procedural
customización limitada por usuario
Regla

La capa declarativa no debe convertirse en el hot path.

22. Decisión recomendada para implementación futura
Camino recomendado
introducir <avatar/> especializado en Rust
sacar JS del hot path del avatar
no mandar huesos completos por red
diseñar un runtime procedural muy capado
hacer que ese runtime solo emita parámetros/offsets limitados
mantener a Rust como dueño de:
red
pose final
LOD
scheduling
budget
añadir un scheduler por distancia/coste
medir por avatar:
media
p95
p99
máximo
frecuencia actual
23. Antiobjetivos explícitos

No hacer:

scripting libre de avatar con acceso a todo
JS como hot path de skeleton
binarios nativos de usuario
sincronización de huesos por red como camino principal
permitir cualquier cantidad de materiales/shaders
usar solo “timeout” como defensa
depender solo del promedio de coste
24. Conclusión final
Mejor síntesis del documento

El sistema ideal no es un “scripting libre para avatares”, sino:

un runtime procedural pequeño, determinista y presupuestable, controlado por Rust y acoplado a un sistema de animación/LOD también nativo.

Resultado esperado si se ejecuta bien

Con la escena base actual (~400 FPS), y manteniendo control duro sobre contenido y budgets:

sistema nativo Rust o VM muy bien acotada:
80–150 avatares cercanos a 60 FPS
200–320 avatares mixtos con LOD fuerte
sistema moderadamente permisivo:
60–120 mixtos ya es un resultado muy bueno
sistema demasiado libre:
cae rápidamente hacia 25–60 o peor
Frase de diseño que resume todo

Hazlo genérico, pero no libre.
Hazlo expresivo, pero con presupuesto fijo.
Y usa bajar FPS como amortiguador, no como única defensa.

25. Resumen ultra corto para relectura rápida
Escena base actual: ~400 FPS
Mejor camino: <avatar/> nativo en Rust
JS: no en hot path
Red: no mandar huesos completos
Runtime custom: sí, pero muy restringido
Mejor forma: DSL/bytecode propio
Lua: útil para cosas secundarias, no núcleo del hot path
Binarios nativos de usuario: no
Outputs del runtime: parámetros, blends, offsets limitados
Rust sigue siendo dueño de pose, LOD, scheduler y budget
Objetivo realista muy bueno:
60–120 mixtos con sistema permisivo controlado
200–320 mixtos en versión muy optimizada/restringida

Si quieres, en otro mensaje puedo convertir este reporte en una versión más “de especificación”, con secciones como:

Objetivos
No objetivos
API propuesta
Budget por programa
Formato de inputs/outputs
Roadmap de implementación