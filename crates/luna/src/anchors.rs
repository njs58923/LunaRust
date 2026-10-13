//! Named pose sources. DOM ownership stays intact; only render placement follows.
use crate::permissions::{CapabilityBits, NativeServiceBits, SpacePolicies};
use crate::{ElemenetWorld, EntityMap, SpaceHandleTables};
use bevy::{prelude::*, render::view::VisibilitySystems, transform::TransformSystem};
use js_runtime::anchors::{Reply, Request, Source, MAX_ANCHORS};
use specs::{Join, WorldExt};
use std::collections::{HashMap, HashSet};
use virtual_dom::dom::element::Attrs;

#[derive(Resource, Default)]
pub struct NativeReferences {
    names: HashMap<(specs::Entity, String), u64>,
    slots: HashMap<u64, Option<Entity>>,
    requirements: HashMap<u64, (CapabilityBits, NativeServiceBits)>,
    next: u64,
}
impl NativeReferences {
    /// The native provider registers aliases in its owning space, not DOM IDs.
    pub fn register(&mut self, owner: specs::Entity, name: &str, entity: Option<Entity>) -> u64 {
        let id = *self.names.entry((owner, name.into())).or_insert_with(|| {
            self.next += 1;
            self.next
        });
        self.slots.insert(id, entity);
        id
    }
    pub fn restrict(&mut self, id: u64, use_caps: CapabilityBits, provider: NativeServiceBits) {
        self.requirements.insert(id, (use_caps, provider));
    }
    pub fn set_target(&mut self, id: u64, target: Option<Entity>) {
        if let Some(slot) = self.slots.get_mut(&id) {
            *slot = target;
        }
    }
}

enum PoseSource {
    Native(u64),
    Element(specs::Entity),
    Program(Box<ProgramSource>),
}
struct ProgramSource {
    program: std::sync::Arc<crate::motion_program::Program>,
    entity: Entity,
    last: [Option<(u64, u32, GlobalTransform)>; 16],
    bindings: [Option<u64>; 16],
    generation: u64,
}
struct Publication {
    owner: specs::Entity,
    request_id: u32,
    key: String,
    source: PoseSource,
}
#[derive(Resource, Default)]
pub struct Anchors {
    entries: HashMap<u64, Publication>,
    keys: HashMap<String, u64>,
    next: u64,
    generation: u64,
}
#[derive(Component)]
struct Follower {
    key: String,
    owner: specs::Entity,
    source: Option<u64>,
}
#[derive(Component)]
struct AnchorAvailability(bool);
#[derive(Component)]
pub(crate) struct NativeControllerInput(pub js_runtime::anchors::ControllerInput);
#[derive(Resource, Default)]
struct AnchorFrame(u64);
#[derive(Resource, Default)]
struct BindingVersion(Option<u64>);
#[derive(Resource, Default)]
#[doc(hidden)]
pub struct AnchorDirty(pub HashSet<u32>);

pub struct AnchorPlugin;
impl Plugin for AnchorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Anchors>()
            .init_resource::<NativeReferences>()
            .init_resource::<BindingVersion>()
            .init_resource::<AnchorDirty>()
            .init_resource::<AnchorFrame>()
            .add_systems(
                PostUpdate,
                sync_bindings.before(TransformSystem::TransformPropagate),
            )
            .add_systems(
                PostUpdate,
                follow
                    .after(TransformSystem::TransformPropagate)
                    .after(VisibilitySystems::VisibilityPropagate)
                    .before(VisibilitySystems::UpdateFrusta)
                    .before(VisibilitySystems::CheckVisibility),
            )
            .add_systems(PostUpdate, capture_listeners.after(follow));
    }
}

