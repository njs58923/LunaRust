use specs::{prelude::*, World, WorldExt};
use specs_derive::Component;
use std::{collections::HashMap, default};

use super::{
    create_new_id,
    hsml::{Include, Model, Script},
};

#[derive(Component, Debug, Default)]
pub struct Hierarchy {
    /// Padre del nodo. Se almacena como `Entity` completo (id + generation)
    /// para evitar bugs por reciclaje de slots: si guardáramos sólo el `u32`
    /// de id, después de un despawn + spawn el id podría apuntar a otro nodo
    /// totalmente distinto (mismo slot, gen distinta). El `Entity` retiene
    /// la gen y `is_alive(ent)` detecta el reciclaje.
    pub parent: Option<Entity>,
    pub children: Vec<Entity>,
}

impl Hierarchy {
    pub fn default() -> Hierarchy {
        Hierarchy {
            parent: None,
            children: vec![],
        }
    }
    pub fn add_child(world: &mut World, parent: Entity, child: Entity) {
        let _ = Self::try_add_child(world, parent, child);
    }

    /// Returns whether the tree changed. Validate before detaching anything.
    pub fn try_add_child(
        world: &mut World,
        parent: Entity,
        child: Entity,
    ) -> Result<bool, &'static str> {
        if parent == child {
            return Err("a node cannot parent itself");
        }
        if !world.entities().is_alive(parent) || !world.entities().is_alive(child) {
            return Err("cannot attach a dead node");
        }
        let old_parent = {
            let hier = world.read_storage::<Hierarchy>();
            if hier.get(parent).is_none() {
                return Err("parent has no hierarchy");
            }
            let old = hier.get(child).ok_or("child has no hierarchy")?.parent;
            if old == Some(parent) {
                return Ok(false);
            }
            // Walk ancestors, not siblings. Floyd's second cursor also protects
            // against a pre-existing corrupt cycle, without allocating a set.
            let mut cursor = Some(parent);
            let mut fast = Some(parent);
            while let Some(node) = cursor {
                if node == child {
                    return Err("attachment would create a cycle");
                }
                cursor = hier.get(node).and_then(|h| h.parent);
                fast = fast
                    .and_then(|n| hier.get(n).and_then(|h| h.parent))
                    .and_then(|n| hier.get(n).and_then(|h| h.parent));
                if cursor.is_some() && cursor == fast {
                    return Err("parent hierarchy contains a cycle");
                }
            }
            old
        };
        let mut hier = world.write_storage::<Hierarchy>();
        if let Some(old) = old_parent {
            if let Some(h) = hier.get_mut(old) {
                h.children.retain(|n| *n != child);
            }
        }
        // The child's authoritative parent proves it is not already in this
        // sibling list. Re-scanning that list makes wide tree construction O(N²).
        hier.get_mut(parent).unwrap().children.push(child);
        hier.get_mut(child).unwrap().parent = Some(parent);
        Ok(true)
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

#[derive(Component, Debug, Clone)]
pub struct BaseUrl(pub String);

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
    world.register::<BaseUrl>();
    world.register::<Text>();
    world.register::<Model>();
    world.register::<Script>();
    world.register::<Include>();

    world
}

struct ElementTest {
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
        let entity = world.create_entity().with(Id(create_new_id())).build();
        Self { entity }
    }

    pub fn with_tag(world: &mut World, tag: &str) -> Self {
        let entity = world
            .create_entity()
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

#[cfg(test)]
mod hierarchy_tests {
    use super::*;
    use specs::{Builder, WorldExt};
    #[test]
    fn reparenting_is_idempotent_and_rejects_cycles_without_mutation() {
        let mut world = World::new();
        world.register::<Hierarchy>();
        let a = world.create_entity().with(Hierarchy::default()).build();
        let b = world.create_entity().with(Hierarchy::default()).build();
        let c = world.create_entity().with(Hierarchy::default()).build();
        assert_eq!(Hierarchy::try_add_child(&mut world, a, b), Ok(true));
        assert_eq!(Hierarchy::try_add_child(&mut world, b, c), Ok(true));
        assert!(Hierarchy::try_add_child(&mut world, c, a).is_err());
        assert!(Hierarchy::try_add_child(&mut world, a, a).is_err());
        assert_eq!(Hierarchy::try_add_child(&mut world, a, b), Ok(false));
        assert_eq!(Hierarchy::get_children(&world, a).unwrap(), vec![b]);
        assert_eq!(Hierarchy::try_add_child(&mut world, a, c), Ok(true));
        assert!(Hierarchy::get_children(&world, b).unwrap().is_empty());
        assert_eq!(Hierarchy::get_children(&world, a).unwrap(), vec![b, c]);
    }
    #[test]
    fn corrupt_parent_chain_and_dead_nodes_are_rejected() {
        let mut world = World::new();
        world.register::<Hierarchy>();
        let a = world.create_entity().with(Hierarchy::default()).build();
        let b = world.create_entity().with(Hierarchy::default()).build();
        let child = world.create_entity().with(Hierarchy::default()).build();
        {
            let mut hier = world.write_storage::<Hierarchy>();
            hier.get_mut(a).unwrap().parent = Some(b);
            hier.get_mut(b).unwrap().parent = Some(a);
        }
        assert!(Hierarchy::try_add_child(&mut world, a, child).is_err());
        assert_eq!(
            world.read_storage::<Hierarchy>().get(child).unwrap().parent,
            None
        );
        world.delete_entity(b).unwrap();
        assert!(Hierarchy::try_add_child(&mut world, b, child).is_err());
    }

    #[test]
    fn wide_tree_preserves_all_siblings_without_duplicates() {
        let mut world = World::new();
        world.register::<Hierarchy>();
        let root = world.create_entity().with(Hierarchy::default()).build();
        let children: Vec<_> = (0..10000)
            .map(|_| world.create_entity().with(Hierarchy::default()).build())
            .collect();
        for &child in &children {
            assert_eq!(Hierarchy::try_add_child(&mut world, root, child), Ok(true));
        }
        for &child in &children {
            assert_eq!(Hierarchy::try_add_child(&mut world, root, child), Ok(false));
        }
        assert_eq!(Hierarchy::get_children(&world, root).unwrap(), children);
    }
}
