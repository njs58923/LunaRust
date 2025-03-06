use bevy::prelude::*;
use virtual_dom::dom::{element::Element, hsml::hsml::HSMLElement, hsml::hsml::MODELElement};


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

pub fn  apply_hsml_element<N>(node:&HSMLElement<N>, transform: &mut Transform) {
    transform.translation = Vec3::new(node.x, node.y, node.z);
    transform.rotation = Quat::from_euler(EulerRot::XYZ, node.rx, node.ry, node.rz);
    transform.scale = Vec3::new(node.sx, node.sy, node.sz);
}

pub fn apply_model_element<N>(node:&MODELElement<N>,  asset_server: &Res<AssetServer>, commands:&mut Commands, entity: &mut Entity) {
    let model_handle: Handle<Scene> = asset_server.load(GltfAssetLabel::Scene(0).from_asset(node.src.clone().unwrap()));

    let child_entity = commands
    .spawn(SceneBundle {
        scene: model_handle,
        transform: Transform::default(),
        ..default()
    }, )
    .id();

    commands.entity(*entity).push_children(&[child_entity]);    
}