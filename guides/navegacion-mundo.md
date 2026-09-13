# Navegar el mundo desde un include

Una puerta puede cambiar el documento espacial que la contiene sin enviar un
callback a su padre:

```js
hiperspace.world.navigate('https://mundos.example/cueva.hsml#entry=entrada');
```

La llamada reemplaza el include principal del montaje espacial del emisor.
Conserva el shell, la pestaña y las apps abiertas. No busca otra pestaña ni
permite que una app o app embebida navegue el mundo de al lado.

## Declaración y permiso

El mundo pide `navigate_world` y lo delega explícitamente a la puerta:

```xml
<space resources="navigate_self,navigate_world">
  <include src="/componentes/puerta.hsml" resources="navigate_world"/>
</space>
```

El documento `puerta.hsml`:

```xml
<hsml>
  <space resources="navigate_world">
    <box id="puerta" y="1" z="-2" sx="1" sy="2" sz="0.15"
         color="#407080" touchable="true"/>
    <script>
      const puerta = hiperspace.dimention.getElementById('puerta');
      puerta.addEventListener('toque', () => {
        hiperspace.world.navigate('/cueva.hsml#entry=entrada');
      });
    </script>
  </space>
</hsml>
```

Los montajes `spatial` incluyen `navigate_world` en sus concesiones predeterminadas.
Si el documento declara una lista de `resources`, debe incluirlo para usarlo o
delegarlo. Cada include intermedio debe volver a delegarlo: tener el mismo origen
no evita esta regla. También puede delegarse a una puerta de otro origen; hacerlo
autoriza a ese componente a cambiar el mundo completo.

No requiere un diálogo nuevo. Se aplica la intersección de capacidades del padre
y del include. `app` y `app-embedded` no reciben esta concesión por defecto y el
host rechaza la operación desde esos montajes incluso si se les otorgara el bit.
El destino se comprueba contra la jerarquía del shell real; atributos parecidos
dentro de un documento remoto no crean un montaje privilegiado.

## URL, llegada y resultado

- El argumento es una cadena URL. Las rutas relativas se resuelven respecto del
  documento de la puerta, igual que su `location`, no respecto del mundo padre.
  Para un componente alojado en otro servidor conviene pasar un destino absoluto.
- `#entry=nombre` utiliza el mecanismo existente de [spawn](llegada.md). El
  documento de destino debe declarar su spawn y permiso correspondiente.
- Navegar al mismo URL ya cargado vuelve a cargar el documento. Una petición
  idéntica que todavía está en vuelo se mantiene agrupada.
- La llamada encola una solicitud y retorna `undefined`. Un tipo incorrecto, URL
  vacía o mayor a 16 KiB lanza una excepción. El host registra los rechazos de
  permiso o montaje en sus logs. No es una promesa de carga completada.
- Si el mismo isolate solicita varios destinos antes del siguiente tick del host,
  sólo se conserva el último.

`location.href` y `location.assign()` conservan su navegación local al include.
Tener ambos permisos no cambia el destino: la API elegida lo determina.
`navigate_global` continúa siendo navegación del shell, un privilegio distinto.

Esta API todavía no añade historial, retorno automático, `arrival`, opciones
`replace` ni teleport dentro del documento. La propuesta extensa de
`disenos/navegacion-de-mundo.md` es un antecedente; el contrato implementado es el
de esta guía.
