use bevy::prelude::*;
use virtual_dom::dom::{element::Element, hsml::hsml::HSMLElement};


// #[derive(Debug, Clone)]
// enum  ElementExtends {
//     Model(ModelElement)
// }

// #[derive(Debug, Clone)]
// struct RenderElement{
//     _id: usize,
//     native: Entity,
//     extends: Vec<ElementExtends>,
// }

// #[derive(Debug, Clone)]
// struct ModelElement{
//     _self: Element,
//     src: String
// } 

pub fn  ApplyElement<N>(node:Element<N>, mut transform: Transform) {
}

pub fn  ApplyHSMLElement<N>(node:HSMLElement<N>, mut transform: Transform) {
    transform.translation = Vec3::new(node.x, node.y, node.z);
    transform.rotation = Quat::from_euler(EulerRot::XYZ, node.rx, node.ry, node.rz);
}