pub fn apply_requests(world: &mut World, space_id: u32, requests: Vec<Request>) -> Vec<Reply> {
    world.init_resource::<Anchors>();
    world.init_resource::<NativeReferences>();
    requests
        .into_iter()
        .map(|request| {
            let error = register_request(world, space_id, &request).err();
            Reply {
                id: request.id,
                error,
            }
        })
        .collect()
}
fn register_request(world: &mut World, space_id: u32, request: &Request) -> Result<(), String> {
    let policy = world
        .get_resource::<SpacePolicies>()
        .and_then(|p| p.by_space.get(&space_id))
        .ok_or("Anchor publisher has no policy")?;
    if !policy
        .effective_caps
        .contains(CapabilityBits::CREATE_ANCHOR)
    {
        return Err("Space lacks create_anchor permission".into());
    }
    if !js_runtime::anchors::valid_key(&request.key) {
        return Err("Invalid anchor key".into());
    }
    let native_services = policy.effective_native;
    let specs = world
        .get_resource::<ElemenetWorld>()
        .ok_or("Missing document")?;
    let owner = specs.0.entities().entity(space_id);
    if !specs.0.entities().is_alive(owner)
        || !specs
            .0
            .read_storage::<virtual_dom::dom::element::Tag>()
            .get(owner)
            .is_some_and(|tag| tag.0 == "space")
    {
        return Err("Anchor publisher is no longer alive".into());
    }
    let mut source = match &request.source {
        None => None,
        Some(Source::Element(local)) => {
            let global = world
                .get_resource::<SpaceHandleTables>()
                .and_then(|t| t.by_space.get(&space_id))
                .and_then(|t| t.local_to_global.get(local))
                .copied()
                .ok_or("Invalid local anchor element")?;
            let node = specs.0.entities().entity(global);
            let owned = match crate::js::find_owner_space_id(&specs.0, node) {
                Some(owner) => owner == space_id,
                None => world
                    .resource::<SpaceHandleTables>()
                    .by_space
                    .get(&space_id)
                    .is_some_and(|table| table.detached_globals.contains(&global)),
            };
            if !specs.0.entities().is_alive(node) || !owned {
                return Err("Anchor source must belong to its publisher".into());
            }
            Some(PoseSource::Element(node))
        }
        Some(Source::Native(name)) => {
            let id = world
                .resource::<NativeReferences>()
                .names
                .get(&(owner, name.clone()))
                .copied()
                .ok_or("Unknown native reference in this space")?;
            let required = world
                .resource::<NativeReferences>()
                .requirements
                .get(&id)
                .map(|r| r.1)
                .unwrap_or_default();
            if !native_services.contains(required) {
                return Err("Native provider service is no longer authorized".into());
            }
            Some(PoseSource::Native(id))
        }
        Some(Source::Program(value)) => {
            let definition: crate::motion_program::Definition =
                serde_json::from_value(value.clone())
                    .map_err(|e| format!("Program definition: {e}"))?;
            let program = crate::motion_program::Program::compile(&definition)?;
            if program.inputs.iter().any(|i| i.key() == request.key) {
                return Err("Program cannot read its own output".into());
            }
            Some(PoseSource::Program(Box::new(ProgramSource {
                program: std::sync::Arc::new(program),
                entity: Entity::PLACEHOLDER,
                last: [None; 16],
                bindings: [None; 16],
                generation: u64::MAX,
            })))
        }
    };
    let anchors = world.resource::<Anchors>();
    let old = anchors.keys.get(&request.key).copied();
    if old.is_some_and(|id| anchors.entries[&id].owner != owner) {
        return Err("Anchor key is already published by another space".into());
    }
    if request.source.is_none()
        && request
            .guard
            .is_some_and(|guard| old.is_some_and(|id| anchors.entries[&id].request_id != guard))
    {
        return Err("Anchor handle belongs to an older publication".into());
    }
    if source.is_some()
        && old.is_none()
        && anchors
            .entries
            .values()
            .filter(|p| p.owner == owner)
            .count()
            >= MAX_ANCHORS
    {
        return Err("Anchor publication limit reached".into());
    }
    let previous_program = old.and_then(|id| match &anchors.entries[&id].source {
        PoseSource::Program(p) => Some(p.entity),
        _ => None,
    });
    if let Some(PoseSource::Program(p)) = &mut source {
        if let Some(previous) = old.and_then(|id| world.resource::<Anchors>().entries.get(&id)) {
            if let PoseSource::Program(previous) = &previous.source {
                if previous.program.inputs.len() == p.program.inputs.len()
                    && previous
                        .program
                        .inputs
                        .iter()
                        .zip(&p.program.inputs)
                        .all(|(a, b)| a.key() == b.key() && a.hold() == b.hold())
                {
                    p.last = previous.last;
                }
            }
        }
        p.entity = world
            .spawn((GlobalTransform::IDENTITY, AnchorAvailability(false)))
            .id();
    }
    if let Some(entity) = previous_program {
        world.despawn(entity);
    }
    let mut anchors = world.resource_mut::<Anchors>();
    if let Some(source) = source {
        let id = old.unwrap_or_else(|| {
            anchors.next += 1;
            anchors.next
        });
        anchors.keys.insert(request.key.clone(), id);
        anchors.entries.insert(
            id,
            Publication {
                owner,
                request_id: request.id,
                key: request.key.clone(),
                source,
            },
        );
    } else if let Some(id) = old {
        anchors.keys.remove(&request.key);
        anchors.entries.remove(&id);
    }
    anchors.generation = anchors.generation.wrapping_add(1);
    Ok(())
}

fn sync_bindings(world: &mut World) {
    let Some(specs) = world.get_resource::<ElemenetWorld>() else {
        return;
    };
    let Some(policies) = world.get_resource::<SpacePolicies>() else {
        return;
    };
    let stale_refs: Vec<_> = world
        .resource::<NativeReferences>()
        .names
        .iter()
        .filter(|((owner, _), _)| !specs.0.entities().is_alive(*owner))
        .map(|(name, &id)| (name.clone(), id))
        .collect();
    let dead: Vec<_> = world
        .resource::<Anchors>()
        .entries
        .iter()
        .filter(|(_, p)| {
            !specs.0.entities().is_alive(p.owner)
                || !policies
                    .by_space
                    .get(&p.owner.id())
                    .is_some_and(|s| s.effective_caps.contains(CapabilityBits::CREATE_ANCHOR))
        })
        .map(|(&id, _)| id)
        .collect();
    if !stale_refs.is_empty() {
        let mut refs = world.resource_mut::<NativeReferences>();
        for (name, id) in stale_refs {
            refs.names.remove(&name);
            refs.slots.remove(&id);
            refs.requirements.remove(&id);
        }
    }
    if !dead.is_empty() {
        let mut program_entities = Vec::new();
        let mut anchors = world.resource_mut::<Anchors>();
        for id in dead {
            if let Some(p) = anchors.entries.remove(&id) {
                anchors.keys.remove(&p.key);
                if let PoseSource::Program(p) = p.source {
                    program_entities.push(p.entity);
                }
            }
        }
        anchors.generation = anchors.generation.wrapping_add(1);
        drop(anchors);
        for entity in program_entities {
            world.despawn(entity);
        }
    }
    // Discover only when the render entity map changes or an anchor attribute
    // changes. Animated transforms and unrelated JSON/DOM changes do not scan
    // all scene attributes every frame.
    let full = world.resource::<BindingVersion>().0.is_none()
        || world
            .get_resource_ref::<EntityMap>()
            .is_some_and(|m| m.is_changed());
    let generation = world.resource::<Anchors>().generation;
    let dirty = std::mem::take(&mut world.resource_mut::<AnchorDirty>().0);
    if !full && dirty.is_empty() && world.resource::<BindingVersion>().0 == Some(generation) {
        return;
    }
    let specs = world.resource::<ElemenetWorld>();
    let attrs = specs.0.read_storage::<Attrs>();
    let entities = specs.0.entities();
    let candidates: Vec<_> = if full {
        (&entities, &attrs)
            .join()
            .filter(|(_, attrs)| attrs.0.contains_key("anchor"))
            .map(|(node, _)| node)
            .collect()
    } else {
        dirty.iter().map(|id| entities.entity(*id)).collect()
    };
    let desired: Vec<_> = candidates
        .into_iter()
        .filter_map(|node| {
            let attrs = attrs.get(node)?;
            let key = attrs.0.get("anchor")?.trim();
            if key.is_empty() {
                return None;
            }
            let entity = *world.get_resource::<EntityMap>()?.0.get(&node.id())?;
            let owner = crate::js::find_owner_space_id(&specs.0, node)?;
            Some((entity, key.to_string(), entities.entity(owner)))
        })
        .collect();
    drop(attrs);
    drop(entities);
    let wanted: HashSet<_> = desired.iter().map(|d| d.0).collect();
    let affected: HashSet<_> = dirty
        .iter()
        .filter_map(|id| world.get_resource::<EntityMap>()?.0.get(id).copied())
        .collect();
    let removed: Vec<_> = world
        .query_filtered::<Entity, With<Follower>>()
        .iter(world)
        .filter(|e| (full || affected.contains(e)) && !wanted.contains(e))
        .collect();
    for entity in removed {
        world.entity_mut(entity).remove::<Follower>();
        if let Some(mut t) = world.get_mut::<Transform>(entity) {
            t.set_changed();
        }
        if let Some(mut v) = world.get_mut::<Visibility>(entity) {
            v.set_changed();
        }
    }
    for (entity, key, owner) in desired {
        let source = world.resource::<Anchors>().keys.get(&key).copied();
        world
            .entity_mut(entity)
            .insert(Follower { key, owner, source });
    }
    let updates: Vec<_> = world
        .query::<(Entity, &Follower)>()
        .iter(world)
        .map(|(e, f)| (e, world.resource::<Anchors>().keys.get(&f.key).copied()))
        .collect();
    for (entity, id) in updates {
        world.get_mut::<Follower>(entity).unwrap().source = id;
    }
    world.resource_mut::<BindingVersion>().0 = Some(generation);
}

