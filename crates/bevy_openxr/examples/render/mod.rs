use bevy::prelude::*;
use specs::{Entity as SpecEntity, ReadStorage, World as SpecWorld, WorldExt};

use virtual_dom::{
    dom::{element::{  Transform2}, hsml::Model}
};

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

pub fn  apply_transform(node:&Transform2, transform: &mut Transform) {
    transform.translation = Vec3::new(node.position.x, node.position.y, node.position.z);
    transform.rotation = Quat::from_euler(EulerRot::XYZ, node.rotation.x, node.rotation.y, node.rotation.z);
    transform.scale = Vec3::new(node.scale.x, node.scale.y, node.scale.z);
}

pub fn apply_model(node:&Model,  asset_server: &Res<AssetServer>, commands:&mut Commands, entity: &mut Entity) {
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