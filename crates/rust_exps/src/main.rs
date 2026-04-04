use core::num;
use std::collections::HashMap;
use virtual_dom;

fn main() {
    // hash_map_test()
    TestDom();
}

fn hash_map_test() {
     // Crear un HashMap que mapea nombres a enteros.
     let mut map: HashMap<&str, i32> = HashMap::new();

     // Insertar algunos valores.
     map.insert("uno", 1);
     map.insert("dos", 2);
     map.insert("tres", 3);
 
     // Buscar un valor de forma directa: O(1) en promedio.
     let key = "dos";
     match map.get(key) {
         Some(&value) => println!("La llave '{}' tiene el valor {}", key, value),
         None => println!("La llave '{}' no existe", key),
     }
 
     // Manejar el caso cuando la llave no existe usando el API `entry`.
     // Si "cuatro" no existe, se inserta con el valor 4.
     let value = map.entry("cuatro").or_insert(4);
     println!("El valor para 'cuatro' es: {}", value);
 
     // Iterar sobre el HashMap de forma óptima:
     // La iteración recorre cada par (clave, valor) sin necesidad de buscar cada elemento.
     for (key, value) in &map {
         println!("{}: {}", key, value);
     }
 
     // Actualizar un valor de forma mutable solo si existe.
     if let Some(value) = map.get_mut("tres") {
         *value += 10;
     }
     println!("HashMap actualizado: {:?}", map);
 
 
     let mut midick: HashMap<&str, &str> = HashMap::new();
 
     midick.insert("a", "aa");
     midick.insert("b", "bb");
 
     println!("TEST {:?}", midick.capacity());
 
     
     // Obtenemos una referencia mutable al valor asociado a la clave "a".
     if let Some(value2) = midick.get_mut("b") {
         // Actualizamos el valor usando el operador de desreferencia.
         *value2 = "AA";
     } else {
         println!("La clave 'a' no existe en el HashMap");
     }
 
     println!("HashMap actualizado: {:?}", midick);
}


fn TestDom(){
    
}

// /// A 3-dimensional vector.
// #[derive(Debug, Clone, Copy, PartialEq)]
// pub struct Vec3 {
//     pub x: f32,
//     pub y: f32,
//     pub z: f32,
// }

// #[derive(Debug, Clone, PartialEq)]
// struct  Transform{
//     position: Vec3,
//     rotation: Vec3,
//     scale: Vec3
// }
// #[derive(Debug, Clone)]
// enum  ElementExtends {
//     Model(ModelElement)
// }
// #[derive(Debug, Clone)]
// struct Element{
//     _id: usize,
//     tag: Option<String>,
//     parent: usize,
//     transforms: Option<Transform>,
//     extends: Vec<ElementExtends>,
//     children: Vec<Element>,
// }

// #[derive(Debug, Clone)]
// struct ModelElement{
//     _self: Element,
//     src: String
// }


/// A 3-dimensional vector.
// #[derive(Debug, Clone, Copy, PartialEq)]
// pub struct Vec3 {
//     pub x: f32,
//     pub y: f32,
//     pub z: f32,
// }

// #[derive(Debug, Clone, PartialEq)]
// struct  Transform{
//     position: Vec3,
//     rotation: Vec3,
//     scale: Vec3
// }

// enum  ElementTypes {
//     Node(Node),
//     Element(Element),
//     HSMLElement(HSMLElement)
// }

// #[derive(Debug, Clone)]
// struct Node{
//     _id: usize,
//     parent: usize,
//     children: Vec<Node>,
// }

// struct Element{
//     tag: Option<String>,
//     transforms: Option<Transform>,
// }

// struct HSMLElement{
//     tag: Option<String>,
//     transforms: Option<Transform>,
// }

// #[derive(Debug, Clone)]
// struct ModelElement{
//     src: String
// }

use specs::prelude::*;
use specs_derive::Component;

// --- COMPONENTES PUROS ---

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

#[derive(Component, Debug, Clone)]
pub struct Tag(pub String);

#[derive(Component, Debug, Clone)]
pub struct Hierarchy {
    pub parent: Option<Entity>,
    pub children: Vec<Entity>,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id(pub usize);

// --- COMPONENTE PARA DIFERENCIAR TIPOS DE ENTIDADES (opcional) ---
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    Node,
    Element,
    HSMLElement,
}

// --- FUNCIONES DE AYUDA (impl externos asociados) ---

impl Hierarchy {
    pub fn add_child(world: &mut World, parent: Entity, child: Entity) {
        let mut hierarchies = world.write_storage::<Hierarchy>();

        // Añadir hijo al padre
        hierarchies.entry(parent).unwrap().or_insert_with(|| Hierarchy {
            parent: None,
            children: vec![],
        }).children.push(child);

        // Añadir padre al hijo
        hierarchies.entry(child).unwrap().or_insert_with(|| Hierarchy {
            parent: Some(parent),
            children: vec![],
        }).parent = Some(parent);
    }

    // Obtener hijos fácilmente
    pub fn get_children(world: &World, entity: Entity) -> Option<Vec<Entity>> {
        let hierarchies = world.read_storage::<Hierarchy>();
        hierarchies.get(entity).map(|h| h.children.clone())
    }
}

// --- EJEMPLO PRÁCTICO COMPLETO ---
fn main2() {
    let mut world = World::new();

    // Registrar componentes
    world.register::<Id>();
    world.register::<Tag>();
    world.register::<Transform>();
    world.register::<Hierarchy>();

    // Crear entidades
    let nodo_padre = world.create_entity()
        .with(Id(1))
        .with(Hierarchy { parent: None, children: vec![] })
        .build();

    let hijo_element = world
        .create_entity()
        .with(Id(2))
        .with(Tag("div".into()))
        .with(Transform {
            position: Vec3 { x: 0.0, y: 1.0, z: 2.0 },
            rotation: Vec3 { x: 0.0, y: 45.0, z: 0.0 },
            scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        })
        .build();

    // Añadir la relación padre-hijo claramente usando métodos externos
    Hierarchy::add_child(&mut world, nodo_padre, hijo_element);

    // --- EJEMPLO DE ESCRITURA (modificar componentes) ---
    {
        let mut transforms = world.write_storage::<Transform>();
        
        if let Some(transform) = transforms.get_mut(hijo_element) {
            transform.position.x += 5.0;
        }
    }

    // --- EJEMPLO DE LECTURA (leer componentes) ---
    {
        let transforms = world.read_storage::<Transform>();
        let tags = world.read_storage::<Tag>();
        let ids = world.read_storage::<Id>();

        for (id, tag, transform) in (&ids, &tags, &transforms).join() {
            println!("Entidad {} ({:?}): Posición {:?}", id.0, tag, transform.position);
        }
    }

    // --- EJEMPLO DE NAVEGACIÓN ENTRE PADRES E HIJOS ---
    {
        let hierarchies = world.read_storage::<Hierarchy>();
        let ids = world.read_storage::<Id>();

        // Obtener hijos del padre
        if let Some(children) = hierarchies.get(nodo_padre).map(|h| &h.children) {
            println!("Hijos del padre ({}):", nodo_padre.id());
            for child in children {
                let id_storage = world.read_storage::<Id>();
                if let Some(id) = id_storage.get(*child) {
                    println!("  - Hijo con ID {:?}", id.0);
                }
            }
        }
    }
}