fn is_descendant(world: &World, mut node: Entity, root: Entity) -> bool {
    for _ in 0..256 {
        if node == root {
            return true;
        }
        let Some(parent) = world.get::<Parent>(node) else {
            return false;
        };
        node = parent.get();
    }
    true // Do not accept malformed/deep source chains as safe dependencies.
}

fn source_entity(world: &World, p: &Publication) -> Option<Entity> {
    match &p.source {
        PoseSource::Native(id) => world
            .resource::<NativeReferences>()
            .slots
            .get(id)
            .copied()
            .flatten(),
        PoseSource::Element(node) => world
            .resource::<ElemenetWorld>()
            .0
            .entities()
            .is_alive(*node)
            .then(|| world.resource::<EntityMap>().0.get(&node.id()).copied())
            .flatten(),
        PoseSource::Program(p) => Some(p.entity),
    }
}
fn source_allowed(world: &World, id: u64, consumer: u32, stack: &mut Vec<u64>) -> bool {
    if stack.len() >= 32 || stack.contains(&id) {
        return false;
    }
    let Some(publication) = world.resource::<Anchors>().entries.get(&id) else {
        return false;
    };
    stack.push(id);
    let allowed = match &publication.source {
        PoseSource::Native(id) => {
            let (required, provider) = world
                .resource::<NativeReferences>()
                .requirements
                .get(id)
                .copied()
                .unwrap_or_default();
            let provider_allowed = provider.is_empty()
                || world
                    .resource::<SpacePolicies>()
                    .by_space
                    .get(&publication.owner.id())
                    .is_some_and(|p| p.effective_native.contains(provider));
            provider_allowed
                && (required.is_empty()
                    || world
                        .resource::<SpacePolicies>()
                        .by_space
                        .get(&consumer)
                        .is_some_and(|p| p.effective_caps.contains(required)))
        }
        PoseSource::Program(p) => p.program.inputs.iter().enumerate().all(|(index, _)| {
            p.bindings[index].is_none_or(|id| source_allowed(world, id, consumer, stack))
        }),
        PoseSource::Element(_) => {
            let mut node = source_entity(world, publication);
            let mut ok = true;
            for _ in 0..256 {
                let Some(e) = node else {
                    break;
                };
                if let Some(f) = world.get::<Follower>(e) {
                    if f.source
                        .is_none_or(|id| !source_allowed(world, id, consumer, stack))
                    {
                        ok = false;
                        break;
                    }
                }
                node = world.get::<Parent>(e).map(|p| p.get());
            }
            ok && node.is_none()
        }
    };
    stack.pop();
    allowed
}
fn execute_program(
    world: &mut World,
    id: u64,
    bound: &HashSet<Entity>,
    done: &HashSet<Entity>,
    unavailable: &HashSet<Entity>,
) -> Option<bool> {
    let (program, owner, entity, mut last, mut bindings, generation) =
        match &world.resource::<Anchors>().entries[&id].source {
            PoseSource::Program(p) => (
                p.program.clone(),
                world.resource::<Anchors>().entries[&id].owner,
                p.entity,
                p.last,
                p.bindings,
                p.generation,
            ),
            _ => unreachable!(),
        };
    let current_generation = world.resource::<Anchors>().generation;
    if generation != current_generation {
        for (index, input) in program.inputs.iter().enumerate() {
            bindings[index] = world.resource::<Anchors>().keys.get(input.key()).copied();
        }
        if let PoseSource::Program(p) = &mut world
            .resource_mut::<Anchors>()
            .entries
            .get_mut(&id)
            .unwrap()
            .source
        {
            p.bindings = bindings;
            p.generation = current_generation;
        }
    }
    let mut inputs = [GlobalTransform::IDENTITY; 16];
    let mut valid = true;
    for (index, input) in program.inputs.iter().enumerate() {
        let publication = bindings[index].and_then(|id| {
            world
                .resource::<Anchors>()
                .entries
                .get(&id)
                .map(|p| (id, p))
        });
        let Some((source_id, p)) = publication else {
            last[index] = None;
            valid = false;
            continue;
        };
        let revision = p.request_id;
        if !source_allowed(world, source_id, owner.id(), &mut Vec::new()) {
            last[index] = None;
            valid = false;
            continue;
        }
        let source = source_entity(world, p);
        let mut ancestor = source;
        let mut available = true;
        while let Some(e) = ancestor {
            if bound.contains(&e) && !done.contains(&e) {
                return None;
            }
            if unavailable.contains(&e) {
                available = false;
            }
            ancestor = world.get::<Parent>(e).map(|p| p.get());
        }
        let pose = source
            .filter(|_| available)
            .and_then(|e| world.get::<GlobalTransform>(e).copied());
        if let Some(pose) = pose {
            last[index] = Some((source_id, revision, pose));
            inputs[index] = pose;
        } else if input.hold() {
            if let Some((old_id, old_revision, pose)) =
                last[index].filter(|(old_id, old_revision, _)| {
                    *old_id == source_id && *old_revision == revision
                })
            {
                let _ = (old_id, old_revision);
                inputs[index] = pose;
            } else {
                valid = false;
            }
        } else {
            last[index] = None;
            valid = false;
        }
    }
    let pose = valid
        .then(|| program.run(&inputs[..program.inputs.len()]))
        .flatten();
    if let Some(pose) = pose {
        if let Some(mut current) = world.get_mut::<GlobalTransform>(entity) {
            if *current != pose {
                *current = pose;
            }
        }
    }
    world
        .entity_mut(entity)
        .insert(AnchorAvailability(pose.is_some()));
    if let PoseSource::Program(p) = &mut world
        .resource_mut::<Anchors>()
        .entries
        .get_mut(&id)
        .unwrap()
        .source
    {
        p.last = last;
    }
    Some(pose.is_some())
}
pub(crate) fn follow(world: &mut World) {
    if world.resource::<Anchors>().entries.is_empty()
        && world
            .query_filtered::<Entity, With<Follower>>()
            .iter(world)
            .next()
            .is_none()
    {
        return;
    }
    let mut pending: Vec<_> = world
        .query::<(Entity, &Follower)>()
        .iter(world)
        .map(|(e, f)| (e, f.source, f.owner, false))
        .collect();
    pending.extend(world.resource::<Anchors>().entries.iter().filter_map(
        |(&id, p)| match &p.source {
            PoseSource::Program(program) => Some((program.entity, Some(id), p.owner, true)),
            _ => None,
        },
    ));
    let bound: HashSet<_> = pending.iter().map(|p| p.0).collect();
    let mut done = HashSet::new();
    let mut unavailable = HashSet::new();
    while !pending.is_empty() {
        let previous = pending.len();
        pending.retain(|(root, id, owner, is_program)| {
            if *is_program {
                let Some(available) =
                    execute_program(world, id.unwrap(), &bound, &done, &unavailable)
                else {
                    return true;
                };
                if !available {
                    unavailable.insert(*root);
                }
                done.insert(*root);
                return false;
            }
            // An anchored logical parent must be placed before its child.
            let mut ancestor = world.get::<Parent>(*root).map(|p| p.get());
            while let Some(node) = ancestor {
                if bound.contains(&node) && !done.contains(&node) {
                    return true;
                }
                ancestor = world.get::<Parent>(node).map(|p| p.get());
            }
            let publication = id.and_then(|id| world.resource::<Anchors>().entries.get(&id));
            let source = publication.and_then(|p| source_entity(world, p));
            let permitted =
                id.is_none_or(|id| source_allowed(world, id, owner.id(), &mut Vec::new()));
            let mut dependency = source;
            let mut dependency_available = true;
            while let Some(node) = dependency {
                if node == *root {
                    break;
                }
                if bound.contains(&node) && !done.contains(&node) {
                    return true;
                }
                if unavailable.contains(&node) {
                    dependency_available = false;
                }
                dependency = world.get::<Parent>(node).map(|p| p.get());
            }
            let valid = permitted
                && dependency_available
                && source.is_some_and(|s| !is_descendant(world, s, *root));
            let pose = source
                .filter(|_| valid)
                .and_then(|s| world.get::<GlobalTransform>(s).copied())
                .zip(world.get::<Transform>(*root).copied())
                .map(|(pose, offset)| pose.mul_transform(offset));
            place_subtree(world, *root, pose);
            if pose.is_none() {
                unavailable.insert(*root);
            }
            done.insert(*root);
            false
        });
        if pending.len() == previous {
            for (root, _, _, is_program) in pending.drain(..) {
                if is_program {
                    world.entity_mut(root).insert(AnchorAvailability(false));
                } else {
                    place_subtree(world, root, None);
                }
            }
        }
    }
}

