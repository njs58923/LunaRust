// src/bin/demo.rs
use js_runtime::Engine;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut eng = Engine::new();
    println!("AAAAAAAAAAA");
    eng.eval(r#"
        console.log("hola desde JS");
        console.log("hola desde JS");
        console.log("hola desde JS");
        console.log("hola desde JS");
        setTimeout(()=>console.log("timeout!"), 50);
        requestAnimationFrame(ts => console.log("raf", Math.round(ts)));
    "#)?;

    for i in 0..5 {
        std::thread::sleep(Duration::from_millis(33));
        eng.fire_raf((i as f64)*16.67);
    }

    Ok(())
}
