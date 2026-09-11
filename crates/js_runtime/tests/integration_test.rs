use js_runtime::run_js; // <- Ajusta al nombre real de tu crate
use anyhow::Result;
use serde_json::Value;

#[tokio::test] // Requiere tokio en dev-dependencies
async fn integration_test_run_js() -> Result<()> {
    let script = r#"
        (function() {
            return [1, 2, 3].map(x => x * 2);
        })();
    "#;
    let result = run_js(script)?;
    assert_eq!(result, Value::from(vec![2, 4, 6]));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use js_runtime::Engine;
    use serde_json::Value;
    use std::time::{Duration, Instant};

    #[test]
    fn vector_assignments_capture_values_and_are_readable_before_host_commit() -> Result<()> {
        let mut eng = Engine::new();
        eng.eval(r#"
            const vector = { x: 1, y: 2, z: 3 };
            const pending = new HSMLElement(-123);
            pending.position = vector;
            pending.rotation = vector;
            pending.scale = vector;
            pending.globalPosition = vector;
            vector.x = 99; vector.y = 98; vector.z = 97;
            if (pending.position.x !== 1 || pending.rotation.y !== 2 || pending.scale.z !== 3)
                throw new Error('pending transform aliased caller memory');
            pending._resolveNodeId(123);
            const resolved = new HSMLElement(456);
            resolved.position = {x: 8, y: 9, z: 10};
            resolved.rotation = {x: 0, y: 0.7, z: 0};
            if (resolved.position.x !== 8 || resolved.rotation.y !== 0.7)
                throw new Error('immediate pose read returned stale snapshot');
        "#)?;
        let positions = eng.drain_transform_position_updates();
        assert_eq!(positions[0].0, 123);
        assert_eq!(positions[0].1.x, 1.0);
        assert_eq!(positions[1].1.x, 8.0);
        assert_eq!(eng.drain_transform_rotation_updates()[0].1.y, 2.0);
        assert_eq!(eng.drain_transform_scale_updates()[0].1.z, 3.0);
        Ok(())
    }

    use deno_core::{v8, FastString, JsRuntime, RuntimeOptions};
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_run_js_sum() -> Result<()> {
        let script = r#"
            function sumar(a, b) {
                return a + b;
            }
            sumar(3, 4);
        "#;
        let result = run_js(script)?;
        assert_eq!(result, Value::from(7));
        Ok(())
    }

    #[test]
    fn test_run_engine() -> Result<()> {
        let mut eng = Engine::new();
        eng.eval(r#"
            console.log("hola desde JS");
            setTimeout(()=>console.log("timeout!"), 5);
            //requestAnimationFrame(ts => console.log("raf", Math.round(ts)));
        "#)?;

        Ok(for i in 0..5 {
            std::thread::sleep(Duration::from_millis(33));
            eng.fire_raf((i as f64)*16.67);
        })
    }

    #[test]
    fn set_interval_repeats_until_cleared() -> Result<()> {
        let mut eng = Engine::new();
        eng.eval(r#"
            globalThis.n = 0;
            globalThis.parada = 0;
            globalThis.id = setInterval(() => {
                n++;
                if (n === 3) { clearInterval(id); parada = n; }
            }, 5);
        "#)?;
        for i in 0..12 {
            std::thread::sleep(Duration::from_millis(15));
            eng.fire_raf((i as f64) * 16.67);
        }
        // Dispara al menos tres veces y ninguna despues de clearInterval. Antes
        // de este cambio setInterval no existia y el eval de arriba fallaba.
        eng.eval(r#"
            if (parada !== 3) throw new Error("no llego a 3: n=" + n);
            if (n !== 3) throw new Error("siguio despues de clearInterval: n=" + n);
        "#)?;
        Ok(())
    }

    #[test]
    fn test_run_js_string() -> Result<()> {
        let script = r#"
            "hello, world!";
        "#;
        let result = run_js(script)?;
        assert_eq!(result, Value::from("hello, world!"));
        Ok(())
    }

    
    #[test]
    fn test_global_dimention_alias_exists() -> Result<()> {
        let mut eng = Engine::new();
        eng.eval(r#"
            console.log(typeof dimention, dimention === hiperspace.dimention, dimention.nodeId);
        "#)?;

        let logs = eng.drain_logs();
        assert!(logs.iter().any(|(_, msg)| msg.contains("object true 0")));
        Ok(())
    }

    #[test]
    fn test_dom_toque_dispatch_works_without_controller_script() -> Result<()> {
        use std::collections::HashMap;
        let mut eng = Engine::new();

        let mut attrs = HashMap::new();
        attrs.insert(0, HashMap::new());
        attrs.insert(1, {
            let mut m = HashMap::new();
            m.insert("id".to_string(), "btn".to_string());
            m
        });
        eng.update_attr_snapshot(attrs);

        let mut tags = HashMap::new();
        tags.insert(0, "hsml".to_string());
        tags.insert(1, "box".to_string());
        eng.update_tag_snapshot(tags);

        let mut parents = HashMap::new();
        parents.insert(0, -1);
        parents.insert(1, 0);

        let mut children = HashMap::new();
        children.insert(0, vec![1]);
        children.insert(1, vec![]);
        eng.update_hierarchy_snapshot(parents, children);

        eng.update_transform_snapshot(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        );

        eng.eval(r#"
            const btn = hiperspace.dimention.getElementById('btn');
            if (!btn) {
              console.log('BTN_MISSING');
            } else {
              btn.addEventListener('toque', () => console.log('TOQUE_OK'));
            }
        "#)?;

        eng.push_dom_toque_event(1, 1.0, 2.0, 3.0);
        eng.fire_raf(0.0);

        let logs = eng.drain_logs();
        assert!(logs.iter().any(|(_, msg)| msg.contains("TOQUE_OK")));
        assert!(!logs.iter().any(|(_, msg)| msg.contains("BTN_MISSING")));
        Ok(())
    }
    
    #[test]
    fn test_increment_performance_rust() -> std::io::Result<()> {
        let iterations = 1_000_000_000;
        let start = Instant::now();

        // Incrementamos la variable en un bucle
        let mut v = 0;
        for _ in 0..iterations {
            v += 1;
        }

        let duration = start.elapsed();
        println!("El incremento nativo se ejecutó en: {:?}", duration);

        // Escribimos los resultados en un archivo.
        let mut file = File::create("tests/logs/test_increment_performance_rust.txt")?;
        writeln!(
            file,
            "Native increment: {} iteraciones en {:?}",
            iterations, duration
        )?;

        // Se espera que v sea igual a 1,000,000
        assert_eq!(v, iterations);
        Ok(())
    }
    
    #[test]
    fn test_increment_performance_v8() -> Result<()> {
        // Script que incrementa una variable 1,000,000 de veces
        let script = r#"
            let v = 0;
            for (let i = 0; i < 1000000; i++) {
                v += 1;
            }
            v;
        "#;
        let start = Instant::now();
        let result = run_js(script)?;
        let duration = start.elapsed();
        println!("El script se ejecutó en: {:?}", duration);

        // Escribimos los resultados en un archivo.
        let mut file = File::create("tests/logs/test_increment_performance_v8.txt")?;
        writeln!(
            file,
            "Native JS increment: {} iterations in {:?}",
            1000000, duration
        )?;

        // Se espera que v sea igual a 1,000,000
        assert_eq!(result, Value::from(1000000));
        Ok(())
    }

    /// Test que accede de forma nativa a la variable global "v" en el runtime de V8,
    /// la incrementa en 1 en cada iteración y mide el tiempo total para 1,000,000 de iteraciones.
    /// El resultado se escribe en un archivo "performance.txt".
    #[test]
    fn test_native_increment_performance() -> Result<()> {
        // Creamos el runtime con opciones por defecto.
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
        
        // Inicializamos la variable global "v" en el contexto JavaScript.
        {
            runtime.execute_script("<init>", FastString::Static("v = 0;"))?;
        }
        
        let iterations = 1_000_000;
        let start = Instant::now();

        {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global = context.global(scope);
            // Creamos la llave "v" fuera del bucle para evitar recrearla en cada iteración.
            let key = v8::String::new(scope, "v").unwrap().into();

            for _ in 0..iterations {
                // Obtenemos el valor actual de "v".
                let value = global.get(scope, key).unwrap();
                // Convertimos el valor a número.
                let num = value.number_value(scope).unwrap();
                let new_num = num + 1.0;
                // Creamos un nuevo número de V8 con el resultado.
                let new_value = v8::Number::new(scope, new_num).into();
                // Actualizamos la propiedad "v" en el objeto global.
                global.set(scope, key, new_value).unwrap();
            }
        }
        let duration = start.elapsed();

        // Obtenemos el valor final de "v" para verificar que sea igual a iterations.
        let final_value = {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global = context.global(scope);
            let key = v8::String::new(scope, "v").unwrap().into();
            let value = global.get(scope, key).unwrap();
            value.number_value(scope).unwrap()
        };

        // Escribimos los resultados en un archivo.
        let mut file = File::create("tests/logs/test_native_increment_performance.txt")?;
        writeln!(
            file,
            "Native increment: {} iterations in {:?}",
            iterations, duration
        )?;

        // Verificamos que el valor final sea correcto.
        assert_eq!(final_value, iterations as f64);
        Ok(())
    }

    #[test]
    fn test_native_increment_performance_ref() -> Result<()> {
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
    
        // Definir la variable `v` en JavaScript con valor 0
        runtime.execute_script("<init>", FastString::Static("var v = 0;"))?;
    
        let iterations = 1_000_000;
        let start = Instant::now();
    
        {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global_obj = context.global(scope);
    
            // Tomamos el valor actual de "v"
            let key = v8::String::new(scope, "v").unwrap().into();
            let val = global_obj.get(scope, key).unwrap();
            let num = val.number_value(scope).unwrap();
    
            // Creamos un Local<v8::Number> con ese valor
            let local_number = v8::Number::new(scope, num);
    
            // Creamos un Global<v8::Number> para no tener que buscar "v" otra vez
            let mut number_global = v8::Global::new(scope, local_number);
    
            // Bucle de incrementos
            for _ in 0..iterations {
                // Creamos un Local<v8::Number> a partir del Global actual
                let local_val = v8::Local::new(scope, &number_global);
                let current = local_val.number_value(scope).unwrap();
                let new_local = v8::Number::new(scope, current + 1.0);
    
                // Reemplazamos el Global con el nuevo valor
                number_global = v8::Global::new(scope, new_local);
            }
    
            // Guardamos el valor final en la propiedad `v` del objeto global
            let final_local = v8::Local::new(scope, &number_global);
            let final_val = final_local.number_value(scope).unwrap();
            let final_js_val = v8::Number::new(scope, final_val).into();
            global_obj.set(scope, key, final_js_val).unwrap();
        }
    
        let duration = start.elapsed();
    
        // Verificamos que el valor final sea igual a la cantidad de iteraciones
        let final_value = {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global_obj = context.global(scope);
            let key = v8::String::new(scope, "v").unwrap().into();
            let val = global_obj.get(scope, key).unwrap();
            val.number_value(scope).unwrap()
        };
    
        let mut file = File::create("tests/logs/test_native_increment_performance_ref.txt")?;
        writeln!(
            file,
            "Ref-based increment: {} iterations in {:?}",
            iterations, duration
        )?;
    
        assert_eq!(final_value, iterations as f64);
        Ok(())
    }

   
    #[test]
    fn test_native_increment_performance_array() -> Result<()> {
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
    
        // Definimos un Int32Array de 1 elemento; buffer[0] = 0
        runtime.execute_script(
            "<init>",
            FastString::Static("var buffer = new Int32Array(1); buffer[0] = 0;"),
        )?;
    
        let iterations = 1_000_000;
        let start = Instant::now();
    
        {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global_obj = context.global(scope);
    
            // Obtenemos una referencia (Global<v8::Object>) al array `buffer`
            let key = v8::String::new(scope, "buffer").unwrap().into();
            let buffer_val = global_obj.get(scope, key).unwrap();
            let buffer_obj = buffer_val.to_object(scope).unwrap();
            let mut buffer_global = v8::Global::new(scope, buffer_obj);
    
            // Increméntalo un millón de veces
            for _ in 0..iterations {
                // Local al Int32Array
                let local_buffer = v8::Local::new(scope, &buffer_global);
                let local_obj = local_buffer.to_object(scope).unwrap();
    
                // buffer[0]
                let current_val = local_obj.get_index(scope, 0).unwrap();
                let current_i32 = current_val.int32_value(scope).unwrap();
    
                // sumamos 1
                let new_i32 = v8::Integer::new(scope, current_i32 + 1);
    
                // set_index requiere un Local<Value>, así que usamos `.into()`
                local_obj.set_index(scope, 0, new_i32.into()).unwrap();
            }
        }
    
        let duration = start.elapsed();
    
        // Leemos buffer[0] de nuevo para comprobar el resultado
        let final_value = {
            let scope = &mut runtime.handle_scope();
            let context = scope.get_current_context();
            let global_obj = context.global(scope);
    
            let key = v8::String::new(scope, "buffer").unwrap().into();
            let buffer_val = global_obj.get(scope, key).unwrap();
            let buffer_obj = buffer_val.to_object(scope).unwrap();
            let val0 = buffer_obj.get_index(scope, 0).unwrap();
            val0.int32_value(scope).unwrap()
        };
    
        let mut file = File::create("tests/logs/test_native_increment_performance_array.txt")?;
        writeln!(
            file,
            "TypedArray increment: {} iterations in {:?}",
            iterations, duration
        )?;
    
        assert_eq!(final_value, iterations as i32);
        Ok(())
    }
}