fn place_subtree(world: &mut World, root: Entity, pose: Option<GlobalTransform>) {
    let available = pose.is_some();
    world.entity_mut(root).insert(AnchorAvailability(available));
    let parent_visible = world
        .get::<Parent>(root)
        .and_then(|p| world.get::<InheritedVisibility>(p.get()))
        .map(|v| v.get())
        .unwrap_or(true);
    let mut queue = vec![(root, pose, parent_visible)];
    while let Some((entity, pose, parent_visible)) = queue.pop() {
        let visible = available
            && match world
                .get::<Visibility>(entity)
                .copied()
                .unwrap_or(Visibility::Inherited)
            {
                Visibility::Hidden => false,
                Visibility::Visible => true,
                Visibility::Inherited => parent_visible,
            };
        if let Some(mut v) = world.get_mut::<InheritedVisibility>(entity) {
            let next = if visible {
                InheritedVisibility::VISIBLE
            } else {
                InheritedVisibility::HIDDEN
            };
            if *v != next {
                *v = next;
            }
        }
        if let Some(pose) = pose {
            if let Some(mut global) = world.get_mut::<GlobalTransform>(entity) {
                if *global != pose {
                    *global = pose;
                }
            }
        }
        let children: Vec<_> = world
            .get::<Children>(entity)
            .map(|c| c.iter().copied().collect())
            .unwrap_or_default();
        for child in children {
            let child_pose = pose
                .zip(world.get::<Transform>(child).copied())
                .map(|(p, t)| p.mul_transform(t));
            queue.push((child, child_pose, visible));
        }
    }
}

