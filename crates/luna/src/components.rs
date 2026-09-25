//! Host-owned include bindings; independent of shell tabs and resource capabilities.
use crate::js::{DomMirror, ScriptRuntimeManager};
use bevy::prelude::*;
use js_runtime::components::{self, Channel, ComponentPort};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct Binding {
    key: Key,
    parent: Arc<ComponentPort>,
    channel: Arc<Channel>,
}
#[derive(PartialEq, Eq)]
struct Key {
    parent: u32,
    child: u32,
    parent_port: usize,
    child_port: usize,
    include_parent: i32,
    src: String,
    events: String,
    local: i32,
}
/// Only the document boundary determines a component channel. Mesh transforms,
/// text, and private descendants must not reparse every include's props.
#[derive(Default)]
struct BindingInputs {
    nodes: HashMap<i32, NodeInput>,
    missing: HashSet<i32>,
    bases: HashMap<specs::Entity, BaseInput>,
    handles: HashMap<(u32, u32), Option<i32>>,
    loaded: HashMap<u32, Option<String>>,
    fallback: String,
}
struct NodeInput {
    tag: String,
    parent: i32,
    children: Option<Vec<i32>>,
    attrs: Option<[Option<String>; 3]>,
}
struct BaseInput {
    value: Option<String>,
    parent: Option<specs::Entity>,
}
impl BindingInputs {
    fn node(&mut self, mirror: &DomMirror, id: i32, children: bool, attrs: bool) {
        let Some(node) = mirror.nodes.get(&id) else {
            self.missing.insert(id);
            return;
        };
        let input = self.nodes.entry(id).or_insert_with(|| NodeInput {
            tag: node.tag.clone(),
            parent: node.parent,
            children: None,
            attrs: None,
        });
        if children {
            input.children = Some(node.children.clone());
        }
        if attrs {
            input.attrs = Some(["src", "props", "events"].map(|key| node.attrs.get(key).cloned()));
        }
    }

    fn base_url(&mut self, world: &specs::World, node: specs::Entity) -> String {
        use specs::WorldExt;
        use virtual_dom::dom::element::{BaseUrl, Hierarchy};
        let entities = world.entities();
        let bases = world.read_storage::<BaseUrl>();
        let hierarchies = world.read_storage::<Hierarchy>();
        let mut current = Some(node);
        while let Some(entity) = current {
            let value = bases.get(entity).map(|base| base.0.clone());
            let parent = hierarchies.get(entity).and_then(|h| h.parent);
            self.bases.insert(
                entity,
                BaseInput {
                    value: value.clone(),
                    parent,
                },
            );
            if let Some(value) = value {
                return value;
            }
            current = parent.filter(|parent| entities.is_alive(*parent));
        }
        self.fallback.clone()
    }

    fn external_matches(&self, world: &World) -> bool {
        let states = world.get_resource::<crate::IncludeLoadStates>();
        let tables = world.get_resource::<crate::SpaceHandleTables>();
        self.loaded.iter().all(|(id, expected)| {
            let loaded = states
                .and_then(|s| s.0.get(id))
                .and_then(|state| match state {
                    crate::IncludeLoadState::Loaded { url } => Some(url),
                    _ => None,
                });
            loaded == expected.as_ref()
        }) && self.handles.iter().all(|((owner, id), expected)| {
            tables
                .and_then(|t| t.by_space.get(owner))
                .and_then(|t| t.global_to_local.get(id))
                .copied()
                == *expected
        })
    }

