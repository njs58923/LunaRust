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
#[derive(Resource, Default)]
pub struct ComponentBindings {
    bindings: HashMap<u32, Binding>,
    last: Option<(u64, u64)>,
    warnings: HashMap<u32, String>,
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
fn parent_space(mirror: &DomMirror, mut id: i32) -> Option<u32> {
    for _ in 0..256 {
        let n = mirror.nodes.get(&id)?;
        if n.tag == "space" {
            return Some(id as u32);
        }
        id = n.parent;
    }
    None
}
fn owning_include(mirror: &DomMirror, space: u32) -> Option<u32> {
    let mut id = mirror.nodes.get(&(space as i32))?.parent;
    for _ in 0..256 {
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
fn public_roots(mirror: &DomMirror, include: u32) -> Result<Vec<u32>, String> {
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
        let Some(n) = mirror.nodes.get(&id) else {
            continue;
        };
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
    world.init_resource::<ComponentBindings>();
    let Some(mirror) = world.get_resource::<DomMirror>() else {
        return;
    };
    let Some(manager) = world.get_non_send_resource::<ScriptRuntimeManager>() else {
        return;
    };
    let version = (mirror.version, manager.context_generation);
    if world.resource::<ComponentBindings>().last == Some(version)
        && world
            .resource::<ComponentBindings>()
            .bindings
            .iter()
            .all(|(id, b)| {
                b.channel.is_open()
                    && world
                        .get_resource::<crate::IncludeLoadStates>()
                        .and_then(|s| s.0.get(id))
                        .is_some_and(
                            |s| matches!(s,crate::IncludeLoadState::Loaded{url} if url==&b.key.src),
                        )
            })
    {
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
    // Inspect only worker ancestors and the small document-root boundary, not scene meshes.
    for &child in manager.contexts.keys() {
        let Some(id) = owning_include(mirror, child) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let node = &mirror.nodes[&(id as i32)];
        if !node.attrs.contains_key("props") && !node.attrs.contains_key("events") {
            continue;
        }
        let result = (|| -> Result<Candidate, String> {
            let roots = public_roots(mirror, id)?;
            if roots.len() != 1 {
                return Err("Component include requires exactly one public root space".into());
            }
            let child = roots[0];
            let child_worker = manager
                .contexts
                .get(&child)
                .ok_or("Component root has no worker")?;
            let owner =
                parent_space(mirror, node.parent).ok_or("Component include has no owning space")?;
            let parent_worker = manager
                .contexts
                .get(&owner)
                .ok_or("Component parent worker unavailable")?;
            let local = world
                .get_resource::<crate::SpaceHandleTables>()
                .and_then(|t| t.by_space.get(&owner))
                .and_then(|t| t.global_to_local.get(&id))
                .copied()
                .ok_or("Component include handle not ready")?;
            let specs = world
                .get_resource::<crate::ElemenetWorld>()
                .ok_or("Missing DOM")?;
            use specs::WorldExt;
            let fallback = world
                .get_resource::<crate::CurrentUrl>()
                .map(|u| u.0.as_str())
                .unwrap_or("");
            let raw = node.attrs.get("src").ok_or("Missing component src")?;
            let src = crate::dom::resolve_node_relative_url(
                &specs.0,
                specs.0.entities().entity(id),
                fallback,
                raw,
            )
            .ok_or("Invalid component src")?;
            if !world
                .get_resource::<crate::IncludeLoadStates>()
                .and_then(|s| s.0.get(&id))
                .is_some_and(|s| matches!(s,crate::IncludeLoadState::Loaded{url} if url==&src))
            {
                return Err("Component include not loaded for its current src".into());
            }
            let actual = crate::dom::find_node_base_url(
                &specs.0,
                specs.0.entities().entity(child),
                fallback,
            );
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
        parent.send(old.local_id, "reset".into(), r#"{"value":123}"#).unwrap();
        let child_port = world.non_send_resource::<ScriptRuntimeManager>().contexts[&ids["child"]]
            .component_port.clone();
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
        assert_eq!(owning_include(mirror, 100000), None);
        assert_eq!(
            public_roots(mirror, ids["door"]).unwrap(),
            vec![ids["child"]]
        );
        assert_eq!(owning_include(mirror, ids["child"]), Some(ids["door"]));
        assert_eq!(
            parent_space(mirror, mirror.nodes[&(ids["door"] as i32)].parent),
            Some(ids["parent"])
        );
    }
}