/// Capture after native following, once per render frame, using the same clock
/// for every subscriber and both hands. No snapshots or DOM pose writes.
fn capture_listeners(world: &mut World) {
    let workers: Vec<_> = world
        .get_non_send_resource::<crate::js::ScriptRuntimeManager>()
        .map(|manager| {
            manager
                .contexts
                .iter()
                .filter(|(_, w)| {
                    w.anchor_port
                        .active
                        .load(std::sync::atomic::Ordering::Acquire)
                })
                .map(|(id, worker)| (*id, worker.anchor_port.clone(), worker.cmd_tx.clone()))
                .collect()
        })
        .unwrap_or_default();
    if workers.is_empty() {
        return;
    }
    world.resource_mut::<AnchorFrame>().0 += 1;
    let frame_id = world.resource::<AnchorFrame>().0;
    let captured = std::time::Instant::now();
    for (owner, port, wake) in workers {
        if capture_port(world, owner, &port, frame_id, captured) {
            // A full queue already guarantees the worker will wake. It checks
            // input before reading that backlog, so the wake can be discarded.
            let _ = wake.try_send(crate::js::JsWorkerCommand::PumpAnchorInput);
        }
    }
}
fn capture_port(
    world: &World,
    owner_id: u32,
    port: &js_runtime::anchors::AnchorPort,
    frame_id: u64,
    captured: std::time::Instant,
) -> bool {
    let mut wake = false;
    for subscription in port.subscriptions() {
        let samples = subscription
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let pose = listener_pose(world, owner_id, target);
                let mut sample = js_runtime::anchors::Sample {
                    index,
                    available: false,
                    position: [0.; 3],
                    rotation: [0., 0., 0., 1.],
                    scale: [1.; 3],
                    error: None,
                    input: None,
                };
                match pose {
                    Ok(Some((pose, input))) => {
                        sample.input = input;
                        let (scale, rotation, position) = pose.to_scale_rotation_translation();
                        sample.available = true;
                        sample.position = position.to_array();
                        sample.rotation = rotation.to_array();
                        sample.scale = scale.to_array();
                    }
                    Ok(None) => {}
                    Err(error) => sample.error = Some(error.into()),
                }
                sample
            })
            .collect();
        wake |= port.publish(
            subscription.id,
            js_runtime::anchors::CapturedFrame {
                frame_id,
                captured,
                samples,
                discontinuous: false,
            },
        );
    }
    wake
}
fn listener_pose(
    world: &World,
    owner_id: u32,
    target: &js_runtime::anchors::ListenTarget,
) -> Result<
    Option<(
        GlobalTransform,
        Option<js_runtime::anchors::ControllerInput>,
    )>,
    &'static str,