    fn mirror_matches(&self, world: &World, mirror: &DomMirror) -> bool {
        if self.fallback
            != world
                .get_resource::<crate::CurrentUrl>()
                .map(|u| u.0.as_str())
                .unwrap_or("")
        {
            return false;
        }
        if self.missing.iter().any(|id| mirror.nodes.contains_key(id))
            || !self.nodes.iter().all(|(id, input)| {
                mirror.nodes.get(id).is_some_and(|node| {
                    input.tag == node.tag
                        && input.parent == node.parent
                        && input
                            .children
                            .as_ref()
                            .is_none_or(|children| children == &node.children)
                        && input.attrs.as_ref().is_none_or(|attrs| {
                            ["src", "props", "events"]
                                .iter()
                                .zip(attrs)
                                .all(|(key, value)| node.attrs.get(*key) == value.as_ref())
                        })
                })
            })
        {
            return false;
        }
        if self.bases.is_empty() {
            return true;
        }
        let Some(specs) = world.get_resource::<crate::ElemenetWorld>() else {
            return false;
        };
        use specs::WorldExt;
        use virtual_dom::dom::element::{BaseUrl, Hierarchy};
        let entities = specs.0.entities();
        let bases = specs.0.read_storage::<BaseUrl>();
        let hierarchies = specs.0.read_storage::<Hierarchy>();
        self.bases.iter().all(|(entity, input)| {
            entities.is_alive(*entity)
                && bases.get(*entity).map(|base| &base.0) == input.value.as_ref()
                && hierarchies.get(*entity).and_then(|h| h.parent) == input.parent
        })
    }
}
#[derive(Resource, Default)]
pub struct ComponentBindings {
    bindings: HashMap<u32, Binding>,
    last: Option<(u64, u64)>,
    warnings: HashMap<u32, String>,
    inputs: BindingInputs,
    #[cfg(test)]
    rebuilds: usize,
}
impl Drop for ComponentBindings {
    fn drop(&mut self) {
        for b in self.bindings.values() {
            b.channel.close();
            b.parent
                .remove_child(b.channel.local_id, &b.channel.generation);
        }
    }
}
fn parent_space(mirror: &DomMirror, mut id: i32, inputs: &mut BindingInputs) -> Option<u32> {
    for _ in 0..256 {
        inputs.node(mirror, id, false, false);
        let n = mirror.nodes.get(&id)?;
        if n.tag == "space" {
            return Some(id as u32);
        }
        id = n.parent;
    }
    None
}
fn owning_include(mirror: &DomMirror, space: u32, inputs: &mut BindingInputs) -> Option<u32> {
    inputs.node(mirror, space as i32, false, false);
    let mut id = mirror.nodes.get(&(space as i32))?.parent;
    for _ in 0..256 {
        inputs.node(mirror, id, false, false);
        let n = mirror.nodes.get(&id)?;
        if n.tag == "space" {
            return None;
        }
        if n.tag == "include" {
            return Some(id as u32);
        }
        id = n.parent;
    }
    None
}
fn public_roots(
    mirror: &DomMirror,
    include: u32,
    inputs: &mut BindingInputs,
) -> Result<Vec<u32>, String> {
    inputs.node(mirror, include as i32, true, false);
    let mut stack = mirror
        .nodes
        .get(&(include as i32))
        .ok_or("Include removed")?
        .children
        .clone();
    let mut roots = Vec::new();
    let mut visited = HashSet::new();
    while let Some(id) = stack.pop() {
        if visited.len() >= 4096 || !visited.insert(id) {
            return Err("Invalid or oversized include root hierarchy".into());
        }
        inputs.node(mirror, id, false, false);
        let Some(n) = mirror.nodes.get(&id) else {
            continue;
        };
        inputs.node(mirror, id, n.tag != "space" && n.tag != "include", false);
        if n.tag == "space" {
            roots.push(id as u32);
        } else if n.tag != "include" {
            stack.extend(n.children.iter().copied());
        }
    }
    Ok(roots)
}
fn origin(src: &str) -> String {
    let Ok(u) = url::Url::parse(src) else {
        return String::new();
    };
    let origin = u.origin().ascii_serialization();
    if origin != "null" {
        origin
    } else {
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""))
    }
}

