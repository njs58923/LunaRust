use specs::{World, WorldExt, prelude::*};
use specs_derive::Component;
use std::{collections::HashMap, default};

use super::{create_new_id, hsml::{Model, Script, Include}};


#[derive(Component, Debug, Default)]
pub struct Hierarchy {
    pub parent: Option<u32>,
    pub children: Vec<Entity>,
}

impl Hierarchy {
    pub fn default()-> Hierarchy{
        Hierarchy { parent: None, children: vec![] }
    }
    pub fn add_child(world: &mut World, parent: Entity, child: Entity) {
        let mut hierarchies = world.write_storage::<Hierarchy>();

        // Añadir hijo al padre
        hierarchies.get_mut(parent).unwrap().children.push(child);

        // Añadir padre al hijo
        hierarchies.get_mut(child).unwrap().parent = Some(parent.id());

    }

    pub fn get_children(world: &World, entity: Entity) -> Option<Vec<Entity>> {
        world
            .read_storage::<Hierarchy>()
            .get(entity)
            .map(|h| h.children.clone())
    }
}

#[derive(Component, Debug, Clone)]
pub struct Id(pub u32);

#[derive(Component, Debug, Clone)]
pub struct Tag(pub String);

#[derive(Component, Debug, Clone)]
pub struct Text(pub String);

#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Component, Debug, Clone, PartialEq)]
pub struct Transform2 {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

#[derive(Component, Debug, Clone)]
pub struct Attrs(pub HashMap<String, String>);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementType {
    Node,
    Element,
    HSMLElement,
}

pub fn build_world() -> World {
    let mut world = World::new();

    world.register::<Id>();
    world.register::<Hierarchy>();
    world.register::<Tag>();
    world.register::<Transform2>();
    world.register::<Attrs>();
    world.register::<Text>();
    world.register::<Model>();
    world.register::<Script>();
    world.register::<Include>();

    world
}

struct ElementTest{
    hierarchy: Option<Hierarchy>,
    tag: Option<Tag>,
    transform2: Option<Transform2>,
    attrs: Option<Attrs>,
    text: Option<Text>,
    model: Option<Model>,
    script: Option<Script>,
    include: Option<Include>,
}

pub struct Element {
    pub entity: Entity,
}

impl Element {
    pub fn new(world: &mut World) -> Self {
        let entity = world.create_entity()
        .with(Id(create_new_id()))
        .build();
        Self { entity }
    }

    pub fn with_tag(world: &mut World, tag: &str) -> Self {
        let entity = world.create_entity()
            .with(Id(create_new_id()))
            .with(Tag(tag.to_string()))
            .build();
        Self { entity }
    }

    pub fn add_child(&self, world: &mut World, child: &Element) {
        Hierarchy::add_child(world, self.entity, child.entity);
    }

    pub fn get_children(&self, world: &World) -> Option<Vec<Entity>> {
        Hierarchy::get_children(world, self.entity)
    }
}

fn main() {
    let mut world = build_world();

    let parent = Element::with_tag(&mut world, "body");
    let child = Element::with_tag(&mut world, "div");

    parent.add_child(&mut world, &child);

    // lectura:
    if let Some(children) = parent.get_children(&world) {
        let tags = world.read_storage::<Tag>();
        for child_entity in children {
            if let Some(tag) = tags.get(child_entity) {
                println!("Child Tag: {}", tag.0);
            }
        }
    }
}