> {
    let specs = world
        .get_resource::<ElemenetWorld>()
        .ok_or("Missing document")?;
    let owner = specs.0.entities().entity(owner_id);
    if !specs.0.entities().is_alive(owner) {
        return Err("Listener owner closed");
    }
    let resolve = |publication: &Publication| -> Result<Option<Entity>, &'static str> {
        match publication.source {
            PoseSource::Element(node) => Ok(specs
                .0
                .entities()
                .is_alive(node)
                .then(|| world.resource::<EntityMap>().0.get(&node.id()).copied())
                .flatten()),
            PoseSource::Native(id) => {
                let refs = world.resource::<NativeReferences>();
                let required = refs.requirements.get(&id).map(|r| r.0).unwrap_or_default();
                if !required.is_empty()
                    && !world
                        .get_resource::<SpacePolicies>()
                        .and_then(|p| p.by_space.get(&owner_id))
                        .is_some_and(|p| p.effective_caps.contains(required))
                {
                    return Err("Listener lacks pose permission");
                }
                Ok(refs.slots.get(&id).copied().flatten())
            }
            PoseSource::Program(ref p) => {
                let id = world
                    .resource::<Anchors>()
                    .keys
                    .get(&publication.key)
                    .copied()
                    .ok_or("Missing program")?;
                if !source_allowed(world, id, owner_id, &mut Vec::new()) {
                    return Err("Listener lacks pose permission");
                }
                Ok(world
                    .get::<AnchorAvailability>(p.entity)
                    .filter(|a| a.0)
                    .map(|_| p.entity))
            }
        }
    };
    let entity = if let Some(key) = &target.key {
        let anchors = world.resource::<Anchors>();
        let Some(publication) = anchors.keys.get(key).and_then(|id| anchors.entries.get(id)) else {
            return Ok(None);
        };
        resolve(publication)?
    } else {
        let node = world
            .get_resource::<SpaceHandleTables>()
            .and_then(|t| t.by_space.get(&owner_id))
            .and_then(|t| t.local_to_global.get(&target.node?))
            .copied()
            .ok_or("Invalid listener element")?;
        let element = specs.0.entities().entity(node);
        if !specs.0.entities().is_alive(element)
            || crate::js::find_owner_space_id(&specs.0, element) != Some(owner_id)
        {
            return Err("Listener element belongs to another space");
        }
        world.resource::<EntityMap>().0.get(&node).copied()
    };
    let mut input = entity
        .and_then(|e| world.get::<NativeControllerInput>(e))
        .map(|i| i.0.clone());
    let mut pending: Vec<_> = entity.into_iter().collect();
    let mut visited = HashSet::new();
    for _ in 0..256 {
        let Some(node) = pending.pop() else {
            break;
        };
        if !visited.insert(node) {
            continue;
        }
        if let Some(follower) = world.get::<Follower>(node) {
            let Some(publication) = follower
                .source
                .and_then(|id| world.resource::<Anchors>().entries.get(&id))
            else {
                return Ok(None);
            };
            let source = resolve(publication)?;
            if source.is_none() || !world.get::<AnchorAvailability>(node).is_some_and(|a| a.0) {
                return Ok(None);
            }
            if input.is_none() {
                input = source
                    .and_then(|e| world.get::<NativeControllerInput>(e))
                    .map(|i| i.0.clone());
            }
            // Follow source dependencies too: a DOM publication can itself
            // follow another anchor. That must not bypass native permissions.
            pending.extend(source);
        }
        pending.extend(world.get::<Parent>(node).map(|p| p.get()));
    }
    if !pending.is_empty() {
        return Ok(None);
    }
    Ok(entity
        .and_then(|e| world.get::<GlobalTransform>(e).copied())
        .map(|pose| (pose, input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::SpacePolicy;
    use bevy::render::view::VisibilityPlugin;
    use bevy_xr_utils::tracking_utils::XrTrackedRightGrip;
    use specs::Builder;
    use virtual_dom::dom::element::{build_world, Hierarchy, Tag};

    struct Fixture {
        app: App,
        publisher: specs::Entity,
        viewer: specs::Entity,
        node: specs::Entity,
        object: Entity,
        child: Entity,
        parent: Entity,
    }
    fn fixture() -> Fixture {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            VisibilityPlugin,
            AnchorPlugin,
            crate::vr_input::VrInputPlugin,
        ));
        app.init_resource::<Assets<Mesh>>();
        let mut specs = build_world();
        let publisher = specs
            .create_entity()
            .with(Tag("space".into()))
            .with(Hierarchy::default())
            .build();
        let viewer = specs
            .create_entity()
            .with(Tag("space".into()))
            .with(Hierarchy::default())
            .build();
        let node = specs
            .create_entity()
            .with(Tag("group".into()))
            .with(Attrs(HashMap::from([(
                "anchor".into(),
                "right-grip".into(),
            )])))
            .with(Hierarchy::default())
            .build();
        Hierarchy::add_child(&mut specs, viewer, node);
        let parent = app
            .world_mut()
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                100., 0., 0.,
            )))
            .id();
        let object = app
            .world_mut()
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                1., 0., 0.,
            )))
            .id();
        let child = app
            .world_mut()
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                0., 2., 0.,
            )))
            .id();
        app.world_mut().entity_mut(object).set_parent(parent);
        app.world_mut().entity_mut(child).set_parent(object);
        app.insert_resource(ElemenetWorld(specs));
        app.insert_resource(EntityMap(HashMap::from([(node.id(), object)])));
        app.insert_resource(SpaceHandleTables::default());
        let mut policies = SpacePolicies::default();
        policies.by_space.insert(
            publisher.id(),
            SpacePolicy {
                effective_caps: CapabilityBits::CREATE_ANCHOR,
                effective_native: NativeServiceBits::VR_INPUT,
                ..default()
            },
        );
        policies.by_space.insert(
            viewer.id(),
            SpacePolicy {
                effective_caps: CapabilityBits::READ_POSE_STREAM,
                ..default()
            },
        );
        app.insert_resource(policies);
        crate::vr_input::prepare_references(app.world_mut());
        Fixture {
            app,
            publisher,
            viewer,
            node,
            object,
            child,
            parent,
        }
    }
    fn create(f: &mut Fixture, key: &str, source: Source) -> Option<String> {
        apply_requests(
            f.app.world_mut(),
            f.publisher.id(),
            vec![Request {
                id: 1,
                guard: None,
                key: key.into(),
                source: Some(source),
            }],
        )
        .pop()
        .unwrap()
        .error
    }
    fn right(f: &mut Fixture, x: f32) -> Entity {
        f.app
            .world_mut()
            .spawn((
                SpatialBundle::from_transform(Transform::from_xyz(x, 0., 0.)),
                XrTrackedRightGrip,
            ))
            .id()
    }

    #[test]
    fn programs_follow_current_inputs_hold_tracking_and_replace_atomically() {
        let mut f = fixture();
        assert!(create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into())
        )
        .is_none());
        assert!(create(
            &mut f,
            "left-grip",
            Source::Native("__vr_left_grip_root".into())
        )
        .is_none());
        right(&mut f, 0.);
        let left = f
            .app
            .world_mut()
            .spawn((
                SpatialBundle::from_transform(Transform::from_xyz(0., 0., -1.)),
                bevy_xr_utils::tracking_utils::XrTrackedLeftGrip,
            ))
            .id();
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps |= CapabilityBits::CREATE_ANCHOR;
        let definition = serde_json::json!({"type":"program","version":1,"inputs":{"a":"right-grip","b":{"anchor":"left-grip","holdLast":true}},"constants":{"up":[0,1,0],"forward":[0,0,-1]},"code":"input pose a\ninput pose b\nconst vec3 up\nconst vec3 forward\ntemp vec3 direction\noutput pose result = a\nsubtract direction, b.position, a.position\naimRotation result.rotation, direction, up, forward, up"});
        let register = |world: &mut World, value| {
            apply_requests(
                world,
                f.viewer.id(),
                vec![Request {
                    id: 40,
                    key: "computed".into(),
                    source: Some(Source::Program(value)),
                    guard: None,
                }],
            )
        };
        assert!(register(f.app.world_mut(), definition.clone())[0]
            .error
            .is_none());
        f.app
            .world_mut()
            .resource_mut::<ElemenetWorld>()
            .0
            .write_storage::<Attrs>()
            .get_mut(f.node)
            .unwrap()
            .0
            .insert("anchor".into(), "computed".into());
        f.app
            .world_mut()
            .resource_mut::<AnchorDirty>()
            .0
            .insert(f.node.id());
        f.app.update();
        f.app
            .world_mut()
            .get_mut::<Transform>(left)
            .unwrap()
            .translation = Vec3::new(1., 0., -1.);
        f.app.update();
        let rotation = f
            .app
            .world()
            .get::<GlobalTransform>(f.object)
            .unwrap()
            .compute_transform()
            .rotation;
        assert!((rotation * Vec3::NEG_Z - Vec3::new(1., 0., -1.).normalize()).length() < 1e-5);
        f.app.world_mut().despawn(left);
        f.app.update();
        assert!(f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        assert!(
            (f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .compute_transform()
                .rotation
                .dot(rotation))
            .abs()
                > 0.9999
        );
        let mut invalid = definition;
        invalid["code"] = serde_json::Value::String("bad instruction".into());
        assert!(register(f.app.world_mut(), invalid)[0].error.is_some());
        f.app.update();
        assert!(f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        let key = js_runtime::anchors::ListenTarget {
            key: Some("computed".into()),
            node: None,
        };
        assert!(listener_pose(f.app.world(), f.publisher.id(), &key).is_err());
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps
            .remove(CapabilityBits::READ_POSE_STREAM);
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
    }

    #[test]
    fn listeners_capture_effective_offsets_and_redact_revoked_or_lost_poses() {
        let mut f = fixture();
        assert!(create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into())
        )
        .is_none());
        let hand = right(&mut f, 3.);
        f.app
            .world_mut()
            .resource_mut::<SpaceHandleTables>()
            .by_space
            .entry(f.viewer.id())
            .or_default()
            .local_to_global
            .insert(5, f.node.id());
        f.app.update();
        let port = std::sync::Arc::new(js_runtime::anchors::AnchorPort::default());
        let mut engine = js_runtime::Engine::new();
        engine.configure_anchor_port(port.clone());
        engine.eval("globalThis.frames=[];dimension.anchors.listen(['right-grip',new HSMLElement(5)],f=>frames.push(f),{priority:'input',delivery:'history'});").unwrap();
        capture_port(
            f.app.world(),
            f.viewer.id(),
            &port,
            1,
            std::time::Instant::now(),
        );
        engine.pump_anchor_input().unwrap();
        engine.eval("if(frames[0].samples[0].position.x!==3||frames[0].samples[1].position.x!==4)throw Error('not rendered pose');").unwrap();
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps = CapabilityBits::empty();
        // Revocation is checked at capture, even before the next follower update.
        capture_port(
            f.app.world(),
            f.viewer.id(),
            &port,
            2,
            std::time::Instant::now(),
        );
        engine.pump_anchor_input().unwrap();
        engine.eval("if(!frames[1].discontinuous||frames[1].samples.some(s=>s.available||s.position.x!==0))throw Error('pose leaked');").unwrap();
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps = CapabilityBits::READ_POSE_STREAM;
        f.app.world_mut().despawn(hand);
        f.app.update();
        capture_port(
            f.app.world(),
            f.viewer.id(),
            &port,
            3,
            std::time::Instant::now(),
        );
        engine.pump_anchor_input().unwrap();
        engine
            .eval("if(frames[2].samples.some(s=>s.available))throw Error('stale pose');")
            .unwrap();
    }

    #[test]
    fn dom_publications_cannot_launder_native_listener_permissions() {
        let mut f = fixture();
        assert!(create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into())
        )
        .is_none());
        right(&mut f, 3.);
        f.app.update();
        // A rendered object following a DOM publication which follows VR.
        let object = f.app.world_mut().spawn(SpatialBundle::default()).id();
        f.app.world_mut().entity_mut(object).insert((
            Follower {
                key: "derived".into(),
                owner: f.publisher,
                source: Some(9),
            },
            AnchorAvailability(true),
        ));
        f.app.world_mut().resource_mut::<Anchors>().entries.insert(
            9,
            Publication {
                owner: f.publisher,
                request_id: 1,
                key: "derived".into(),
                source: PoseSource::Element(f.node),
            },
        );
        f.app
            .world_mut()
            .resource_mut::<NativeReferences>()
            .register(f.publisher, "__derived", Some(object));
        let id =
            f.app.world().resource::<NativeReferences>().names[&(f.publisher, "__derived".into())];
        f.app.world_mut().resource_mut::<Anchors>().entries.insert(
            10,
            Publication {
                owner: f.publisher,
                request_id: 1,
                key: "wrapped".into(),
                source: PoseSource::Native(id),
            },
        );
        f.app
            .world_mut()
            .resource_mut::<Anchors>()
            .keys
            .insert("wrapped".into(), 10);
        let target = js_runtime::anchors::ListenTarget {
            key: Some("wrapped".into()),
            node: None,
        };
        assert_eq!(
            listener_pose(f.app.world(), f.publisher.id(), &target).unwrap_err(),
            "Listener lacks pose permission"
        );
        assert!(listener_pose(f.app.world(), f.viewer.id(), &target)
            .unwrap()
            .is_some());
    }

    #[test]
    fn native_source_follows_in_the_same_frame_and_reconnects_without_js() {
        let mut f = fixture();
        assert!(create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into())
        )
        .is_none());
        f.app.update(); // Registration before the XR session is allowed.
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        let hand = right(&mut f, 3.);
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation(),
            Vec3::new(4., 0., 0.)
        );
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.child)
                .unwrap()
                .translation(),
            Vec3::new(4., 2., 0.)
        );
        assert_eq!(
            f.app.world().get::<Parent>(f.object).unwrap().get(),
            f.parent
        );
        assert_eq!(
            f.app
                .world()
                .get::<Transform>(f.object)
                .unwrap()
                .translation,
            Vec3::X
        );
        f.app
            .world_mut()
            .get_mut::<Transform>(hand)
            .unwrap()
            .translation
            .x = 7.;
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            8.
        );
        f.app.world_mut().despawn(hand);
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.child)
            .unwrap()
            .get());
        assert!(f.app.world().get_entity(f.object).is_some());
        right(&mut f, 10.);
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            11.
        );
        assert!(f
            .app
            .world()
            .get::<InheritedVisibility>(f.child)
            .unwrap()
            .get());
    }

    #[test]
    fn permissions_alias_scope_and_publication_collisions_are_enforced() {
        let mut f = fixture();
        assert!(create(&mut f, "bad", Source::Native("__missing".into())).is_some());
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.publisher.id())
            .unwrap()
            .effective_native = NativeServiceBits::empty();
        assert!(create(&mut f, "bad", Source::Native("__vr_right_grip_root".into())).is_some());
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.publisher.id())
            .unwrap()
            .effective_native = NativeServiceBits::VR_INPUT;
        assert!(create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into())
        )
        .is_none());
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps |= CapabilityBits::CREATE_ANCHOR;
        let collision = apply_requests(
            f.app.world_mut(),
            f.viewer.id(),
            vec![Request {
                id: 2,
                guard: None,
                key: "right-grip".into(),
                source: None,
            }],
        );
        assert!(collision[0].error.is_some());
        right(&mut f, 3.);
        f.app.update();
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.viewer.id())
            .unwrap()
            .effective_caps
            .remove(CapabilityBits::READ_POSE_STREAM);
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        f.app
            .world_mut()
            .resource_mut::<SpacePolicies>()
            .by_space
            .get_mut(&f.publisher.id())
            .unwrap()
            .effective_caps = CapabilityBits::empty();
        f.app
            .world_mut()
            .resource_mut::<ElemenetWorld>()
            .0
            .maintain();
        f.app.update();
        assert!(f.app.world().resource::<Anchors>().entries.is_empty());
    }

    #[test]
    fn removing_anchor_restores_authored_parent_pose_and_closing_publisher_releases_slots() {
        let mut f = fixture();
        create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into()),
        );
        right(&mut f, 4.);
        f.app.update();
        f.app
            .world()
            .resource::<ElemenetWorld>()
            .0
            .write_storage::<Attrs>()
            .get_mut(f.node)
            .unwrap()
            .0
            .remove("anchor");
        f.app
            .world_mut()
            .resource_mut::<AnchorDirty>()
            .0
            .insert(f.node.id());
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            101.
        );
        f.app
            .world()
            .resource::<ElemenetWorld>()
            .0
            .entities()
            .delete(f.publisher)
            .unwrap();
        f.app
            .world_mut()
            .resource_mut::<ElemenetWorld>()
            .0
            .maintain();
        f.app.update();
        assert!(f.app.world().resource::<Anchors>().entries.is_empty());
        assert!(f
            .app
            .world()
            .resource::<NativeReferences>()
            .slots
            .is_empty());
    }

    #[test]
    fn element_sources_follow_native_physics_and_cycles_are_unavailable() {
        let mut f = fixture();
        let source = {
            let mut specs = &mut f.app.world_mut().resource_mut::<ElemenetWorld>().0;
            let node = specs
                .create_entity()
                .with(Tag("group".into()))
                .with(Hierarchy::default())
                .build();
            Hierarchy::add_child(&mut specs, f.publisher, node);
            node
        };
        let source_entity = f
            .app
            .world_mut()
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                6., 0., 0.,
            )))
            .id();
        f.app
            .world_mut()
            .resource_mut::<EntityMap>()
            .0
            .insert(source.id(), source_entity);
        let mut table = crate::SpaceHandleTable::default();
        table.local_to_global.insert(4, source.id());
        f.app
            .world_mut()
            .resource_mut::<SpaceHandleTables>()
            .by_space
            .insert(f.publisher.id(), table);
        assert!(create(&mut f, "right-grip", Source::Element(4)).is_none());
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            7.
        );
        // A publication source beneath its consumer would depend on itself.
        f.app
            .world_mut()
            .entity_mut(source_entity)
            .set_parent(f.child);
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
    }

    #[test]
    fn old_handles_cannot_dispose_replaced_publications_and_foreign_nodes_are_rejected() {
        let mut f = fixture();
        create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into()),
        );
        let replies = apply_requests(
            f.app.world_mut(),
            f.publisher.id(),
            vec![
                Request {
                    id: 2,
                    key: "right-grip".into(),
                    source: Some(Source::Native("__vr_left_grip_root".into())),
                    guard: None,
                },
                Request {
                    id: 3,
                    key: "right-grip".into(),
                    source: None,
                    guard: Some(1),
                },
            ],
        );
        assert!(replies[0].error.is_none());
        assert!(replies[1].error.is_some());
        assert_eq!(f.app.world().resource::<Anchors>().entries.len(), 1);
        let mut table = crate::SpaceHandleTable::default();
        table.local_to_global.insert(4, f.node.id());
        f.app
            .world_mut()
            .resource_mut::<SpaceHandleTables>()
            .by_space
            .insert(f.publisher.id(), table);
        assert!(create(&mut f, "foreign", Source::Element(4)).is_some());
    }

    #[test]
    fn freshly_created_owned_element_can_publish_before_append_child_is_committed() {
        let mut f = fixture();
        let node = f
            .app
            .world_mut()
            .resource_mut::<ElemenetWorld>()
            .0
            .create_entity()
            .with(Tag("group".into()))
            .with(Hierarchy::default())
            .build();
        let mut table = crate::SpaceHandleTable::default();
        table.local_to_global.insert(4, node.id());
        table.detached_globals.insert(node.id());
        f.app
            .world_mut()
            .resource_mut::<SpaceHandleTables>()
            .by_space
            .insert(f.publisher.id(), table);
        assert!(create(&mut f, "right-grip", Source::Element(4)).is_none());
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        let source = f
            .app
            .world_mut()
            .spawn(SpatialBundle::from_transform(Transform::from_xyz(
                3., 0., 0.,
            )))
            .id();
        f.app
            .world_mut()
            .resource_mut::<EntityMap>()
            .0
            .insert(node.id(), source);
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            4.
        );
    }

    #[test]
    fn raw_grip_pivot_wins_over_visual_proxy_and_tracking_loss_hides_consumers() {
        use bevy_mod_xr::spaces::XrSpaceLocationFlags;
        use bevy_xr_utils::tracking_utils::RightGrip;
        let mut f = fixture();
        create(
            &mut f,
            "right-grip",
            Source::Native("__vr_right_grip_root".into()),
        );
        right(&mut f, 99.);
        let raw = f
            .app
            .world_mut()
            .spawn((
                SpatialBundle::from_transform(Transform::from_xyz(2., 0., 0.)),
                RightGrip,
                XrSpaceLocationFlags {
                    position_tracked: true,
                    rotation_tracked: true,
                    ..default()
                },
            ))
            .id();
        f.app.update();
        assert_eq!(
            f.app
                .world()
                .get::<GlobalTransform>(f.object)
                .unwrap()
                .translation()
                .x,
            3.
        );
        f.app
            .world_mut()
            .get_mut::<XrSpaceLocationFlags>(raw)
            .unwrap()
            .position_tracked = false;
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        f.app
            .world_mut()
            .get_mut::<XrSpaceLocationFlags>(raw)
            .unwrap()
            .position_tracked = true;
        f.app
            .insert_resource(bevy_mod_openxr::resources::OxrFrameState(
                openxr::FrameState {
                    predicted_display_time: openxr::Time::from_nanos(1),
                    predicted_display_period: openxr::Duration::from_nanos(1),
                    should_render: false,
                },
            ));
        f.app.update();
        assert!(!f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
        f.app
            .world_mut()
            .resource_mut::<bevy_mod_openxr::resources::OxrFrameState>()
            .0
            .should_render = true;
        f.app.update();
        assert!(f
            .app
            .world()
            .get::<InheritedVisibility>(f.object)
            .unwrap()
            .get());
    }
}