pub fn sync_components(world: &mut World) {
    let _profile = crate::profiling::span("sync_components");
    world.init_resource::<ComponentBindings>();
    let Some(mirror) = world.get_resource::<DomMirror>() else {
        return;
    };
    let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
        return;
    };
    let version = (mirror.version, manager.context_generation);
    let registry = world.resource::<ComponentBindings>();
    if registry.last.is_some_and(|last| last.1 == version.1)
        && registry.bindings.values().all(|b| b.channel.is_open())
        && registry.inputs.external_matches(world)
        && (registry.last == Some(version) || registry.inputs.mirror_matches(world, mirror))
    {
        if registry.last != Some(version) {
            world.resource_mut::<ComponentBindings>().last = Some(version);
        }
        return;
    }
    struct Candidate {
        id: u32,
        key: Key,
        parent: Arc<ComponentPort>,
        child: Arc<ComponentPort>,
        props: serde_json::Value,
        events: std::collections::BTreeSet<String>,
        origin: String,
    }
    let mut candidates = Vec::new();
    let mut warnings = HashMap::new();
    let mut seen = HashSet::new();
    let mut inputs = BindingInputs {
        fallback: world
            .get_resource::<crate::CurrentUrl>()
            .map(|u| u.0.clone())
            .unwrap_or_default(),
        ..default()
    };
    // Inspect only worker ancestors and the small document-root boundary, not scene meshes.
    for &child in manager.contexts.keys() {
        let Some(id) = owning_include(mirror, child, &mut inputs) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let node = &mirror.nodes[&(id as i32)];
        inputs.node(mirror, id as i32, false, true);
        if !node.attrs.contains_key("props") && !node.attrs.contains_key("events") {
            continue;
        }
        let result = (|| -> Result<Candidate, String> {
            let roots = public_roots(mirror, id, &mut inputs)?;
            if roots.len() != 1 {
                return Err("Component include requires exactly one public root space".into());
            }
            let child = roots[0];
            let child_worker = manager
                .contexts
                .get(&child)
                .ok_or("Component root has no worker")?;
            let owner = parent_space(mirror, node.parent, &mut inputs)
                .ok_or("Component include has no owning space")?;
            let parent_worker = manager
                .contexts
                .get(&owner)
                .ok_or("Component parent worker unavailable")?;
            let local = world
                .get_resource::<crate::SpaceHandleTables>()
                .and_then(|t| t.by_space.get(&owner))
                .and_then(|t| t.global_to_local.get(&id))
                .copied();
            inputs.handles.insert((owner, id), local);
            let local = local.ok_or("Component include handle not ready")?;
            let specs = world
                .get_resource::<crate::ElemenetWorld>()
                .ok_or("Missing DOM")?;
            use specs::WorldExt;
            let raw = node.attrs.get("src").ok_or("Missing component src")?;
            let base = inputs.base_url(&specs.0, specs.0.entities().entity(id));
            let src =
                crate::render::resolve_remote_path(&base, raw).ok_or("Invalid component src")?;
            let loaded = world
                .get_resource::<crate::IncludeLoadStates>()
                .and_then(|s| s.0.get(&id))
                .and_then(|s| match s {
                    crate::IncludeLoadState::Loaded { url } => Some(url),
                    _ => None,
                });
            inputs.loaded.insert(id, loaded.cloned());
            if loaded != Some(&src) {
                return Err("Component include not loaded for its current src".into());
            }
            let actual = inputs.base_url(&specs.0, specs.0.entities().entity(child));
            if origin(&actual) != origin(&src) {
                return Err("Component origin differs from declared src".into());
            }
            let props = components::parse_json(
                node.attrs.get("props").map(String::as_str).unwrap_or("{}"),
                true,
            )?;
            let raw_events = node.attrs.get("events").cloned().unwrap_or_default();
            let events = components::parse_events(&raw_events)?;
            Ok(Candidate {
                id,
                key: Key {
                    parent: owner,
                    child,
                    parent_port: Arc::as_ptr(&parent_worker.component_port) as usize,
                    child_port: Arc::as_ptr(&child_worker.component_port) as usize,
                    include_parent: node.parent,
                    src: src.clone(),
                    events: raw_events,
                    local,
                },
                parent: parent_worker.component_port.clone(),
                child: child_worker.component_port.clone(),
                props,
                events,
                origin: origin(&src),
            })
        })();
        match result {
            Ok(c) => candidates.push(c),
            Err(e) => {
                warnings.insert(id, e);
            }
        }
    }
    let mut logs = Vec::new();
    world.resource_scope(|_, mut registry: Mut<ComponentBindings>| {
        let desired: HashMap<_, _> = candidates.iter().map(|c| (c.id, &c.key)).collect();
        registry.bindings.retain(|id, b| {
            if desired.get(id).is_some_and(|k| **k == b.key) && b.channel.is_open() {
                return true;
            }
            b.channel.close();
            b.parent
                .remove_child(b.channel.local_id, &b.channel.generation);
            logs.push(format!(
                "[component] include:{id} disconnected generation:{}",
                b.channel.generation
            ));
            false
        });
        for c in candidates {
            if let Some(b) = registry.bindings.get(&c.id) {
                b.channel.update_props(c.props);
                continue;
            }
            match Channel::new(
                &c.parent,
                &c.child,
                c.key.local,
                c.origin.clone(),
                c.props,
                c.events,
            ) {
                Ok(channel) => {
                    logs.push(format!(
                        "[component] include:{} connected origin:{} generation:{}",
                        c.id, c.origin, channel.generation
                    ));
                    registry.bindings.insert(
                        c.id,
                        Binding {
                            key: c.key,
                            parent: c.parent,
                            channel,
                        },
                    );
                }
                Err(e) => {
                    warnings.insert(c.id, e);
                }
            }
        }
        for (id, error) in &warnings {
            if registry.warnings.get(id) != Some(error) {
                logs.push(format!("[component] include:{id} rejected: {error}"));
            }
        }
        registry.warnings = warnings;
        registry.last = Some(version);
        registry.inputs = inputs;
        #[cfg(test)]
        {
            registry.rebuilds += 1;
        }
    });
    if let Some(mut panel) = world.get_resource_mut::<crate::LogPanel>() {
        for log in logs {
            panel.push_info(log);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use specs::{Join, WorldExt};
    use virtual_dom::dom::element::{build_world, Attrs, BaseUrl};
    fn fixture(extra: &str) -> (World, HashMap<String, u32>) {
        let mut specs = build_world();
        let root=virtual_dom::parse_xml(&mut specs,&format!(r#"<space id="parent"><include id="door" src="https://components.test/door.hsml" props='{{"title":"initial"}}' events="change"><space id="child"><script/></space>{extra}</include></space>"#)).unwrap();
        let ids: HashMap<_, _> = (&specs.entities(), &specs.read_storage::<Attrs>())
            .join()
            .filter_map(|(e, a)| a.0.get("id").map(|id| (id.clone(), e.id())))
            .collect();
        specs
            .write_storage::<BaseUrl>()
            .insert(root, BaseUrl("https://parent.test/world".into()))
            .unwrap();
        let child = specs.entities().entity(ids["child"]);
        specs
            .write_storage::<BaseUrl>()
            .insert(child, BaseUrl("https://components.test/door.hsml".into()))
            .unwrap();
        let nodes = specs.entities().join().map(|e| (e.id(), e)).collect();
        let mut world = World::new();
        world.insert_resource(crate::ElemenetWorld(specs));
        world.insert_resource(crate::VirtualDomData { nodes });
        world.init_resource::<crate::js::DomMirror>();
        world.init_resource::<crate::js::DomMirrorDirty>();
        world.init_resource::<crate::JsSnapshotState>();
        world.init_resource::<crate::SpaceHandleTables>();
        world.init_resource::<crate::LogPanel>();
        world.init_resource::<crate::PendingScripts>();
        world.insert_resource(crate::CurrentUrl("https://parent.test/world".into()));
        let mut states = crate::IncludeLoadStates::default();
        states.0.insert(
            ids["door"],
            crate::IncludeLoadState::Loaded {
                url: "https://components.test/door.hsml".into(),
            },
        );
        world.insert_resource(states);
        world.insert_non_send_resource(ScriptRuntimeManager::default());
        crate::js::js_update_snapshots_system(&mut world);
        (world, ids)
    }
    fn evaluate(world: &mut World, space: u32, code: &str) {
        world.resource_mut::<crate::PendingScripts>().0.push((
            space,
            "eval://component-test".into(),
            code.into(),
        ));
        // This function must bind initial props BEFORE it submits EvalScript.
        crate::js::js_eval_pending_scripts(world);
        let mut manager = world.non_send_resource_mut::<ScriptRuntimeManager>();
        let worker = manager.contexts.get_mut(&space).unwrap();
        loop {
            match worker
                .event_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
            {
                crate::js::JsWorkerEvent::EvalResult { error, .. } => {
                    assert!(error.is_none(), "{error:?}");
                    break;
                }
                crate::js::JsWorkerEvent::WorkerError(e) => panic!("{e}"),
                _ => {}
            }
        }
    }
    #[test]
    fn component_host_bootstrap_revisions_reload_and_permissions() {
        let (mut world, ids) = fixture("");
        evaluate(&mut world,ids["child"],"if(!component.connected||component.props.title!=='initial')throw Error('missing initial props');component.emit('change',{value:1});");
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
        let old = world.resource::<ComponentBindings>().bindings[&ids["door"]]
            .channel
            .clone();
        let parent = world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["parent"]]
            .component_port
            .clone();
        assert_eq!(parent.drain()[0].origin, "https://components.test");
        parent
            .send(old.local_id, "reset".into(), r#"{"value":123}"#)
            .unwrap();
        let child_port = world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
            .component_port
            .clone();
        assert!(child_port.take_wake());
        let messages = child_port.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].detail["value"], 123);
        assert!(child_port.validate_message(&messages[0].generation));
        // No UX_EMBED or root permission was required; only the declared event.
        evaluate(&mut world,ids["child"],"let denied=false;try{Deno.core.ops.op_component_emit('navigate','{}')}catch(e){denied=true}if(!denied)throw Error('undeclared event')");
        {
            let mut m = world.resource_mut::<DomMirror>();
            m.nodes
                .get_mut(&(ids["door"] as i32))
                .unwrap()
                .attrs
                .insert("props".into(), r#"{"title":"updated"}"#.into());
            m.version += 1;
        }
        sync_components(&mut world);
        assert_eq!(
            world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
                .component_port
                .context()
                .props["title"],
            "updated"
        );
        // A same-URL reload starts by removing the committed state, even before DOM changes.
        world
            .resource_mut::<crate::IncludeLoadStates>()
            .0
            .remove(&ids["door"]);
        sync_components(&mut world);
        assert!(!old.is_open());
    }
    #[test]
    fn component_host_rejects_ambiguous_roots_and_wrong_origin() {
        let (mut world, _) = fixture("<space><script/></space>");
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        assert!(world
            .resource::<ComponentBindings>()
            .warnings
            .values()
            .any(|e| e.contains("exactly one")));
        drop(world);
        let (mut world, ids) = fixture("");
        {
            let specs = world.resource::<crate::ElemenetWorld>();
            let node = specs.0.entities().entity(ids["child"]);
            specs
                .0
                .write_storage::<BaseUrl>()
                .insert(node, BaseUrl("https://unexpected.test/door".into()))
                .unwrap();
        }
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        assert!(world
            .resource::<ComponentBindings>()
            .warnings
            .values()
            .any(|e| e.contains("origin")));
    }
    #[test]
    fn component_root_search_does_not_export_private_subspaces() {
        let (mut world, ids) = fixture("");
        sync_components(&mut world);
        {
            let mut mirror = world.resource_mut::<DomMirror>();
            let mut private = mirror.nodes[&(ids["child"] as i32)].clone();
            private.parent = ids["child"] as i32;
            private.children.clear();
            mirror.nodes.insert(100000, private);
            mirror
                .nodes
                .get_mut(&(ids["child"] as i32))
                .unwrap()
                .children
                .push(100000);
        }
        let mirror = world.resource::<DomMirror>();
        let mut inputs = BindingInputs::default();
        assert_eq!(owning_include(mirror, 100000, &mut inputs), None);
        assert_eq!(
            public_roots(mirror, ids["door"], &mut inputs).unwrap(),
            vec![ids["child"]]
        );
        assert_eq!(
            owning_include(mirror, ids["child"], &mut inputs),
            Some(ids["door"])
        );
        assert_eq!(
            parent_space(
                mirror,
                mirror.nodes[&(ids["door"] as i32)].parent,
                &mut inputs
            ),
            Some(ids["parent"])
        );
    }

    fn set_mirror_attr(world: &mut World, id: u32, key: &str, value: &str) {
        let mut mirror = world.resource_mut::<DomMirror>();
        mirror
            .nodes
            .get_mut(&(id as i32))
            .unwrap()
            .attrs
            .insert(key.into(), value.into());
        mirror.version += 1;
    }

    #[test]
    fn component_bindings_ignore_render_changes_and_private_children() {
        let (mut world, ids) = fixture("");
        sync_components(&mut world);
        let registry = world.resource::<ComponentBindings>();
        let rebuilds = registry.rebuilds;
        let channel = registry.bindings[&ids["door"]].channel.clone();
        let child = world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
            .component_port
            .clone();
        child.take_wake();
        for i in 0..20 {
            set_mirror_attr(&mut world, ids["door"], "x", &i.to_string());
            set_mirror_attr(&mut world, ids["child"], "class", &i.to_string());
            {
                let mut mirror = world.resource_mut::<DomMirror>();
                let mut private = mirror.nodes[&(ids["child"] as i32)].clone();
                private.parent = ids["child"] as i32;
                private.children.clear();
                mirror.nodes.insert(100000 + i, private);
                mirror
                    .nodes
                    .get_mut(&(ids["child"] as i32))
                    .unwrap()
                    .children
                    .push(100000 + i);
                mirror.version += 1;
            }
            sync_components(&mut world);
        }
        assert_eq!(world.resource::<ComponentBindings>().rebuilds, rebuilds);
        assert!(channel.is_open());
        assert_eq!(child.context().revision, 1);
        assert!(!child.take_wake());
    }

    #[test]
    fn cached_component_inputs_preserve_props_events_reload_and_handle_retries() {
        let (mut world, ids) = fixture("");
        sync_components(&mut world);
        let initial = world.resource::<ComponentBindings>().bindings[&ids["door"]]
            .channel
            .clone();
        let child = world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
            .component_port
            .clone();
        set_mirror_attr(&mut world, ids["door"], "props", r#"{"title":"changed"}"#);
        sync_components(&mut world);
        assert!(initial.is_open());
        assert_eq!(child.context().props["title"], "changed");
        assert_eq!(child.context().revision, 2);
        set_mirror_attr(&mut world, ids["door"], "events", "change,ready");
        sync_components(&mut world);
        assert!(!initial.is_open());
        let next = world.resource::<ComponentBindings>().bindings[&ids["door"]]
            .channel
            .clone();
        assert_ne!(initial.generation, next.generation);
        // Host-only changes must be observed even with an unchanged mirror version.
        let local = world
            .resource_mut::<crate::SpaceHandleTables>()
            .by_space
            .get_mut(&ids["parent"])
            .unwrap()
            .global_to_local
            .remove(&ids["door"])
            .unwrap();
        sync_components(&mut world);
        assert!(!next.is_open());
        world
            .resource_mut::<crate::SpaceHandleTables>()
            .by_space
            .get_mut(&ids["parent"])
            .unwrap()
            .global_to_local
            .insert(ids["door"], local);
        sync_components(&mut world);
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
        world
            .resource_mut::<crate::IncludeLoadStates>()
            .0
            .remove(&ids["door"]);
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        world.resource_mut::<crate::IncludeLoadStates>().0.insert(
            ids["door"],
            crate::IncludeLoadState::Loaded {
                url: "https://components.test/door.hsml".into(),
            },
        );
        sync_components(&mut world);
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
    }

    #[test]
    fn cached_component_inputs_revalidate_origins_and_public_roots() {
        let (mut world, ids) = fixture("");
        sync_components(&mut world);
        let initial = world.resource::<ComponentBindings>().bindings[&ids["door"]]
            .channel
            .clone();
        {
            let specs = world.resource::<crate::ElemenetWorld>();
            let entity = specs.0.entities().entity(ids["child"]);
            specs
                .0
                .write_storage::<BaseUrl>()
                .insert(entity, BaseUrl("https://unexpected.test/door".into()))
                .unwrap();
        }
        world.resource_mut::<DomMirror>().version += 1;
        sync_components(&mut world);
        assert!(!initial.is_open());
        assert!(world.resource::<ComponentBindings>().warnings[&ids["door"]].contains("origin"));
        {
            let specs = world.resource::<crate::ElemenetWorld>();
            let entity = specs.0.entities().entity(ids["child"]);
            specs
                .0
                .write_storage::<BaseUrl>()
                .insert(entity, BaseUrl("https://components.test/door.hsml".into()))
                .unwrap();
        }
        world.resource_mut::<DomMirror>().version += 1;
        sync_components(&mut world);
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
        {
            let mut mirror = world.resource_mut::<DomMirror>();
            let mut root = mirror.nodes[&(ids["child"] as i32)].clone();
            root.children.clear();
            mirror.nodes.insert(100000, root);
            mirror
                .nodes
                .get_mut(&(ids["door"] as i32))
                .unwrap()
                .children
                .push(100000);
            mirror.version += 1;
        }
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        assert!(
            world.resource::<ComponentBindings>().warnings[&ids["door"]].contains("exactly one")
        );
    }

    #[test]
    fn cached_component_inputs_detect_declarations_and_reject_changed_sources() {
        let (mut world, ids) = fixture("");
        {
            let mut mirror = world.resource_mut::<DomMirror>();
            let include = mirror.nodes.get_mut(&(ids["door"] as i32)).unwrap();
            include.attrs.remove("props");
            include.attrs.remove("events");
            mirror.version += 1;
        }
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        set_mirror_attr(&mut world, ids["door"], "events", "ready");
        sync_components(&mut world);
        let channel = world.resource::<ComponentBindings>().bindings[&ids["door"]]
            .channel
            .clone();
        set_mirror_attr(
            &mut world,
            ids["door"],
            "src",
            "https://other.test/door.hsml",
        );
        sync_components(&mut world);
        assert!(!channel.is_open());
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        assert!(world.resource::<ComponentBindings>().warnings[&ids["door"]].contains("not loaded"));
        set_mirror_attr(
            &mut world,
            ids["door"],
            "src",
            "https://components.test/door.hsml",
        );
        sync_components(&mut world);
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
        set_mirror_attr(&mut world, ids["door"], "props", "[]");
        sync_components(&mut world);
        assert!(world.resource::<ComponentBindings>().bindings.is_empty());
        assert!(
            world.resource::<ComponentBindings>().warnings[&ids["door"]].contains("JSON object")
        );
    }

    #[test]
    #[ignore = "manual CPU benchmark of component binding validation, not frame time"]
    fn benchmark_static_component_bindings() {
        let (mut world, ids) = fixture("");
        let props = serde_json::json!({"recipe": (0..3000).collect::<Vec<_>>()}).to_string();
        set_mirror_attr(&mut world, ids["door"], "props", &props);
        sync_components(&mut world);
        let mut cached = Vec::new();
        let mut full = Vec::new();
        for sample in 0..200 {
            for force in if sample % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                set_mirror_attr(&mut world, ids["door"], "x", &sample.to_string());
                if force {
                    world.resource_mut::<ComponentBindings>().last = None;
                }
                let start = std::time::Instant::now();
                sync_components(&mut world);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                if force {
                    full.push(elapsed);
                } else {
                    cached.push(elapsed);
                }
            }
        }
        cached.sort_by(f64::total_cmp);
        full.sort_by(f64::total_cmp);
        assert_eq!(world.resource::<ComponentBindings>().bindings.len(), 1);
        assert_eq!(
            world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
                .component_port
                .context()
                .revision,
            1
        );
        println!(
            "props_bytes={} cached_median_ms={:.4} full_median_ms={:.4}",
            props.len(),
            cached[100],
            full[100]
        );
    }
}
