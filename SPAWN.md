# Puntos de aparición

```xml
<hsml>
  <space resources="spawn,navigate_self,desktop_camera_control,vr_locomotion">
    <spawn id="entrada" default="true" x="0" y="0" z="4" ry="0"/>
    <group x="10" y="2" z="0" ry="1.5707963">
      <spawn id="puerta-sur" z="2"/>
    </group>
    <!-- contenido del mundo -->
  </space>
</hsml>
```

`spawn` es un marcador sin malla ni hitbox. Su posición corresponde a los pies;
`ry` está en radianes y cero mira hacia -Z. Se respeta la transformación de sus
grupos y del montaje. Sólo se utiliza la dirección horizontal: no inclina el
visor. En escritorio los ojos quedan a 1,7 m del punto; en VR se conserva la
altura física y se compensa el desplazamiento del visor respecto al tracking
origin. Los mundos deben mantener su escala en metros.

## Permiso y autoridad

El documento debe solicitar `spawn` en `resources` y el shell debe concederlo.
Los montajes `spatial` lo ofrecen por defecto. Una lista `grants` personalizada
puede excluirlo, incluso si el documento lo solicita. No requiere un diálogo de
consentimiento: es una capacidad limitada a la llegada a ese mundo, no una API
general para teletransportar al usuario.

Sólo participan marcadores del documento principal de un montaje espacial
visible, directamente bajo el root del shell. Los includes anidados y espacios
anidados quedan excluidos, al igual que las apps, apps embebidas y el ambiente
compartido. Ni siquiera una concesión explícita de `spawn` les permite mover al
visitante. Compartir origen no concede autoridad sobre el spawn principal:
el documento principal debe declarar el punto, por ejemplo al lado del objeto
incluido. No hay una excepción automática basada en el dominio.

## Selección y ciclo de vida

- `https://ejemplo.test/mundo.hsml#entry=puerta-sur` selecciona un `id`.
- Sin coincidencia se utiliza `default="true"`; sin default, el primer marcador
  por orden de creación. Conviene declarar un único default y usar IDs únicos.
- La aparición se aplica una sola vez por instancia del documento. Mover,
  eliminar o recrear marcadores después no vuelve a mover al visitante.
- Se espera a que el include principal haya confirmado la carga de su URL y a
  que exista la cámara o el tracking VR. No se espera a todas las mallas GLB.
- No se aplican marcadores de una URL anterior mientras carga otra navegación.
- Sin marcadores autorizados se conserva la posición actual. Ocultar/mostrar
  un documento ya utilizado no repite su aparición.
- Puede crearse el primer marcador desde JS: se aplica cuando aparece. Para
  evitar saltos tardíos, se recomienda declararlo directamente en el HSML.
- Una transformación no finita o sin dirección horizontal se rechaza y deja un
  aviso en los logs. No se busca suelo ni se detectan obstáculos: el autor debe
  colocar el marcador en una zona libre.

Esta etapa no implementa historial espacial, restauración con Atrás ni puertas
con contexto de retorno. Un cambio de fragmento tampoco constituye por sí solo
una orden de teletransporte: el spawn sigue siendo único por carga.

## Verificación

`cargo test -p luna --lib player_spawn` cubre petición y concesión del permiso,
exclusión de apps/includes, prioridad del default, aplicación única, navegación
pendiente, visibilidad y matemáticas de transformación y altura VR. La prueba
física con visor sigue siendo necesaria para validar la experiencia real.
