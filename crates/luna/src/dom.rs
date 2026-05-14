use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use anyhow::Result;
use bevy::prelude::*;
use specs::{Entity as SpecEntity, Join, ReadStorage, World as SpecWorld, WorldExt};
use tokio::runtime::Runtime;

use virtual_dom::{
    dom::{
        element::{build_world, Attrs, BaseUrl, Hierarchy, Tag, Transform2},
        hsml::{Include, Model, Script},
        TRANSFORM_POSITION, TRANSFORM_ROTATION, TRANSFORM_SCALE,
    },
    load_xml_from_url, parse_xml,
};

use crate::io::{
    clear_async_node_state, request_document_load, request_include_load, request_model_prepare,
    request_script_load,
};
use crate::render::{
    apply_transform, build_text_transform, get_or_create_primitive_material,
    get_or_create_text_material, parse_hex_color, parse_text_attrs, resolve_remote_path,
};
use crate::{
    ActiveDocumentLoad, AsyncDomParams, AttributeUpdates, CompletedDocumentLoad, CurrentUrl,
    DeleteRequests, Dirty, DirtyNodes, DocumentLoadState, ElemenetWorld, EntityMap, IoService,
    LoadedDocumentBundle, LogPanel, ModelLoadState, ModelLoadStates, NavigationEpoch,
    PendingDocumentLoads, PendingJsAttachNodes, PendingJsFirstRenderNodes, PendingModelLoads, PerformanceStats, ReloadTrigger, ScriptLoadState,
    ScriptLoadStates, SharedResources, TextRenderParams, TokioRuntime, VirtualDomData,
    VIRTUAL_ROUTES,
};

const DOM_SYNC_VERBOSE_LOGS: bool = false;
const ATTR_DELETE_SENTINEL: &str = "[DEL]";

pub fn commit_pending_js_attaches_system(
    mut pending_js_attaches: ResMut<PendingJsAttachNodes>,
    world: Res<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut pending_first_render: ResMut<PendingJsFirstRenderNodes>,
    mut space_handle_tables: ResMut<crate::SpaceHandleTables>,
) {
    if pending_js_attaches.0.is_empty() {
        return;
    }

    let pending: Vec<u32> = pending_js_attaches.0.drain(..).collect();
    let entities = world.0.entities();

    for node_id in &pending {
        let ent = entities.entity(*node_id);
        if !entities.is_alive(ent) {
            continue;
        }
        dom_data.nodes.insert(*node_id, ent);
        pending_first_render.0.push((*node_id, 1));
    }

    // Ahora que dom_data tiene los nodos, podemos sacarlos de
    // `detached_globals` sin riesgo de que `sync_space_handle_table` pode
    // su mapping local→global. Ver nota en apply_js_tick (hierarchy append).
    for table in space_handle_tables.by_space.values_mut() {
        for node_id in &pending {
            table.detached_globals.remove(node_id);
        }
    }
}

pub fn activate_pending_js_first_render_system(
    mut pending_first_render: ResMut<PendingJsFirstRenderNodes>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    if pending_first_render.0.is_empty() {
        return;
    }

    let pending = pending_first_render.0.drain(..).collect::<Vec<_>>();
    for (node_id, delay) in pending {
        if delay == 0 {
            dirty_nodes.0.push(node_id);
        } else {
            pending_first_render.0.push((node_id, delay - 1));
        }
    }
}

fn is_structural_tag(tag: &str) -> bool {
    matches!(
        tag,
        "" | "hsml" | "head" | "name" | "meta" | "state" | "group" | "div"
    )
}

fn set_node_base_url(world: &mut SpecWorld, node: SpecEntity, base_url: &str) {
    let mut base_urls = world.write_storage::<BaseUrl>();
    let _ = base_urls.insert(node, BaseUrl(base_url.to_string()));
}

pub(crate) fn find_node_base_url(
    world: &SpecWorld,
    node: SpecEntity,
    fallback_base_url: &str,
) -> String {
    let entities = world.entities();
    let hierarchies = world.read_storage::<Hierarchy>();
    let base_urls = world.read_storage::<BaseUrl>();

    let mut current = Some(node);
    while let Some(ent) = current {
        if let Some(base) = base_urls.get(ent) {
            return base.0.clone();
        }

        current = hierarchies
            .get(ent)
            .and_then(|h| h.parent)
            .and_then(|parent_id| {
                let parent = entities.entity(parent_id);
                entities.is_alive(parent).then_some(parent)
            });
    }

    fallback_base_url.to_string()
}

pub(crate) fn resolve_node_relative_url(
    world: &SpecWorld,
    node: SpecEntity,
    fallback_base_url: &str,
    remote_path: &str,
) -> Option<String> {
    let base_url = find_node_base_url(world, node, fallback_base_url);
    resolve_remote_path(&base_url, remote_path)
}

pub(crate) fn find_nearest_ancestor_include(
    world: &SpecWorld,
    node: SpecEntity,
) -> Option<SpecEntity> {
    let entities = world.entities();
    let hierarchies = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();

    let mut current = Some(node);
    while let Some(ent) = current {
        let parent_id = hierarchies.get(ent).and_then(|h| h.parent)?;
        let parent = entities.entity(parent_id);
        if !entities.is_alive(parent) {
            return None;
        }
        if matches!(tags.get(parent), Some(tag) if tag.0 == "include") {
            return Some(parent);
        }
        current = Some(parent);
    }
    None
}

fn include_ancestor_chain_contains(
    world: &SpecWorld,
    node: SpecEntity,
    candidate_url: &str,
    fallback_base_url: &str,
) -> bool {
    let entities = world.entities();
    let hierarchies = world.read_storage::<Hierarchy>();
    let attrs = world.read_storage::<Attrs>();
    let tags = world.read_storage::<Tag>();

    let mut current = Some(node);
    while let Some(ent) = current {
        let parent_id = hierarchies.get(ent).and_then(|h| h.parent);
        let Some(parent_id) = parent_id else {
            break;
        };

        let parent = entities.entity(parent_id);
        if !entities.is_alive(parent) {
            break;
        }

        if matches!(tags.get(parent), Some(tag) if tag.0 == "include") {
            if let Some(src) = attrs.get(parent).and_then(|a| a.0.get("src")) {
                let parent_base = find_node_base_url(world, parent, fallback_base_url);
                let ancestor_url =
                    resolve_remote_path(&parent_base, src).unwrap_or_else(|| src.clone());
                if ancestor_url == candidate_url {
                    return true;
                }
            }
        }

        current = Some(parent);
    }

    false
}

fn include_would_cycle(
    world: &SpecWorld,
    include_node: SpecEntity,
    candidate_url: &str,
    fallback_base_url: &str,
) -> bool {
    candidate_url == find_node_base_url(world, include_node, fallback_base_url)
        || include_ancestor_chain_contains(world, include_node, candidate_url, fallback_base_url)
}

fn primitive_color(attrs_storage: &ReadStorage<Attrs>, node: SpecEntity) -> Color {
    attrs_storage
        .get(node)
        .and_then(|a| a.0.get("color"))
        .and_then(|c| parse_hex_color(c))
        .unwrap_or(Color::srgb(0.5, 0.5, 0.5))
}

fn node_visible(attrs_storage: &ReadStorage<Attrs>, node: SpecEntity) -> bool {
    let Some(attrs) = attrs_storage.get(node) else {
        return true;
    };
    let Some(value) = attrs.0.get("visible") else {
        return true;
    };
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "0" | "no" | "off" | "hidden"
    )
}
fn node_touchable(attrs_storage: &ReadStorage<Attrs>, node: SpecEntity) -> bool {
    let Some(attrs) = attrs_storage.get(node) else {
        return false;
    };
    let Some(value) = attrs.0.get("touchable") else {
        return false;
    };
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}
fn spawn_colored_primitive(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    primitive_material_cache: &mut crate::PrimitiveMaterialCache,
    mesh: Handle<Mesh>,
    color: Color,
    transform: Transform,
    double_sided: bool,
    touchable_node_id: Option<u32>,
    hit_shape: Option<crate::touch::HitShape>,
) -> Entity {
    let material =
        get_or_create_primitive_material(primitive_material_cache, materials, color, double_sided);
    let mut entity_commands = commands.spawn((
        PbrBundle {
            mesh,
            material,
            transform,
            ..Default::default()
        },
        Dirty,
    ));
    if let Some(node_id) = touchable_node_id {
        entity_commands.insert(crate::touch::Toqueable(node_id));
        if let Some(shape) = hit_shape {
            entity_commands.insert(shape);
        }
    }
    entity_commands.id()
}

// ─── Reload ──────────────────────────────────────────────────────────────────

pub fn request_navigation_system(
    mut reload_trigger: ResMut<ReloadTrigger>,
    current_url: Res<CurrentUrl>,
    tokio_rt: Res<TokioRuntime>,
    io_service: Res<IoService>,
    mut nav_epoch: ResMut<NavigationEpoch>,
    mut document_load_state: ResMut<DocumentLoadState>,
    mut log_panel: ResMut<LogPanel>,
) {
    let url = current_url.0.clone();
    nav_epoch.0 += 1;
    let epoch = nav_epoch.0;
    document_load_state.0 = Some(ActiveDocumentLoad {
        epoch,
        url: url.clone(),
    });
    reload_trigger.0 = false;
    request_document_load(&tokio_rt.0, &io_service, epoch, url.clone());
    log_panel.push_info(format!("Queued document load (epoch {epoch}): {url}"));
}

pub fn commit_pending_document_load_system(
    mut world: ResMut<ElemenetWorld>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut commands: Commands,
    mut entity_map: ResMut<EntityMap>,
    mut pending_document_loads: ResMut<PendingDocumentLoads>,
    mut document_load_state: ResMut<DocumentLoadState>,
    mut log_panel: ResMut<LogPanel>,
    mut manager: NonSendMut<crate::js::ScriptRuntimeManager>,
    mut commit: crate::DocumentCommitParams,
    mut space_policies: ResMut<crate::permissions::SpacePolicies>,
) {
    let pending = pending_document_loads.0.drain(..).collect::<Vec<_>>();
    if pending.is_empty() {
        return;
    }

    for CompletedDocumentLoad { epoch, url, result } in pending {
        let is_active = matches!(
            document_load_state.0.as_ref(),
            Some(active) if active.epoch == epoch && active.url == url
        );
        if !is_active {
            log_panel.push_info(format!(
                "Dropping stale document load (epoch {epoch}): {url}"
            ));
            continue;
        }

        match result {
            Ok(bundle) => {
                let mut new_world = build_world();
                match flatten_loaded_document_bundle(&mut new_world, &url, &bundle, &mut log_panel)
                {
                    Ok((new_nodes, new_dirty)) => {
                        for worker in manager.contexts.values_mut() {
                            crate::js::stop_space_worker(worker);
                        }
                        manager.contexts.clear();
                        log_panel
                            .push_info("[JS] All JS contexts cleared for committed navigation");

                        commit.script_load_states.0.clear();
                        commit.pending_model_loads.0.clear();
                        commit.model_load_states.0.clear();
                        commit.pending_includes.0.clear();
                        commit.include_load_states.0.clear();
                        commit.attribute_updates.0.clear();
                        commit.delete_requests.0.clear();
                        commit.pending_scripts.0.clear();
                        commit.space_handle_tables.by_space.clear();
                        commit.space_handle_tables.next_runtime_id = 0;
                        commit.text_material_cache.materials.clear();
                        commit.primitive_material_cache.materials.clear();
                        // Drenar colas JS→DOM del espacio anterior. Si quedan,
                        // sus node_ids stale colisionan con slots del nuevo
                        // SPECS world (build_world() reinicia desde 0) y
                        // corrompen entidades del nuevo HSML.
                        commit.transform_updates.positions.clear();
                        commit.transform_updates.rotations.clear();
                        commit.transform_updates.scales.clear();
                        commit.transform_only_dirty.0.clear();
                        commit.pending_js_attaches.0.clear();
                        commit.pending_js_first_render.0.clear();

                        for bevy_ent in entity_map.0.drain().map(|(_, ent)| ent) {
                            commands.entity(bevy_ent).despawn_recursive();
                        }

                        world.0 = new_world;
                        dom_data.nodes = new_nodes;
                        dirty_nodes.0 = new_dirty;
                        document_load_state.0 = None;
                        commit.js_snapshot_state.dirty = true;
                        space_policies.dirty = true;
                        log_panel
                            .push_info(format!("Document commit complete (epoch {epoch}): {url}"));
                    }
                    Err(error) => {
                        document_load_state.0 = None;
                        log_panel.push_error(format!("Error committing XML from {url}: {error}"));
                    }
                }
            }
            Err(error) => {
                document_load_state.0 = None;
                log_panel.push_error(format!("Error loading document {url}: {error}"));
            }
        }
    }
}

// ─── Load & flatten ──────────────────────────────────────────────────────────

pub fn load_and_flatten_xml(
    world: &mut SpecWorld,
    url: &str,
    rt: &Runtime,
    log_panel: &mut LogPanel,
) -> Result<(HashMap<u32, SpecEntity>, Vec<u32>)> {
    log_panel.push_info(format!("Loading document from: {url}"));

    let xml_content = if crate::routes::VirtualRoutes::is_virtual_url(url) {
        match VIRTUAL_ROUTES.resolve(url) {
            Some(content) => {
                log_panel.push_info(format!("Virtual route resolved: {url}"));
                content
            }
            None => {
                log_panel.push_error(format!("Virtual route not found: {url}"));
                return Err(anyhow::anyhow!("Virtual route not found: {url}"));
            }
        }
    } else {
        match rt.block_on(load_xml_from_url(url)) {
            Ok(c) => c,
            Err(e) => return Err(anyhow::anyhow!("Error downloading XML from {url}: {e}")),
        }
    };

    flatten_loaded_xml(world, url, &xml_content, rt, log_panel)
}

pub fn flatten_loaded_xml(
    world: &mut SpecWorld,
    url: &str,
    xml_content: &str,
    rt: &Runtime,
    log_panel: &mut LogPanel,
) -> Result<(HashMap<u32, SpecEntity>, Vec<u32>)> {
    log_panel.push_info(format!(
        "Content retrieved. Length: {} chars",
        xml_content.len()
    ));

    let root_node =
        parse_xml(world, &xml_content).map_err(|e| anyhow::anyhow!("Error parsing XML: {e}"))?;
    set_node_base_url(world, root_node, url);

    let mut include_dirty = expand_includes(world, url, rt, log_panel).unwrap_or_default();
    finish_flatten(world, root_node, &mut include_dirty, log_panel)
}

pub fn flatten_loaded_document_bundle(
    world: &mut SpecWorld,
    url: &str,
    bundle: &LoadedDocumentBundle,
    log_panel: &mut LogPanel,
) -> Result<(HashMap<u32, SpecEntity>, Vec<u32>)> {
    log_panel.push_info(format!(
        "Content retrieved. Length: {} chars",
        bundle.root_xml.len()
    ));
    for warning in &bundle.warnings {
        log_panel.push_warn(warning.clone());
    }

    let root_node = parse_xml(world, &bundle.root_xml)
        .map_err(|e| anyhow::anyhow!("Error parsing XML: {e}"))?;
    set_node_base_url(world, root_node, url);

    let mut include_dirty = expand_includes_from_bundle(world, url, bundle, log_panel)?;
    finish_flatten(world, root_node, &mut include_dirty, log_panel)
}

fn finish_flatten(
    world: &SpecWorld,
    root_node: SpecEntity,
    include_dirty: &mut Vec<u32>,
    log_panel: &mut LogPanel,
) -> Result<(HashMap<u32, SpecEntity>, Vec<u32>)> {
    let mut map = HashMap::new();
    let mut dirty = Vec::new();
    let hierarchies = world.read_storage::<Hierarchy>();

    fn flatten_dom(
        node: SpecEntity,
        map: &mut HashMap<u32, SpecEntity>,
        dirty: &mut Vec<u32>,
        hierarchies: &ReadStorage<Hierarchy>,
    ) {
        dirty.push(node.id());
        map.insert(node.id(), node);
        if let Some(h) = hierarchies.get(node) {
            for &child in &h.children {
                flatten_dom(child, map, dirty, hierarchies);
            }
        }
    }

    flatten_dom(root_node, &mut map, &mut dirty, &hierarchies);
    dirty.extend(include_dirty.drain(..));
    dirty.sort_unstable();
    dirty.dedup();

    log_panel.push_info(format!("DOM tree parsed. {} nodes found.", map.len()));
    Ok((map, dirty))
}

pub fn collect_subtree_ids(world: &SpecWorld, root: SpecEntity, out: &mut Vec<u32>) {
    let hier = world.read_storage::<Hierarchy>();
    out.push(root.id());
    if let Some(h) = hier.get(root) {
        for &c in &h.children {
            collect_subtree_ids(world, c, out);
        }
    }
}

fn collect_bevy_subtree_roots_from_bevy(
    subtree_ids: &[u32],
    entity_map: &EntityMap,
    bevy_parents: &Query<&Parent>,
) -> Vec<Entity> {
    let subtree_bevy: HashSet<Entity> = subtree_ids
        .iter()
        .filter_map(|node_id| entity_map.0.get(node_id).copied())
        .collect();

    let mut seen = HashSet::new();
    let mut roots = Vec::new();

    for &bevy_ent in &subtree_bevy {
        let parent_inside_subtree = bevy_parents
            .get(bevy_ent)
            .ok()
            .map(|p| p.get())
            .is_some_and(|parent| subtree_bevy.contains(&parent));

        if !parent_inside_subtree && seen.insert(bevy_ent) {
            roots.push(bevy_ent);
        }
    }

    roots
}

fn remove_dom_subtree(
    world: &mut SpecWorld,
    root: SpecEntity,
    entity_map: &mut EntityMap,
    commands: &mut Commands,
    dom_data: &mut VirtualDomData,
    include_load_states: &mut crate::IncludeLoadStates,
    script_load_states: &mut ScriptLoadStates,
    pending_model_loads: &mut PendingModelLoads,
    model_load_states: &mut ModelLoadStates,
    space_handle_tables: &mut crate::SpaceHandleTables,
    bevy_parents: &Query<&Parent>,
) -> usize {
    if !world.entities().is_alive(root) {
        return 0;
    }

    let parent_id = {
        let hierarchies = world.read_storage::<Hierarchy>();
        hierarchies.get(root).and_then(|h| h.parent)
    };
    if let Some(parent_id) = parent_id {
        let parent = world.entities().entity(parent_id);
        let mut hierarchies = world.write_storage::<Hierarchy>();
        if let Some(parent_hierarchy) = hierarchies.get_mut(parent) {
            parent_hierarchy.children.retain(|child| *child != root);
        }
        if let Some(root_hierarchy) = hierarchies.get_mut(root) {
            root_hierarchy.parent = None;
        }
    }

    let mut subtree_ids = Vec::new();
    collect_subtree_ids(world, root, &mut subtree_ids);
    if subtree_ids.is_empty() {
        return 0;
    }

    // Usar roots REALES de Bevy, no inferidos desde Specs.
    // Esto permite volver a despawn_recursive() sin dejar huérfanos
    // y sin pagar el costo de despawnear entidad por entidad.
    let bevy_roots = collect_bevy_subtree_roots_from_bevy(&subtree_ids, entity_map, bevy_parents);

    for &node_id in &subtree_ids {
        clear_async_node_state(
            node_id,
            script_load_states,
            pending_model_loads,
            model_load_states,
        );
        include_load_states.0.remove(&node_id);
        dom_data.nodes.remove(&node_id);
        entity_map.0.remove(&node_id);

        for table in space_handle_tables.by_space.values_mut() {
            if let Some(local_id) = table.global_to_local.remove(&node_id) {
                table.local_to_global.remove(&local_id);
            }
            table.detached_globals.remove(&node_id);
        }
    }

    for bevy_root in bevy_roots {
        commands.entity(bevy_root).despawn_recursive();
    }

    for &node_id in &subtree_ids {
        let ent = world.entities().entity(node_id);
        let _ = world.delete_entity(ent);
    }

    subtree_ids.len()
}

pub fn expand_includes(
    world: &mut SpecWorld,
    base_url: &str,
    rt: &Runtime,
    log: &mut LogPanel,
) -> anyhow::Result<Vec<u32>> {
    expand_includes_with_loader(world, base_url, log, |final_url, log| {
        if crate::routes::VirtualRoutes::is_virtual_url(final_url) {
            match VIRTUAL_ROUTES.resolve(final_url) {
                Some(content) => {
                    log.push_info(format!("Virtual include: {}", final_url));
                    Ok(Some(content))
                }
                None => {
                    log.push_error(format!("Virtual include not found: {}", final_url));
                    Ok(None)
                }
            }
        } else {
            match rt.block_on(load_xml_from_url(final_url)) {
                Ok(x) => Ok(Some(x)),
                Err(e) => {
                    log.push_error(format!("include: download error {} -> {e}", final_url));
                    Ok(None)
                }
            }
        }
    })
}

pub fn expand_includes_from_bundle(
    world: &mut SpecWorld,
    base_url: &str,
    bundle: &LoadedDocumentBundle,
    log: &mut LogPanel,
) -> anyhow::Result<Vec<u32>> {
    expand_includes_with_loader(world, base_url, log, |final_url, log| {
        if let Some(content) = bundle.includes.get(final_url) {
            return Ok(Some(content.clone()));
        }
        if crate::routes::VirtualRoutes::is_virtual_url(final_url) {
            match VIRTUAL_ROUTES.resolve(final_url) {
                Some(content) => {
                    log.push_info(format!("Virtual include: {}", final_url));
                    return Ok(Some(content));
                }
                None => {
                    log.push_error(format!("Virtual include not found: {}", final_url));
                    return Ok(None);
                }
            }
        }
        log.push_warn(format!("include bundle missing: {}", final_url));
        Ok(None)
    })
}

fn expand_includes_with_loader<F>(
    world: &mut SpecWorld,
    base_url: &str,
    log: &mut LogPanel,
    mut load: F,
) -> anyhow::Result<Vec<u32>>
where
    F: FnMut(&str, &mut LogPanel) -> anyhow::Result<Option<String>>,
{
    let mut processed = HashSet::new();
    let mut new_dirty = Vec::new();

    loop {
        let targets: Vec<(SpecEntity, String)> = {
            let entities = world.entities();
            let includes_r = world.read_storage::<Include>();
            let mut v = Vec::new();
            for (ent, inc) in (&entities, &includes_r).join() {
                if processed.contains(&ent.id()) {
                    continue;
                }
                if let Some(ref src) = inc.src {
                    v.push((ent, src.clone()));
                }
            }
            v
        };

        if targets.is_empty() {
            break;
        }

        for (parent_ent, src) in targets {
            processed.insert(parent_ent.id());
            let Some(final_url) =
                resolve_node_relative_url(world, parent_ent, base_url, &src)
            else {
                log.push_warn(format!(
                    "include: cannot resolve src='{src}' against base='{base_url}'"
                ));
                continue;
            };
            if include_would_cycle(world, parent_ent, &final_url, base_url) {
                log.push_warn(format!(
                    "include cycle blocked: {} -> parent {}",
                    final_url,
                    parent_ent.id()
                ));
                continue;
            }

            let Some(xml) = load(&final_url, log)? else {
                continue;
            };

            let child_root = match parse_xml(world, &xml) {
                Ok(r) => r,
                Err(e) => {
                    log.push_error(format!("include: parse error {} -> {e}", final_url));
                    continue;
                }
            };
            set_node_base_url(world, child_root, &final_url);

            Hierarchy::add_child(world, parent_ent, child_root);
            collect_subtree_ids(world, child_root, &mut new_dirty);
        }
    }

    Ok(new_dirty)
}

// ─── Attribute updates ───────────────────────────────────────────────────────

pub fn apply_transform_updates(
    mut transform_updates: ResMut<crate::TransformUpdates>,
    mut world: ResMut<ElemenetWorld>,
    dom_data: Res<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut transform_only_dirty: ResMut<crate::TransformOnlyDirtyNodes>,
) {
    if transform_updates.is_empty() {
        return;
    }

    let entities = world.0.entities();
    let mut tr_storage = world.0.write_storage::<Transform2>();

    // Stream-write directo a Transform2: el orden del Vec preserva
    // last-wins para duplicados, sin HashMaps intermedios.
    // Cada nodo afectado se marca dirty + transform-only sin un Set extra:
    // `take_unique` deduplica al consumir, y el HashSet de transform-only
    // ignora reinserciones.
    let positions = transform_updates.positions.len();
    let rotations = transform_updates.rotations.len();
    let scales = transform_updates.scales.len();
    let total = positions + rotations + scales;
    dirty_nodes.0.reserve(total);
    transform_only_dirty.0.reserve(total);

    for (node_id, pos) in transform_updates.positions.drain(..) {
        let ent = entities.entity(node_id);
        if !entities.is_alive(ent) {
            continue;
        }
        if let Some(tr) = tr_storage.get_mut(ent) {
            tr.position.x = pos.x;
            tr.position.y = pos.y;
            tr.position.z = pos.z;
            if dom_data.nodes.contains_key(&node_id) {
                dirty_nodes.0.push(node_id);
                transform_only_dirty.0.insert(node_id);
            }
        }
    }

    for (node_id, rot) in transform_updates.rotations.drain(..) {
        let ent = entities.entity(node_id);
        if !entities.is_alive(ent) {
            continue;
        }
        if let Some(tr) = tr_storage.get_mut(ent) {
            tr.rotation.x = rot.x;
            tr.rotation.y = rot.y;
            tr.rotation.z = rot.z;
            if dom_data.nodes.contains_key(&node_id) {
                dirty_nodes.0.push(node_id);
                transform_only_dirty.0.insert(node_id);
            }
        }
    }

    for (node_id, scale) in transform_updates.scales.drain(..) {
        let ent = entities.entity(node_id);
        if !entities.is_alive(ent) {
            continue;
        }
        if let Some(tr) = tr_storage.get_mut(ent) {
            tr.scale.x = scale.x;
            tr.scale.y = scale.y;
            tr.scale.z = scale.z;
            if dom_data.nodes.contains_key(&node_id) {
                dirty_nodes.0.push(node_id);
                transform_only_dirty.0.insert(node_id);
            }
        }
    }
}


pub fn apply_attribute_updates(
    mut attribute_updates: ResMut<AttributeUpdates>,
    world: ResMut<ElemenetWorld>,
    dom_data: Res<crate::VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut js_snapshot_state: ResMut<crate::JsSnapshotState>,
    mut space_policies: ResMut<crate::permissions::SpacePolicies>,
    mut transform_only_dirty: ResMut<crate::TransformOnlyDirtyNodes>,
) {
    if attribute_updates.0.is_empty() {
        return;
    }

    js_snapshot_state.dirty = true;

    let entities = world.0.entities();
    let mut attrs_storage = world.0.write_storage::<Attrs>();
    let mut tr_storage = world.0.write_storage::<Transform2>();
    let mut model_storage = world.0.write_storage::<Model>();
    let mut include_storage = world.0.write_storage::<Include>();
    let mut script_storage = world.0.write_storage::<Script>();

    let (px, py, pz) = (
        TRANSFORM_POSITION[0],
        TRANSFORM_POSITION[1],
        TRANSFORM_POSITION[2],
    );
    let (rx, ry, rz) = (
        TRANSFORM_ROTATION[0],
        TRANSFORM_ROTATION[1],
        TRANSFORM_ROTATION[2],
    );
    let (sx, sy, sz) = (TRANSFORM_SCALE[0], TRANSFORM_SCALE[1], TRANSFORM_SCALE[2]);

    for (ent_id, key, val) in attribute_updates.drain_coalesced() {
        let ent = entities.entity(ent_id);
        if !entities.is_alive(ent) {
            continue;
        }
        let is_attached = dom_data.nodes.contains_key(&ent_id);
        transform_only_dirty.0.remove(&ent_id);


        if attrs_storage.get(ent).is_none() {
            let _ = attrs_storage.insert(ent, Attrs(HashMap::new()));
        }

        if let Some(a) = attrs_storage.get_mut(ent) {
            if val == ATTR_DELETE_SENTINEL {
                a.0.remove(&key);
                if let Some(tr) = tr_storage.get_mut(ent) {
                    match key.as_str() {
                        "x" => tr.position.x = 0.0,
                        "y" => tr.position.y = 0.0,
                        "z" => tr.position.z = 0.0,
                        "rx" => tr.rotation.x = 0.0,
                        "ry" => tr.rotation.y = 0.0,
                        "rz" => tr.rotation.z = 0.0,
                        "sx" => tr.scale.x = 1.0,
                        "sy" => tr.scale.y = 1.0,
                        "sz" => tr.scale.z = 1.0,
                        _ => {}
                    }
                }
            } else {
                a.0.insert(key.clone(), val.clone());
            }
        }

        if let Some(tr) = tr_storage.get_mut(ent) {
            let parse_f32 = || -> Option<f32> { val.trim().parse::<f32>().ok() };
            match key.as_str() {
                k if k == px || k == "x" => {
                    if let Some(f) = parse_f32() {
                        tr.position.x = f;
                    }
                }
                k if k == py || k == "y" => {
                    if let Some(f) = parse_f32() {
                        tr.position.y = f;
                    }
                }
                k if k == pz || k == "z" => {
                    if let Some(f) = parse_f32() {
                        tr.position.z = f;
                    }
                }
                k if k == rx || k == "rx" => {
                    if let Some(f) = parse_f32() {
                        tr.rotation.x = f;
                    }
                }
                k if k == ry || k == "ry" => {
                    if let Some(f) = parse_f32() {
                        tr.rotation.y = f;
                    }
                }
                k if k == rz || k == "rz" => {
                    if let Some(f) = parse_f32() {
                        tr.rotation.z = f;
                    }
                }
                "s" => {
                    if let Some(f) = parse_f32() {
                        tr.scale.x = f;
                        tr.scale.y = f;
                        tr.scale.z = f;
                    }
                }
                k if k == sx || k == "sx" => {
                    if let Some(f) = parse_f32() {
                        tr.scale.x = f;
                    }
                }
                k if k == sy || k == "sy" => {
                    if let Some(f) = parse_f32() {
                        tr.scale.y = f;
                    }
                }
                k if k == sz || k == "sz" => {
                    if let Some(f) = parse_f32() {
                        tr.scale.z = f;
                    }
                }
                _ => {}
            }
        }

        if key == "src" {
            if let Some(m) = model_storage.get_mut(ent) {
                m.src = (val != ATTR_DELETE_SENTINEL).then(|| val.clone());
            }
            if let Some(i) = include_storage.get_mut(ent) {
                i.src = (val != ATTR_DELETE_SENTINEL).then(|| val.clone());
            }
            if let Some(s) = script_storage.get_mut(ent) {
                s.src = (val != ATTR_DELETE_SENTINEL).then(|| val.clone());
            }
        }

        if is_attached {
            dirty_nodes.0.push(ent_id);
        }

        if is_attached && matches!(key.as_str(), "resources" | "system-space") {
            space_policies.dirty = true;
        }
    }
}

// ─── Delete ──────────────────────────────────────────────────────────────────

pub fn process_delete_requests(
    mut delete_requests: ResMut<DeleteRequests>,
    mut world: ResMut<ElemenetWorld>,
    mut entity_map: ResMut<EntityMap>,
    mut commands: Commands,
    mut log_panel: ResMut<LogPanel>,
    mut script_load_states: ResMut<ScriptLoadStates>,
    mut pending_model_loads: ResMut<PendingModelLoads>,
    mut model_load_states: ResMut<ModelLoadStates>,
    mut space_handle_tables: ResMut<crate::SpaceHandleTables>,
    mut dom_data: ResMut<VirtualDomData>,
    bevy_parents: Query<&Parent>,
    mut include_load_states: ResMut<crate::IncludeLoadStates>,
    mut js_snapshot_state: ResMut<crate::JsSnapshotState>,
    mut space_policies: ResMut<crate::permissions::SpacePolicies>,
) {
    for ent_id in delete_requests.0.drain(..) {
        let sp_ent = world.0.entities().entity(ent_id);
        if !world.0.entities().is_alive(sp_ent) {
            continue;
        }

        let deleted = remove_dom_subtree(
            &mut world.0,
            sp_ent,
            &mut entity_map,
            &mut commands,
            &mut dom_data,
            &mut include_load_states,
            &mut script_load_states,
            &mut pending_model_loads,
            &mut model_load_states,
            &mut space_handle_tables,
            &bevy_parents,
        );

        if deleted == 0 {
            continue;
        }

        log_panel.push_info(format!(
            "Entity subtree (ID={}) deleted ({} nodes).",
            ent_id, deleted
        ));
        js_snapshot_state.dirty = true;
        space_policies.dirty = true;
    }
}

// ─── Mark dirty ──────────────────────────────────────────────────────────────
//
// El componente `Dirty` ya no se inserta por frame: causaba archetype churn de
// O(N) cubos × 2 moves cada frame durante animaciones (3000 cubos→500 cubos a
// <60fps). La señal real de "este nodo necesita re-sync" es `dirty_nodes.0`,
// que ya garantizan `apply_transform_updates` y `apply_attribute_updates`.
// `Dirty` se conserva como marker en spawns (compat con tests) y para señalar
// "primer render pendiente" — no se toca en el hot-path por frame.
pub fn mark_dirty_system(mut dirty_nodes: ResMut<DirtyNodes>) {
    dirty_nodes.dedup_in_place();
}

// ─── Include loading ─────────────────────────────────────────────────────────

fn queue_include_load_if_needed(
    node_id: u32,
    attrs_storage: &ReadStorage<Attrs>,
    node: SpecEntity,
    current_url: &CurrentUrl,
    include_load_states: &mut crate::IncludeLoadStates,
    tokio_rt: &TokioRuntime,
    io_service: &IoService,
    log_panel: &mut LogPanel,
    specs_world: &SpecWorld,
) {
    let src = attrs_storage
        .get(node)
        .and_then(|a| a.0.get("src"))
        .cloned();
    let Some(src) = src else {
        return;
    };
    if src.is_empty() {
        return;
    }
    let Some(final_url) =
        resolve_node_relative_url(specs_world, node, &current_url.0, &src)
    else {
        log_panel.push_warn(format!("include: cannot resolve src='{src}'"));
        return;
    };
    if include_would_cycle(specs_world, node, &final_url, &current_url.0) {
        include_load_states.0.insert(
            node_id,
            crate::IncludeLoadState::Failed {
                url: final_url.clone(),
            },
        );
        log_panel.push_warn(format!(
            "Include cycle blocked: {final_url} (node {node_id})"
        ));
        return;
    }

    // Check if already loading/loaded for this URL
    if let Some(state) = include_load_states.0.get(&node_id) {
        match state {
            crate::IncludeLoadState::Loading { url } if *url == final_url => return,
            crate::IncludeLoadState::Loaded { url } if *url == final_url => return,
            _ => {} // URL changed or failed, re-request
        }
    }

    include_load_states.0.insert(
        node_id,
        crate::IncludeLoadState::Loading {
            url: final_url.clone(),
        },
    );
    request_include_load(&tokio_rt.0, io_service, node_id, final_url.clone());
    log_panel.push_info(format!("Include load queued: {final_url} (node {node_id})"));
}

pub fn commit_pending_includes_system(
    mut world: ResMut<ElemenetWorld>,
    mut pending_includes: ResMut<crate::PendingIncludes>,
    mut dom_data: ResMut<VirtualDomData>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut log_panel: ResMut<LogPanel>,
    mut include_load_states: ResMut<crate::IncludeLoadStates>,
    current_url: Res<CurrentUrl>,
    mut entity_map: ResMut<EntityMap>,
    mut commands: Commands,
    mut script_load_states: ResMut<ScriptLoadStates>,
    mut pending_model_loads: ResMut<PendingModelLoads>,
    mut model_load_states: ResMut<ModelLoadStates>,
    mut space_handle_tables: ResMut<crate::SpaceHandleTables>,
    mut js_snapshot_state: ResMut<crate::JsSnapshotState>,
    bevy_parents: Query<&Parent>,
    mut space_policies: ResMut<crate::permissions::SpacePolicies>,
) {
    let pending: Vec<_> = pending_includes.0.drain(..).collect();
    if pending.is_empty() {
        return;
    }

    for inc in pending {
        let parent_ent = world.0.entities().entity(inc.parent_node_id);
        if !world.0.entities().is_alive(parent_ent) {
            log_panel.push_warn(format!(
                "Include parent node {} no longer alive, skipping",
                inc.parent_node_id
            ));
            include_load_states.0.remove(&inc.parent_node_id);
            continue;
        }

        let is_current_request = matches!(
            include_load_states.0.get(&inc.parent_node_id),
            Some(crate::IncludeLoadState::Loading { url }) if *url == inc.url
        );
        if !is_current_request {
            log_panel.push_info(format!(
                "Dropping stale include result: {} (parent {})",
                inc.url, inc.parent_node_id
            ));
            continue;
        }

        let url = inc.url.clone();
        if include_would_cycle(&world.0, parent_ent, &url, &current_url.0) {
            log_panel.push_warn(format!(
                "Include cycle blocked during commit: {} -> parent {}",
                url, inc.parent_node_id
            ));
            include_load_states
                .0
                .insert(inc.parent_node_id, crate::IncludeLoadState::Failed { url });
            continue;
        }

        match parse_xml(&mut world.0, &inc.xml) {
            Ok(child_root) => {
                set_node_base_url(&mut world.0, child_root, &inc.url);

                let previous_children = {
                    let hierarchies = world.0.read_storage::<Hierarchy>();
                    hierarchies
                        .get(parent_ent)
                        .map(|hierarchy| hierarchy.children.clone())
                        .unwrap_or_default()
                };
                let mut replaced_nodes = 0usize;
                for previous_child in previous_children {
                    replaced_nodes += remove_dom_subtree(
                        &mut world.0,
                        previous_child,
                        &mut entity_map,
                        &mut commands,
                        &mut dom_data,
                        &mut include_load_states,
                        &mut script_load_states,
                        &mut pending_model_loads,
                        &mut model_load_states,
                        &mut space_handle_tables,
                        &bevy_parents,
                    );
                }

                Hierarchy::add_child(&mut world.0, parent_ent, child_root);
                let mut new_dirty = Vec::new();
                collect_subtree_ids(&world.0, child_root, &mut new_dirty);
                log_panel.push_info(format!(
                    "Include committed: {} ({} new nodes, {} replaced) -> parent {}",
                    inc.url,
                    new_dirty.len(),
                    replaced_nodes,
                    inc.parent_node_id,
                ));

                for &nid in &new_dirty {
                    let ent = world.0.entities().entity(nid);
                    dom_data.nodes.insert(nid, ent);
                }
                dirty_nodes.0.extend(new_dirty);
                js_snapshot_state.dirty = true;
                space_policies.dirty = true;

                include_load_states.0.insert(
                    inc.parent_node_id,
                    crate::IncludeLoadState::Loaded { url: inc.url },
                );
            }
            Err(e) => {
                log_panel.push_error(format!(
                    "Include parse error: {} (parent {}): {e}",
                    inc.url, inc.parent_node_id
                ));
                include_load_states.0.insert(
                    inc.parent_node_id,
                    crate::IncludeLoadState::Failed { url: inc.url },
                );
            }
        }
    }
}

// ─── DOM → Bevy sync ─────────────────────────────────────────────────────────

fn queue_script_load_if_needed(
    node: SpecEntity,
    current_url: &CurrentUrl,
    scripts_storage: &ReadStorage<Script>,
    script_load_states: &mut ScriptLoadStates,
    tokio_rt: &TokioRuntime,
    io_service: &IoService,
    log_panel: &mut LogPanel,
    specs_world: &SpecWorld,
    pending_scripts: &mut crate::PendingScripts,
) {
    let Some(script_comp) = scripts_storage.get(node) else {
        return;
    };

    // Inline script: no src, has inline code
    if script_comp.src.is_none() {
        if let Some(code) = script_comp.inline.as_ref() {
            let node_id = node.id();
            let inline_url = format!("inline://{}", node_id);

            // Only run once (same dedup as src-based scripts)
            if matches!(
                script_load_states.0.get(&node_id),
                Some(ScriptLoadState::Loaded { url }) if *url == inline_url
            ) {
                return;
            }

            if let Some(space_id) = crate::js::find_owner_space_id(specs_world, node) {
                pending_scripts
                    .0
                    .push((space_id, inline_url.clone(), code.clone()));
                script_load_states
                    .0
                    .insert(node_id, ScriptLoadState::Loaded { url: inline_url });
                log_panel.push_info(format!(
                    "[JS][space:{space_id}] Inline script queued (node {node_id})"
                ));
            }
        }
        return;
    }

    let src = script_comp.src.as_ref().unwrap();
    let Some(final_url) =
        resolve_node_relative_url(specs_world, node, &current_url.0, src)
    else {
        log_panel.push_error(format!(
            "Cannot resolve script src '{src}' against '{}'",
            current_url.0
        ));
        return;
    };

    let should_request = !matches!(
        script_load_states.0.get(&node.id()),
        Some(ScriptLoadState::Requested { url })
            | Some(ScriptLoadState::Loaded { url })
            | Some(ScriptLoadState::Failed { url, .. })
            if *url == final_url
    );

    if should_request {
        script_load_states.0.insert(
            node.id(),
            ScriptLoadState::Requested {
                url: final_url.clone(),
            },
        );
        request_script_load(&tokio_rt.0, io_service, node.id(), final_url.clone());
        log_panel.push_info(format!("Queued async script load: {final_url}"));
    }
}

fn queue_model_prepare_if_needed(
    node_id: u32,
    final_url: &str,
    pending_model_loads: &mut PendingModelLoads,
    model_load_states: &mut ModelLoadStates,
    tokio_rt: &TokioRuntime,
    io_service: &IoService,
    log_panel: &mut LogPanel,
) {
    let should_request = !matches!(
        model_load_states.0.get(&node_id),
        Some(ModelLoadState::Requested { url })
            | Some(ModelLoadState::Ready { url, .. })
            | Some(ModelLoadState::Failed { url, .. })
            if *url == final_url
    );

    if should_request {
        model_load_states.0.insert(
            node_id,
            ModelLoadState::Requested {
                url: final_url.to_string(),
            },
        );
        if pending_model_loads.enqueue(final_url, node_id) {
            request_model_prepare(&tokio_rt.0, io_service, final_url.to_string());
            log_panel.push_info(format!("Queued async model prepare: {final_url}"));
        }
    }
}

pub fn dom_sync_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    dom_data: Res<VirtualDomData>,
    shared_resources: Res<SharedResources>,
    mut entity_map: ResMut<EntityMap>,
    mut dirty_nodes: ResMut<DirtyNodes>,
    mut query: Query<(
        Entity,
        &mut Transform,
        Option<&mut Handle<StandardMaterial>>,
        Option<&Parent>,
    )>,
    mut visibility_query: Query<&mut Visibility>,
    asset_server: Res<AssetServer>,
    mut log_panel: ResMut<LogPanel>,
    mut perf_stats: ResMut<PerformanceStats>,
    mut async_dom: AsyncDomParams,
    mut text_render: TextRenderParams,
    mut meshes: ResMut<Assets<Mesh>>,
    mut skybox: crate::SkyboxParams,
) {
    let start_time = Instant::now();
    let dirty_node_ids = dirty_nodes.take_unique();
    if dirty_node_ids.is_empty() {
        return;
    }

    // Despawns from in-place replacement (model/text/skybox) are deferred to
    // the END of this system. Reason — engine_race step 7: queueing
    // `despawn_recursive` before sibling/descendant inserts in the same
    // command buffer fires B0003 on flush:
    //   "Could not insert a bundle ... because [entity] doesn't exist"
    // By queueing all despawns last, every queued insert applies onto an
    // entity that is still alive at that point in the queue.
    let mut deferred_despawns: Vec<(Entity, u32)> = Vec::new();

    let tokio_rt = &async_dom.tokio_rt;
    let io_service = &async_dom.io_service;
    let current_url = &async_dom.current_url;
    let script_load_states = &mut async_dom.script_load_states;
    let pending_model_loads = &mut async_dom.pending_model_loads;
    let model_load_states = &mut async_dom.model_load_states;
    let transform_only_dirty = &mut async_dom.transform_only_dirty;

    let tags = world.0.read_storage::<Tag>();
    let transforms = world.0.read_storage::<Transform2>();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let models = world.0.read_storage::<Model>();
    let scripts_storage = world.0.read_storage::<Script>();
    let attrs_storage = world.0.read_storage::<Attrs>();

    // log_panel.push_info(format!("dom_sync: processing {} dirty nodes...", dirty_nodes.0.len()));

    fn sync_parent_if_needed(
        commands: &mut Commands,
        parent_id: Option<u32>,
        entity_map: &EntityMap,
        bevy_ent: Entity,
        current_parent: Option<Entity>,
    ) {
        let expected_parent = parent_id.and_then(|pid| entity_map.0.get(&pid).copied());

        if current_parent == expected_parent {
            return;
        }

        if let Some(parent_ent) = expected_parent {
            commands.entity(bevy_ent).set_parent(parent_ent);
        } else {
            commands.entity(bevy_ent).remove_parent();
        }
    }

    for node_id in dirty_node_ids {
        let Some(node) = dom_data.nodes.get(&node_id) else {
            if DOM_SYNC_VERBOSE_LOGS {
                log_panel.push_warn(format!("dom_sync: skipping detached node id={}", node_id));
            }
            continue;
        };

        let tag = tags.get(*node).map(|t| t.0.clone()).unwrap_or_default();
        let hierarchy = hierarchies.get(*node);
        let parent_id = hierarchy.and_then(|h| h.parent);
        let transform_only = transform_only_dirty.0.contains(&node_id);

        let mut transform_b = Transform::default();
        if let Some(tr2) = transforms.get(*node) {
            apply_transform(tr2, &mut transform_b);
        }

        if let Some(&bevy_ent) = entity_map.0.get(&node_id) {
            // --- Update existing entity ---
            let current_parent = {
                match query.get_mut(bevy_ent) {
                    Ok((_, _, _, parent)) => parent.map(|p| p.get()),
                    Err(_) => None,
                }
            };

            sync_parent_if_needed(
                &mut commands,
                parent_id,
                &entity_map,
                bevy_ent,
                current_parent,
            );

            //
            // REGLA: la señal de "este nodo necesita re-sync" es estar en `dirty_nodes.0`
            // (lo garantizan `apply_transform_updates` y `apply_attribute_updates`).
            // Los attrs deben leerse desde `attrs_storage` directamente.
            //
            // Patrón correcto para agregar soporte a un nuevo tag con atributos mutables:
            //   1. Leer los attrs desde `attrs_storage.get(*node)` directamente.
            //   2. Aplicar el cambio al asset/componente Bevy sin guard adicional.
            //   3. Agregar `continue` para no caer en el default de transform-only.
            if tag == "model" {
                let resolved_asset_path = models
                    .get(*node)
                    .and_then(|model_data| model_data.src.as_ref())
                    .and_then(|original_src| {
                        resolve_node_relative_url(&world.0, *node, &current_url.0, original_src)
                    })
                    .and_then(|final_url| {
                        queue_model_prepare_if_needed(
                            node_id,
                            &final_url,
                            pending_model_loads,
                            model_load_states,
                            tokio_rt,
                            io_service,
                            &mut log_panel,
                        );

                        match model_load_states.0.get(&node_id) {
                            Some(ModelLoadState::Ready { url, asset_path })
                                if *url == final_url =>
                            {
                                Some(asset_path.clone())
                            }
                            Some(ModelLoadState::Failed { url, error }) if *url == final_url => {
                                log_panel.push_warn(format!(
                                    "Model load failed for {final_url}: {error}"
                                ));
                                None
                            }
                            _ => None,
                        }
                    });

                if let Some(asset_path) = resolved_asset_path {
                    deferred_despawns.push((bevy_ent, node_id));
                    entity_map.0.remove(&node_id);

                    let new_ent = spawn_model_entity(
                        &mut commands,
                        &asset_server,
                        transform_b,
                        Some(asset_path.as_str()),
                        &shared_resources,
                    );
                    set_parent(&mut commands, new_ent, parent_id, &entity_map);
                    entity_map.0.insert(node_id, new_ent);
                } else if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                }
                // IMPORTANTE:
                // un <model> puede haber recibido position/scale antes de que el asset
                // termine de cargar. Si queda pegado en transform_only_dirty, cuando
                // el modelo quede Ready el fast-path de transform-only se comería para
                // siempre el reemplazo del placeholder por la escena real.
                transform_only_dirty.0.remove(&node_id);
                continue;
            }

            if tag == "skybox" {
                if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                }
                transform_only_dirty.0.remove(&node_id);
                continue;
            }

            if transform_only {
                if tag == "text" {
                    let empty_map = HashMap::new();
                    let attrs_map = attrs_storage.get(*node).map(|a| &a.0).unwrap_or(&empty_map);
                    let (text_value, text_size, _) = parse_text_attrs(attrs_map);
                    let text_transform = build_text_transform(transform_b, &text_value, text_size);
                    if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                        *t = text_transform;
                    }
                } else {
                    if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                        *t = transform_b;
                    }
                }

                transform_only_dirty.0.remove(&node_id);
                continue;
            }

            if tag == "text" {
                let empty_map = HashMap::new();
                let attrs_map = attrs_storage.get(*node).map(|a| &a.0).unwrap_or(&empty_map);
                let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                let text_material = get_or_create_text_material(
                    &mut text_render.text_material_cache,
                    &mut text_render.materials,
                    &mut text_render.images,
                    &text_value,
                    text_size,
                    text_color,
                );
                let text_transform = build_text_transform(transform_b, &text_value, text_size);

                let mut updated_in_place = false;
                if let Ok((_, mut t, maybe_material, _)) = query.get_mut(bevy_ent) {
                    *t = text_transform;
                    if let Some(mut material_handle) = maybe_material {
                        *material_handle = text_material.clone();
                        updated_in_place = true;
                    }
                }
                if updated_in_place {
                    continue;
                }

                deferred_despawns.push((bevy_ent, node_id));
                entity_map.0.remove(&node_id);
                let new_ent = commands
                    .spawn((
                        PbrBundle {
                            mesh: shared_resources.plane_mesh.clone(),
                            material: text_material,
                            transform: text_transform,
                            ..Default::default()
                        },
                        Dirty,
                    ))
                    .id();
                set_parent(&mut commands, new_ent, parent_id, &entity_map);
                entity_map.0.insert(node_id, new_ent);
                continue;
            }

            if tag == "script" {
                queue_script_load_if_needed(
                    *node,
                    current_url,
                    &scripts_storage,
                    script_load_states,
                    tokio_rt,
                    io_service,
                    &mut log_panel,
                    &world.0,
                    &mut async_dom.pending_scripts,
                );
                if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                }
                continue;
            }

            if tag == "posezone" {
                commands
                    .entity(bevy_ent)
                    .insert((crate::touch::PoseZone(node_id), crate::touch::HitShape::Box));
                if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                }
                if let Ok(mut visibility) = visibility_query.get_mut(bevy_ent) {
                    *visibility = if node_visible(&attrs_storage, *node) {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
                continue;
            }


            if tag == "space" || tag == "include" || is_structural_tag(&tag) {
                if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                }
                if let Ok(mut visibility) = visibility_query.get_mut(bevy_ent) {
                    *visibility = if node_visible(&attrs_storage, *node) {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                }
                // Trigger include load if src changed or not yet loaded
                if tag == "include" {
                    queue_include_load_if_needed(
                        node_id,
                        &attrs_storage,
                        *node,
                        &current_url,
                        &mut async_dom.include_load_states,
                        tokio_rt,
                        io_service,
                        &mut log_panel,
                        &world.0,
                    );
                }
                continue;
            }

            if tag == "box" || tag == "sphere" || tag == "plane" || tag == "cylinder" {
                let hit_shape = match tag.as_str() {
                    "box" => crate::touch::HitShape::Box,
                    "plane" => crate::touch::HitShape::Plane,
                    _ => crate::touch::HitShape::Sphere,
                };
                let touchable = node_touchable(&attrs_storage, *node);
                if touchable {
                    commands
                        .entity(bevy_ent)
                        .insert((crate::touch::Toqueable(node_id), hit_shape));
                } else {
                    commands.entity(bevy_ent).remove::<crate::touch::Toqueable>();
                    commands.entity(bevy_ent).remove::<crate::touch::HitShape>();
                }
                let color = primitive_color(&attrs_storage, *node);
                let double_sided = tag == "plane";
                let material = get_or_create_primitive_material(
                    &mut text_render.primitive_material_cache,
                    &mut text_render.materials,
                    color,
                    double_sided,
                );

                // Regenerate mesh if border-radius changed (box or plane)
                let attrs_opt = attrs_storage.get(*node);
                let border_radius: Option<f32> = attrs_opt
                    .and_then(|a| a.0.get("border-radius"))
                    .and_then(|v| v.parse().ok());
                let new_mesh = match (tag.as_str(), border_radius) {
                    ("box", Some(r)) => {
                        Some(meshes.add(crate::utils::shapes::create_rounded_cube(r, 6)))
                    }
                    ("box", None) => Some(shared_resources.cube_mesh.clone()),
                    ("plane", Some(r)) => {
                        Some(meshes.add(crate::utils::shapes::create_rounded_plane(r, 6)))
                    }
                    ("plane", None) => Some(shared_resources.plane_mesh.clone()),
                    _ => None,
                };
                if let Some(mesh_handle) = new_mesh {
                    commands.entity(bevy_ent).insert(mesh_handle);
                }

                if let Ok((_, mut t, maybe_material, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                    if let Some(mut material_handle) = maybe_material {
                        *material_handle = material;
                    }
                }
                continue;
            }

            // Default: nodo en `dirty_nodes.0` ⇒ transform desactualizado, escribir.
            if let Ok((_, mut t, _, _)) = query.get_mut(bevy_ent) {
                *t = transform_b;
            }
        } else {
            // --- Create new entity ---
            if DOM_SYNC_VERBOSE_LOGS {
                log_panel.push_info("  Creating new entity...");
            }

            let new_ent = match tag.as_str() {
                "model" => {
                    let resolved_asset_path = models
                        .get(*node)
                        .and_then(|model_data| model_data.src.as_ref())
                        .and_then(|original_src| resolve_remote_path(&current_url.0, original_src))
                        .and_then(|final_url| {
                            queue_model_prepare_if_needed(
                                node_id,
                                &final_url,
                                pending_model_loads,
                                model_load_states,
                                tokio_rt,
                                io_service,
                                &mut log_panel,
                            );

                            match model_load_states.0.get(&node_id) {
                                Some(ModelLoadState::Ready { url, asset_path })
                                    if *url == final_url =>
                                {
                                    Some(asset_path.clone())
                                }
                                Some(ModelLoadState::Failed { url, error })
                                    if *url == final_url =>
                                {
                                    log_panel.push_warn(format!(
                                        "Model load failed for {final_url}: {error}"
                                    ));
                                    None
                                }
                                _ => None,
                            }
                        });

                    spawn_model_entity(
                        &mut commands,
                        &asset_server,
                        transform_b,
                        resolved_asset_path.as_deref(),
                        &shared_resources,
                    )
                }
                "script" => {
                    queue_script_load_if_needed(
                        *node,
                        current_url,
                        &scripts_storage,
                        script_load_states,
                        tokio_rt,
                        io_service,
                        &mut log_panel,
                        &world.0,
                        &mut async_dom.pending_scripts,
                    );
                    commands
                        .spawn((
                            SpatialBundle {
                                transform: transform_b,
                                ..Default::default()
                            },
                            Dirty,
                        ))
                        .id()
                }
                "space" | "include" => {
                    if tag == "include" {
                        queue_include_load_if_needed(
                            node_id,
                            &attrs_storage,
                            *node,
                            &current_url,
                            &mut async_dom.include_load_states,
                            tokio_rt,
                            io_service,
                            &mut log_panel,
                            &world.0,
                        );
                    }
                    commands
                        .spawn((
                            SpatialBundle {
                                transform: transform_b,
                                visibility: if node_visible(&attrs_storage, *node) {
                                    Visibility::Visible
                                } else {
                                    Visibility::Hidden
                                },
                                ..Default::default()
                            },
                            Dirty,
                        ))
                        .id()
                }
                "posezone" => commands
                    .spawn((
                        SpatialBundle {
                            transform: transform_b,
                            visibility: if node_visible(&attrs_storage, *node) {
                                Visibility::Visible
                            } else {
                                Visibility::Hidden
                            },
                            ..Default::default()
                        },
                        Dirty,
                        crate::touch::PoseZone(node_id),
                        crate::touch::HitShape::Box,
                    ))
                    .id(),
                _other if is_structural_tag(&tag) => commands
                    .spawn((
                        SpatialBundle {
                            transform: transform_b,
                            visibility: if node_visible(&attrs_storage, *node) {
                                Visibility::Visible
                            } else {
                                Visibility::Hidden
                            },
                            ..Default::default()
                        },
                        Dirty,
                    ))
                    .id(),
                "skybox" => {
                    let has_perm = crate::js::find_owner_space_id(&world.0, *node)
                        .and_then(|sid| skybox.space_policies.by_space.get(&sid))
                        .map(|p| p.effective_caps.contains(crate::permissions::CapabilityBits::SKYBOX))
                        .unwrap_or(false);

                    if !has_perm {
                        log_panel.push_warn(format!(
                            "Skybox blocked: space lacks 'skybox' permission (node {node_id})"
                        ));
                        commands.spawn((SpatialBundle { transform: transform_b, ..Default::default() }, Dirty)).id()
                    } else {
                        let src = attrs_storage
                            .get(*node)
                            .and_then(|a| a.0.get("src"))
                            .cloned()
                            .unwrap_or_default();

                        // Despawn previous skybox if different node
                        if let Some((old_node_id, old_ent)) = skybox.skybox_entity.0.take() {
                            if old_node_id != node_id {
                                deferred_despawns.push((old_ent, old_node_id));
                                entity_map.0.remove(&old_node_id);
                            } else {
                                skybox.skybox_entity.0 = Some((old_node_id, old_ent));
                            }
                        }

                        let skybox_root = commands
                            .spawn((SpatialBundle { transform: transform_b, ..Default::default() }, Dirty))
                            .id();

                        spawn_skybox_faces(
                            &src,
                            &world.0,
                            *node,
                            &current_url.0,
                            &tokio_rt.0,
                            &asset_server,
                            &mut text_render.materials,
                            &shared_resources,
                            &mut commands,
                            skybox_root,
                            &mut log_panel,
                        );

                        skybox.skybox_entity.0 = Some((node_id, skybox_root));
                        skybox_root
                    }
                }
                "box" => {
                    let attrs_opt = attrs_storage.get(*node);
                    let color = primitive_color(&attrs_storage, *node);
                    let touchable = node_touchable(&attrs_storage, *node);
                    let border_radius: Option<f32> = attrs_opt
                        .and_then(|a| a.0.get("border-radius"))
                        .and_then(|v| v.parse().ok());

                    let mesh = if let Some(radius) = border_radius {
                        meshes.add(crate::utils::shapes::create_rounded_cube(radius, 6))
                    } else {
                        shared_resources.cube_mesh.clone()
                    };
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        &mut text_render.primitive_material_cache,
                        mesh,
                        color,
                        transform_b,
                        false,
                        if touchable { Some(node_id) } else { None },
                        if touchable { Some(crate::touch::HitShape::Box) } else { None },
                    )
                }
                "sphere" => {
                    let touchable = node_touchable(&attrs_storage, *node);
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        &mut text_render.primitive_material_cache,
                        shared_resources.sphere_mesh.clone(),
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        false,
                        if touchable { Some(node_id) } else { None },
                        if touchable { Some(crate::touch::HitShape::Sphere) } else { None },
                    )
                },
                "plane" => {
                    let attrs_opt = attrs_storage.get(*node);
                    let touchable = node_touchable(&attrs_storage, *node);
                    let border_radius: Option<f32> = attrs_opt
                        .and_then(|a| a.0.get("border-radius"))
                        .and_then(|v| v.parse().ok());
                    let mesh = if let Some(radius) = border_radius {
                        meshes.add(crate::utils::shapes::create_rounded_plane(radius, 6))
                    } else {
                        shared_resources.plane_mesh.clone()
                    };
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        &mut text_render.primitive_material_cache,
                        mesh,
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        true,
                        if touchable { Some(node_id) } else { None },
                        if touchable { Some(crate::touch::HitShape::Plane) } else { None },
                    )
                }
                "cylinder" => {
                    let touchable = node_touchable(&attrs_storage, *node);
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        &mut text_render.primitive_material_cache,
                        shared_resources.cylinder_mesh.clone(),
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        false,
                        if touchable { Some(node_id) } else { None },
                        if touchable { Some(crate::touch::HitShape::Sphere) } else { None },
                    )
                },
                "text" => {
                    let empty_map = HashMap::new();
                    let attrs_map = attrs_storage.get(*node).map(|a| &a.0).unwrap_or(&empty_map);
                    let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                    let text_material = get_or_create_text_material(
                        &mut text_render.text_material_cache,
                        &mut text_render.materials,
                        &mut text_render.images,
                        &text_value,
                        text_size,
                        text_color,
                    );
                    let text_transform = build_text_transform(transform_b, &text_value, text_size);
                    commands
                        .spawn((
                            PbrBundle {
                                mesh: shared_resources.plane_mesh.clone(),
                                material: text_material,
                                transform: text_transform,
                                ..Default::default()
                            },
                            Dirty,
                        ))
                        .id()
                }
                _other => commands
                    .spawn((
                        PbrBundle {
                            mesh: shared_resources.cube_mesh.clone(),
                            material: shared_resources.default_material.clone(),
                            transform: transform_b,
                            ..Default::default()
                        },
                        Dirty,
                    ))
                    .id(),
            };

            set_parent(&mut commands, new_ent, parent_id, &entity_map);
            entity_map.0.insert(node_id, new_ent);
            
            // Si este nodo venía marcado como transform-only desde antes de tener
            // entidad Bevy, la marca debe limpiarse ahora. Si no, un dirty futuro
            // (por ejemplo cuando un model pasa a Ready) entrará al fast-path y
            // salteará lógica importante.
            transform_only_dirty.0.remove(&node_id);
        }
    }

    // Drain deferred despawns LAST so that every queued insert/component op
    // earlier in this system targets a still-alive entity. For each despawn
    // target we also walk its SPECS subtree, drop any dangling descendant
    // entries from `entity_map` (the Bevy cascade kills those entities), and
    // re-queue them as dirty so dom_sync recreates fresh Bevy entities for
    // them next frame. See engine_race step5/step7 tests for the rationale.
    if !deferred_despawns.is_empty() {
        // Drop the storage borrows we are still holding on world.0 before
        // walking the subtree (we re-borrow inside).
        drop(hierarchies);
        drop(tags);
        drop(transforms);
        drop(models);
        drop(scripts_storage);
        drop(attrs_storage);

        let entities = world.0.entities();
        let hier = world.0.read_storage::<Hierarchy>();

        let mut next_dirty: Vec<u32> = Vec::new();

        for (old_ent, root_node_id) in &deferred_despawns {
            let root_spec = entities.entity(*root_node_id);
            if !entities.is_alive(root_spec) {
                continue;
            }
            // Collect descendants only (skip the root, which is being respawned).
            let mut stack: Vec<SpecEntity> = Vec::new();
            if let Some(h) = hier.get(root_spec) {
                for &c in &h.children {
                    if entities.is_alive(c) {
                        stack.push(c);
                    }
                }
            }
            while let Some(ent) = stack.pop() {
                let did = ent.id();
                if entity_map.0.remove(&did).is_some() {
                    next_dirty.push(did);
                }
                if let Some(h) = hier.get(ent) {
                    for &c in &h.children {
                        if entities.is_alive(c) {
                            stack.push(c);
                        }
                    }
                }
            }

            commands.entity(*old_ent).despawn_recursive();
        }

        if !next_dirty.is_empty() {
            dirty_nodes.0.extend(next_dirty);
        }
    }

    perf_stats.dom_sync_ms = start_time.elapsed().as_secs_f32() * 1000.0;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn set_parent(
    commands: &mut Commands,
    new_ent: Entity,
    parent_id: Option<u32>,
    entity_map: &EntityMap,
) {
    if let Some(pid) = parent_id {
        if let Some(&parent_bevy_ent) = entity_map.0.get(&pid) {
            commands.entity(new_ent).set_parent(parent_bevy_ent);
            return;
        }
    }
    commands.entity(new_ent).remove_parent();
}

fn spawn_model_entity(
    commands: &mut Commands,
    asset_server: &AssetServer,
    transform_b: Transform,
    asset_path: Option<&str>,
    _shared_resources: &SharedResources,
) -> Entity {
    if let Some(asset_path) = asset_path {
        let scene_handle = if asset_path.ends_with(".gltf") || asset_path.ends_with(".glb") {
            asset_server.load(format!("{asset_path}#Scene0"))
        } else {
            asset_server.load(asset_path.to_string())
        };
        return commands
            .spawn((
                SceneBundle {
                    scene: scene_handle,
                    transform: transform_b,
                    ..Default::default()
                },
                Dirty,
            ))
            .id();
    }

    commands
        .spawn((
            SpatialBundle {
                transform: transform_b,
                visibility: Visibility::Inherited,
                ..Default::default()
            },
            Dirty,
        ))
        .id()
}

fn spawn_skybox_faces(
    src_pattern: &str,
    specs_world: &specs::World,
    node: specs::Entity,
    current_url: &str,
    rt: &tokio::runtime::Runtime,
    asset_server: &AssetServer,
    materials: &mut Assets<StandardMaterial>,
    shared_resources: &SharedResources,
    commands: &mut Commands,
    parent: Entity,
    log_panel: &mut LogPanel,
) {
    use std::f32::consts::{FRAC_PI_2, PI};

    const SKY_SIZE: f32 = 500.0;

    let faces: &[(&str, [f32; 3], bevy::math::Quat)] = &[
        ("pz", [0.0, 0.0, -SKY_SIZE], bevy::math::Quat::IDENTITY),
        ("nz", [0.0, 0.0,  SKY_SIZE], bevy::math::Quat::from_rotation_y(PI)),
        ("nx", [-SKY_SIZE, 0.0, 0.0], bevy::math::Quat::from_rotation_y(FRAC_PI_2)),
        ("px", [ SKY_SIZE, 0.0, 0.0], bevy::math::Quat::from_rotation_y(-FRAC_PI_2)),
        ("py", [0.0,  SKY_SIZE, 0.0], bevy::math::Quat::from_rotation_x(FRAC_PI_2)),
        ("ny", [0.0, -SKY_SIZE, 0.0], bevy::math::Quat::from_rotation_x(-FRAC_PI_2)),
    ];

    for (face, pos, rot) in faces {
        let face_src = src_pattern.replace("$1", face);
        let resolved_url = resolve_node_relative_url(specs_world, node, current_url, &face_src)
            .unwrap_or_else(|| face_src.clone());

        let image_handle = load_skybox_face_image(&resolved_url, rt, asset_server, log_panel);

        let mat = materials.add(StandardMaterial {
            base_color_texture: Some(image_handle),
            unlit: true,
            cull_mode: None,
            ..Default::default()
        });

        let face_ent = commands
            .spawn(PbrBundle {
                mesh: shared_resources.plane_mesh.clone(),
                material: mat,
                transform: Transform {
                    translation: Vec3::from_array(*pos),
                    rotation: *rot,
                    scale: Vec3::splat(SKY_SIZE * 2.0),
                },
                ..Default::default()
            })
            .id();

        commands.entity(face_ent).set_parent(parent);
    }
}

fn load_skybox_face_image(
    url: &str,
    rt: &tokio::runtime::Runtime,
    asset_server: &AssetServer,
    log_panel: &mut LogPanel,
) -> Handle<Image> {
    let (assets_dir, cache_dir) = crate::utils::folder::resolve_assets_and_cache_dirs();
    let _ = std::fs::create_dir_all(&cache_dir);
    let filename = crate::render::encode_url_to_filename(url);
    let local_path = cache_dir.join(&filename);

    if !local_path.exists() {
        if url.starts_with("http://") || url.starts_with("https://") {
            match rt.block_on(crate::render::load_bytes_from_url(url)) {
                Ok(bytes) => {
                    if let Err(e) = std::fs::write(&local_path, &bytes) {
                        log_panel.push_error(format!("Skybox face write '{url}': {e}"));
                        return Handle::default();
                    }
                }
                Err(e) => {
                    log_panel.push_error(format!("Skybox face download '{url}': {e}"));
                    return Handle::default();
                }
            }
        } else {
            let from = std::path::PathBuf::from(url);
            if let Err(e) = std::fs::copy(&from, &local_path) {
                log_panel.push_error(format!("Skybox face copy '{url}': {e}"));
                return Handle::default();
            }
        }
    }

    match crate::utils::folder::to_assets_relative(&local_path, &assets_dir) {
        Some(rel) => asset_server.load(rel),
        None => {
            log_panel.push_error(format!("Skybox face outside assets dir: {}", local_path.display()));
            Handle::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::App;
    use specs::{Join, WorldExt};
    use virtual_dom::{
        dom::{
            element::{build_world, Hierarchy, Tag},
            hsml::Model,
        },
        parse_xml,
    };
    use crate::{IncludeLoadState, IncludeLoadStates, JsSnapshotState, PendingInclude, PendingIncludes, SpaceHandleTables};
    use bevy::ecs::system::RunSystemOnce;

    fn collect_all_nodes(world: &SpecWorld, root: SpecEntity) -> HashMap<u32, SpecEntity> {
        let mut ids = Vec::new();
        collect_subtree_ids(world, root, &mut ids);
        let entities = world.entities();
        ids.into_iter()
            .map(|id| (id, entities.entity(id)))
            .collect::<HashMap<_, _>>()
    }

    #[test]
    fn runtime_include_uses_include_document_base_for_model_src() {
        let mut specs_world = build_world();

        let root = parse_xml(
            &mut specs_world,
            r#"
            <hsml>
              <space>
                <include src="http://localhost:2052/demo/index.hsml" />
              </space>
            </hsml>
            "#,
        )
        .expect("root parse failed");

        set_node_base_url(&mut specs_world, root, "luna://root");

        let outer_include = {
            let hier = specs_world.read_storage::<Hierarchy>();
            let space = hier.get(root).unwrap().children[0];
            hier.get(space).unwrap().children[0]
        };

        let mut app = App::new();
        app.insert_resource(ElemenetWorld(specs_world));
        app.insert_resource(VirtualDomData {
            nodes: collect_all_nodes(&app.world().resource::<ElemenetWorld>().0, root),
        });
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(LogPanel::default());
        app.insert_resource(CurrentUrl("luna://root".to_string()));
        app.insert_resource(EntityMap::default());
        app.insert_resource(ScriptLoadStates::default());
        app.insert_resource(PendingModelLoads::default());
        app.insert_resource(ModelLoadStates::default());
        app.insert_resource(SpaceHandleTables::default());
        app.insert_resource(JsSnapshotState::default());
        app.insert_resource(crate::permissions::SpacePolicies::default());

        let mut include_states = IncludeLoadStates::default();
        include_states.0.insert(
            outer_include.id(),
            IncludeLoadState::Loading {
                url: "http://localhost:2052/demo/index.hsml".to_string(),
            },
        );
        app.insert_resource(include_states);

        app.insert_resource(PendingIncludes(vec![PendingInclude {
            parent_node_id: outer_include.id(),
            url: "http://localhost:2052/demo/index.hsml".to_string(),
            xml: r#"
                <hsml>
                  <space>
                    <model src="models/tree.glb" />
                  </space>
                </hsml>
            "#
            .to_string(),
        }]));

        app.add_systems(Update, commit_pending_includes_system);
        app.update();

        let specs_world = &app.world().resource::<ElemenetWorld>().0;
        let entities = specs_world.entities();
        let tags = specs_world.read_storage::<Tag>();
        let models = specs_world.read_storage::<Model>();

        let model_ent = (&entities, &tags, &models)
            .join()
            .find_map(|(ent, tag, _)| (tag.0 == "model").then_some(ent))
            .expect("model node from runtime include not found");

        let resolved = resolve_node_relative_url(
            specs_world,
            model_ent,
            "luna://root",
            "models/tree.glb",
        )
        .expect("model url should resolve");

        assert_eq!(
            resolved,
            "http://localhost:2052/demo/models/tree.glb"
        );
    }

    // ─── Engine race repro: stepping-stone tests ─────────────────────────────
    //
    // The production crash is:
    //   "Could not insert a bundle (of type (MaterialMeshBundle<StandardMaterial>, Dirty))
    //    for entity Entity { index: 119, generation: 8 } because it doesn't exist"
    //
    // panic source: bevy_ecs/src/system/commands/mod.rs (B0003).
    //
    // The Entity is at generation 8 → the slot was recycled many times. So the
    // panic shape is: a queued `Commands::entity(ent).insert(bundle)` whose
    // `ent` is dead by the time commands flush.
    //
    // These tests narrow down the failure layer by layer, NOT by speculating.

    /// Step 1: confirm the exact panic shape. `commands.entity(dead).insert()`
    /// panics on flush. This is the reference behavior we have to defend
    /// against anywhere we plumb an Entity through `entity_map`.
    #[test]
    fn engine_race_step1_insert_into_dead_entity_panics() {
        use bevy::prelude::*;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut app = App::new();
        let dead = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(dead);

        #[derive(Resource)]
        struct DeadEnt(Entity);
        app.insert_resource(DeadEnt(dead));

        fn sys(dead: Res<DeadEnt>, mut commands: Commands) {
            commands.entity(dead.0).insert(GlobalTransform::default());
        }
        app.add_systems(Update, sys);

        let result = catch_unwind(AssertUnwindSafe(|| app.update()));
        assert!(
            result.is_err(),
            "B0003 sanity: insert into despawned entity must panic on command flush"
        );
    }

    /// Step 2: probe what Bevy 0.14 actually accepts on a stale handle.
    ///
    /// Finding: `commands.entity(stale)` ITSELF panics in 0.14 with
    ///   "Attempting to create an EntityCommands for entity ...,
    ///    which doesn't exist."
    /// — even before `.try_insert(...)` is queued. So `try_insert` is NOT a
    /// sufficient guard against stale-handle inputs (it only protects against
    /// post-queue despawn). The mark_dirty_system comment at dom.rs:1003
    /// implies otherwise — that comment is misleading for the stale-handle
    /// scenario; it only covers the queue-order scenario.
    ///
    /// The correct guard for stale handles is `commands.get_entity(ent)` →
    /// `Option<EntityCommands>`. This test pins that behavior so we don't
    /// regress to the wrong fix.
    #[test]
    fn engine_race_step2_get_entity_is_the_real_guard() {
        use bevy::prelude::*;

        let mut app = App::new();
        let dead = app.world_mut().spawn_empty().id();
        app.world_mut().despawn(dead);

        #[derive(Resource)]
        struct DeadEnt(Entity);
        app.insert_resource(DeadEnt(dead));

        fn sys(dead: Res<DeadEnt>, mut commands: Commands) {
            // get_entity → None for despawned/stale handles. No panic.
            if let Some(mut ec) = commands.get_entity(dead.0) {
                ec.insert(GlobalTransform::default());
            }
        }
        app.add_systems(Update, sys);

        app.update();
    }

    /// Step 3: prove the recycle pattern. `commands.spawn(bundle)` reserves
    /// an entity index. If despawn_recursive of a parent that this entity was
    /// just `set_parent`-ed under fires before the spawn-bundle apply, the
    /// reserved entity is killed by the cascade and the bundle insert panics
    /// with the SAME B0003 shape as production.
    ///
    /// This is the suspected trigger when dom_sync_system processes dirty
    /// nodes in an order where a parent's text/model replacement (which
    /// despawn_recursive's the OLD parent) runs after a child's spawn that
    /// linked itself to the OLD parent.
    #[test]
    fn engine_race_step3_spawn_then_parent_then_recursive_despawn() {
        use bevy::prelude::*;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut app = App::new();
        let parent_old = app.world_mut().spawn(SpatialBundle::default()).id();

        #[derive(Resource)]
        struct ParentOld(Entity);
        app.insert_resource(ParentOld(parent_old));

        // Order matches the "child processed first, then parent" iteration
        // case in dom_sync_system. The child reserves an entity, set_parents
        // under the OLD parent, then the OLD parent gets despawn_recursive'd.
        fn sys(parent_old: Res<ParentOld>, mut commands: Commands) {
            let child = commands
                .spawn((SpatialBundle::default(), super::Dirty))
                .id();
            commands.entity(child).set_parent(parent_old.0);
            commands.entity(parent_old.0).despawn_recursive();
        }
        app.add_systems(Update, sys);

        let result = catch_unwind(AssertUnwindSafe(|| app.update()));
        // Document whichever way Bevy 0.14 resolves the order — the test is
        // primarily a probe. If this panics, it confirms the production
        // crash trigger; if not, the bug is elsewhere and we move on.
        if result.is_err() {
            eprintln!(
                "engine_race_step3: spawn+set_parent+despawn_recursive panicked → \
                 confirms dom_sync iteration order can trigger B0003"
            );
        }
    }

    /// Step 4: same pattern but with `try_insert` on the bundle. Demonstrates
    /// whether switching to try_insert would defuse step 3.
    #[test]
    fn engine_race_step4_try_insert_under_recursive_despawn() {
        use bevy::prelude::*;

        let mut app = App::new();
        let parent_old = app.world_mut().spawn(SpatialBundle::default()).id();

        #[derive(Resource)]
        struct ParentOld(Entity);
        app.insert_resource(ParentOld(parent_old));

        fn sys(parent_old: Res<ParentOld>, mut commands: Commands) {
            // Reserve, then immediately mutate via try_insert paths only.
            let child = commands.spawn_empty().id();
            commands
                .entity(child)
                .try_insert((SpatialBundle::default(), super::Dirty));
            commands.entity(child).set_parent(parent_old.0);
            commands.entity(parent_old.0).despawn_recursive();
        }
        app.add_systems(Update, sys);

        // Should not panic regardless of cascade ordering.
        app.update();
    }

    /// Step 5: model/text replacement path in dom_sync_system at lines
    /// 1456 and 1548 does `commands.entity(bevy_ent).despawn_recursive()` and
    /// then `entity_map.0.remove(&node_id)` — but this only clears the
    /// PARENT's entry. Any descendant (grand-children) still mapped in
    /// `entity_map` becomes a dangling pointer to a dead Entity.
    ///
    /// On the next dom_sync tick, if a descendant is dirty, the unguarded
    /// `commands.entity(bevy_ent).insert(...)` calls at:
    ///   - dom.rs:1590 (posezone)
    ///   - dom.rs:1647 (box/sphere/plane/cylinder Toqueable+HitShape)
    ///   - dom.rs:1674 (mesh handle)
    /// will panic with B0003. This test reproduces the exact pattern.
    #[test]
    fn engine_race_step5_descendant_in_entity_map_after_parent_despawn_recursive() {
        use bevy::prelude::*;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut app = App::new();
        let parent = app.world_mut().spawn(SpatialBundle::default()).id();
        let child = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(parent).add_child(child);

        // Simulate what dom_sync line 1456/1548 does: despawn parent
        // recursively but only remove the parent from the (stand-in)
        // entity_map. Child's entity_map entry is now dangling.
        #[derive(Resource)]
        struct EntityMapStub {
            child: Entity,
        }
        app.insert_resource(EntityMapStub { child });

        fn despawn_parent(In(parent): In<Entity>, mut commands: Commands) {
            commands.entity(parent).despawn_recursive();
        }
        app.world_mut().run_system_once_with(parent, despawn_parent);

        // Now a later "tick" runs dom_sync logic for the child: it looks up
        // entity_map[child], gets the dangling Entity, and does the same
        // unguarded insert that the production box/sphere path does.
        fn unguarded_insert_like_dom_sync(
            stub: Res<EntityMapStub>,
            mut commands: Commands,
        ) {
            commands
                .entity(stub.child)
                .insert(GlobalTransform::default());
        }
        app.add_systems(Update, unguarded_insert_like_dom_sync);

        let result = catch_unwind(AssertUnwindSafe(|| app.update()));
        assert!(
            result.is_err(),
            "step5: descendant left in entity_map after parent despawn_recursive \
             reproduces the production B0003 panic"
        );
    }

    /// Step 6: same scenario as step 5 but using `get_entity` as the guard.
    /// Confirms the targeted fix shape for the dom_sync_system sites that
    /// look up `bevy_ent` from `entity_map` and unconditionally call
    /// `commands.entity(bevy_ent).insert(...)` (dom.rs:1590, :1647, :1674).
    #[test]
    fn engine_race_step6_get_entity_guard_on_dangling_descendant() {
        use bevy::prelude::*;

        let mut app = App::new();
        let parent = app.world_mut().spawn(SpatialBundle::default()).id();
        let child = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(parent).add_child(child);

        #[derive(Resource)]
        struct EntityMapStub {
            child: Entity,
        }
        app.insert_resource(EntityMapStub { child });

        fn despawn_parent(In(parent): In<Entity>, mut commands: Commands) {
            commands.entity(parent).despawn_recursive();
        }
        app.world_mut().run_system_once_with(parent, despawn_parent);

        fn guarded_insert(stub: Res<EntityMapStub>, mut commands: Commands) {
            if let Some(mut ec) = commands.get_entity(stub.child) {
                ec.insert(GlobalTransform::default());
            }
        }
        app.add_systems(Update, guarded_insert);

        // Must not panic.
        app.update();
    }

    /// Step 7: actually reproduce B0003 — the production panic shape.
    ///
    /// B0003 is distinct from the "EntityCommands for entity which doesn't
    /// exist" panic of step 1: B0003 only fires when the entity was ALIVE at
    /// queue time and dead at flush time. Production stack trace ends in
    /// `bevy_ecs::system::commands::mod.rs:1256` with that exact wording.
    ///
    /// Trigger: in a single system, queue insert into `child`, then queue
    /// `despawn_recursive` of `parent` (where parent was already linked to
    /// child outside this system). Apply order:
    ///   1. insert(child, bundle)            — child alive, OK so far
    ///   2. despawn_recursive(parent)        — kills child too
    /// Wait — that order applies insert first, which succeeds. So flip the
    /// queue order: despawn first, insert second.
    #[test]
    fn engine_race_step7_b0003_queue_order_repro() {
        use bevy::prelude::*;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut app = App::new();
        let parent = app.world_mut().spawn(SpatialBundle::default()).id();
        let child = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(parent).add_child(child);

        #[derive(Resource)]
        struct Pair {
            parent: Entity,
            child: Entity,
        }
        app.insert_resource(Pair { parent, child });

        // Simulates dom_sync iterating [parent_id, child_id]:
        //   - parent: text/model in-place replacement → despawn_recursive
        //   - child:  unguarded `commands.entity(bevy_ent_old).insert(...)`
        // entity_map[child] still points to the now-doomed child entity.
        fn racy(p: Res<Pair>, mut commands: Commands) {
            commands.entity(p.parent).despawn_recursive();
            commands.entity(p.child).insert(GlobalTransform::default());
        }
        app.add_systems(Update, racy);

        let result = catch_unwind(AssertUnwindSafe(|| app.update()));
        assert!(
            result.is_err(),
            "step7: queue-order race must panic — this is the production B0003"
        );
    }

    /// Step 8: same race, but the second op uses `get_entity`. Bevy's
    /// `get_entity` checks the world snapshot at QUEUE time, so it cannot
    /// know that a later despawn in the same buffer will kill the entity —
    /// `get_entity` returns `Some` and the queued insert still panics.
    ///
    /// This means `get_entity` only fixes step 5 (stale handles) — it does
    /// NOT fix step 7. The full fix needs ordering or queue-time recording
    /// of pending-despawns. Pin the failing behavior so we don't regress
    /// to "just sprinkle get_entity everywhere and call it done."
    #[test]
    fn engine_race_step8_get_entity_does_not_fix_queue_order() {
        use bevy::prelude::*;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut app = App::new();
        let parent = app.world_mut().spawn(SpatialBundle::default()).id();
        let child = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(parent).add_child(child);

        #[derive(Resource)]
        struct Pair {
            parent: Entity,
            child: Entity,
        }
        app.insert_resource(Pair { parent, child });

        fn racy(p: Res<Pair>, mut commands: Commands) {
            commands.entity(p.parent).despawn_recursive();
            if let Some(mut ec) = commands.get_entity(p.child) {
                ec.insert(GlobalTransform::default());
            }
        }
        app.add_systems(Update, racy);

        let result = catch_unwind(AssertUnwindSafe(|| app.update()));
        assert!(
            result.is_err(),
            "step8: get_entity does NOT defuse the same-frame queue-order race"
        );
    }

    /// Step 9: the actual fix shape for the queue-order race — flush the
    /// despawn to the world BEFORE queueing the insert. In dom_sync this
    /// would mean: when iterating dirty nodes, process all in-place
    /// despawn_recursive paths (model/text/skybox) FIRST, flush commands,
    /// THEN process descendants. Or apply despawns directly via exclusive
    /// world access (like apply_js_tick already does for JS-side removes).
    #[test]
    fn engine_race_step9_apply_despawn_before_insert_is_safe() {
        use bevy::prelude::*;

        let mut app = App::new();
        let parent = app.world_mut().spawn(SpatialBundle::default()).id();
        let child = app.world_mut().spawn(SpatialBundle::default()).id();
        app.world_mut().entity_mut(parent).add_child(child);

        // Despawn synchronously via exclusive world access first.
        app.world_mut().entity_mut(parent).despawn_recursive();

        #[derive(Resource)]
        struct ChildEnt(Entity);
        app.insert_resource(ChildEnt(child));

        // Now the descendant lookup happens AFTER the despawn applied,
        // so get_entity correctly returns None.
        fn guarded(c: Res<ChildEnt>, mut commands: Commands) {
            if let Some(mut ec) = commands.get_entity(c.0) {
                ec.insert(GlobalTransform::default());
            }
        }
        app.add_systems(Update, guarded);

        app.update();
    }

    // ─── Performance regression: cube animation hot-path ─────────────────────
    //
    // Antes la demo de cubos sostenía 3000 cubos a ~400 fps; tras un cambio,
    // 500 cubos caen por debajo de 60 fps. Causa raíz: `mark_dirty_system`
    // encolaba `try_insert(Dirty)` por cubo cada frame, y `dom_sync_system`
    // hacía `remove::<Dirty>` por cubo cada frame. Eso son 1–2 archetype
    // moves por entidad por frame, copiando todos los componentes de
    // `PbrBundle` a otro archetype. Estos tests pinchan la garantía de
    // que el hot-path NO produce archetype churn.

    /// `mark_dirty_system` no debe insertar `Dirty` por frame en entidades
    /// existentes. Si lo hiciera, archetype churn O(N) por frame durante
    /// animaciones masivas tira FPS.
    #[test]
    fn mark_dirty_system_no_dirty_insertion_during_animation() {
        use bevy::prelude::*;

        let mut app = App::new();
        // Entidad creada sin `Dirty` (simula entidad ya sincronizada por
        // dom_sync en frames anteriores).
        let bevy_ent = app.world_mut().spawn(SpatialBundle::default()).id();

        let mut entity_map = EntityMap::default();
        entity_map.0.insert(42, bevy_ent);
        app.insert_resource(entity_map);
        app.insert_resource(DirtyNodes::default());
        app.add_systems(Update, mark_dirty_system);

        // Simula 30 "frames" de animación: cada frame el nodo se marca
        // dirty (como hace `apply_transform_updates`).
        for _ in 0..30 {
            app.world_mut().resource_mut::<DirtyNodes>().0.push(42);
            app.update();
        }

        assert!(
            !app.world().entity(bevy_ent).contains::<Dirty>(),
            "mark_dirty_system insertó Dirty: regresión perf cubos (archetype churn)"
        );
        // Tras dedup la lista de dirty queda con un único id por frame.
        assert_eq!(app.world().resource::<DirtyNodes>().0.len(), 1);
    }

    /// `apply_transform_updates` debe escribir el Transform2 de SPECS,
    /// drenar `TransformUpdates`, y marcar el nodo en `DirtyNodes` +
    /// `TransformOnlyDirtyNodes` SIN reservar HashMaps intermedios.
    /// (La forma observable: comportamiento correcto en una pasada.)
    #[test]
    fn apply_transform_updates_streams_directly_to_specs() {
        use crate::{TransformUpdates, TransformOnlyDirtyNodes};
        use virtual_dom::dom::element::{Attrs, build_world, Hierarchy, Tag, Transform2};

        let mut specs_world = build_world();
        let mut node_ids: Vec<u32> = Vec::new();
        let mut node_ents: Vec<SpecEntity> = Vec::new();
        {
            let entities = specs_world.entities();
            let mut tags = specs_world.write_storage::<Tag>();
            let mut trs = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            for _ in 0..128 {
                let e = entities.create();
                tags.insert(e, Tag("box".into())).ok();
                trs.insert(
                    e,
                    Transform2 {
                        position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                        rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                        scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                    },
                )
                .ok();
                hier.insert(e, Hierarchy { parent: None, children: vec![] }).ok();
                attrs.insert(e, Attrs(HashMap::new())).ok();
                node_ids.push(e.id());
                node_ents.push(e);
            }
        }
        specs_world.maintain();

        let dom_data = VirtualDomData {
            nodes: node_ids
                .iter()
                .zip(node_ents.iter())
                .map(|(id, e)| (*id, *e))
                .collect(),
        };

        let mut app = App::new();
        app.insert_resource(ElemenetWorld(specs_world));
        app.insert_resource(dom_data);
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(TransformOnlyDirtyNodes::default());
        app.add_systems(Update, apply_transform_updates);

        // Push posición + rotación por nodo (pattern de setTransformBatch).
        {
            let mut upd = app.world_mut().resource_mut::<TransformUpdates>();
            for (i, &id) in node_ids.iter().enumerate() {
                upd.positions.push((
                    id,
                    js_runtime::Vec3 { x: i as f32, y: 0.0, z: 0.0 },
                ));
                upd.rotations.push((
                    id,
                    js_runtime::Vec3 { x: 0.0, y: i as f32 * 0.01, z: 0.0 },
                ));
            }
        }
        app.update();

        // TransformUpdates drenado.
        let upd = app.world().resource::<TransformUpdates>();
        assert!(upd.is_empty(), "TransformUpdates no se drenó");

        // SPECS Transform2 actualizado.
        let world = &app.world().resource::<ElemenetWorld>().0;
        let trs = world.read_storage::<Transform2>();
        for (i, &id) in node_ids.iter().enumerate() {
            let e = world.entities().entity(id);
            let t = trs.get(e).expect("Transform2 missing");
            assert_eq!(t.position.x, i as f32);
            assert!((t.rotation.y - i as f32 * 0.01).abs() < 1e-5);
        }

        // Cada nodo aparece marcado (puede aparecer múltiples veces antes
        // de dedup; lo importante es que esté presente).
        let dirty = app.world().resource::<DirtyNodes>();
        let to_dirty = app.world().resource::<TransformOnlyDirtyNodes>();
        let dirty_set: HashSet<u32> = dirty.0.iter().copied().collect();
        for &id in &node_ids {
            assert!(dirty_set.contains(&id), "nodo {} no está dirty", id);
            assert!(to_dirty.0.contains(&id), "nodo {} no está en transform-only", id);
        }
    }

    /// Nodos no adjuntos al árbol (no en `VirtualDomData::nodes`) reciben
    /// la actualización de Transform2 pero NO se marcan dirty — evita
    /// gastar work de dom_sync en entidades sin Bevy entity.
    #[test]
    fn apply_transform_updates_ignores_detached_nodes_for_dirty() {
        use crate::{TransformUpdates, TransformOnlyDirtyNodes};
        use virtual_dom::dom::element::{build_world, Transform2};

        let mut specs_world = build_world();
        let nid = {
            let entities = specs_world.entities();
            let mut trs = specs_world.write_storage::<Transform2>();
            let e = entities.create();
            trs.insert(
                e,
                Transform2 {
                    position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            )
            .ok();
            e.id()
        };
        specs_world.maintain();

        let mut app = App::new();
        app.insert_resource(ElemenetWorld(specs_world));
        app.insert_resource(VirtualDomData::default()); // sin entradas → detached
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(TransformOnlyDirtyNodes::default());
        app.add_systems(Update, apply_transform_updates);

        {
            let mut upd = app.world_mut().resource_mut::<TransformUpdates>();
            upd.positions
                .push((nid, js_runtime::Vec3 { x: 5.0, y: 0.0, z: 0.0 }));
        }
        app.update();

        // Transform2 sí se escribe (mantener invariante).
        let world = &app.world().resource::<ElemenetWorld>().0;
        let trs = world.read_storage::<Transform2>();
        let e = world.entities().entity(nid);
        assert_eq!(trs.get(e).unwrap().position.x, 5.0);

        // Pero el nodo no se marcó dirty.
        assert!(app.world().resource::<DirtyNodes>().0.is_empty());
        assert!(
            app.world()
                .resource::<TransformOnlyDirtyNodes>()
                .0
                .is_empty()
        );
    }

    // ─── Regression: bullet stuck in air (zombies demo) ──────────────────────
    //
    // Tras quitar `Dirty` del hot-path, los sistemas del DOM dejaron de tener
    // Commands como barrera implícita. Sin `.chain()` Bevy podía correr
    // `dom_sync_system` ANTES que `apply_transform_updates` en el mismo frame:
    //   - dom_sync drena `dirty_nodes` (vacío en ese momento) y sale.
    //   - apply_transform_updates escribe SPECS y marca dirty para el frame
    //     siguiente.
    //   - dom_sync no vuelve a correr ese frame → Transform de Bevy nunca se
    //     actualiza.
    // Resultado visible: bullets de zombies aparecen pero quedan congelados
    // ("se queda en el aire en el punto donde aparece sin ser afectada por
    // su animacion"), 97% del tiempo, dependiendo del orden no-determinista
    // que elige el scheduler.
    //
    // El fix es chain(). Estos tests pinchan el invariante.

    /// Una única ronda de update con (a) entidad ya creada en Bevy, (b) un
    /// transform_update pendiente: el Transform de Bevy debe quedar escrito
    /// en el MISMO frame, sin importar el orden interno del scheduler.
    #[test]
    fn js_position_update_reaches_bevy_transform_same_frame() {
        use crate::{
            EntityMap, SharedResources, TextMaterialCache, PrimitiveMaterialCache,
            TransformOnlyDirtyNodes, TransformUpdates,
        };
        use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<Image>();
        app.init_asset::<Scene>();

        // SPECS: 1 nodo "sphere" attached al árbol.
        let mut specs_world = virtual_dom::dom::element::build_world();
        let nid = {
            let entities = specs_world.entities();
            let mut tags = specs_world.write_storage::<Tag>();
            let mut trs = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let e = entities.create();
            tags.insert(e, Tag("sphere".into())).ok();
            trs.insert(
                e,
                Transform2 {
                    position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            )
            .ok();
            hier.insert(e, Hierarchy { parent: None, children: vec![] }).ok();
            attrs.insert(e, Attrs(HashMap::new())).ok();
            e.id()
        };
        specs_world.maintain();
        let nspec = specs_world.entities().entity(nid);

        // Bevy: entidad real con PbrBundle (lo que dom_sync produciría tras CREATE).
        let bevy_ent = app
            .world_mut()
            .spawn(SpatialBundle {
                transform: Transform::from_xyz(0.0, 0.0, 0.0),
                ..Default::default()
            })
            .id();

        // Recursos del flujo DOM.
        app.insert_resource(ElemenetWorld(specs_world));
        let mut dom_data = VirtualDomData::default();
        dom_data.nodes.insert(nid, nspec);
        app.insert_resource(dom_data);

        let mut entity_map = EntityMap::default();
        entity_map.0.insert(nid, bevy_ent);
        app.insert_resource(entity_map);

        app.insert_resource(DirtyNodes::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(TransformOnlyDirtyNodes::default());
        app.insert_resource(crate::AttributeUpdates::default());
        app.insert_resource(crate::JsSnapshotState::default());
        app.insert_resource(crate::permissions::SpacePolicies::default());
        app.insert_resource(crate::CurrentUrl("luna://test".to_string()));
        app.insert_resource(LogPanel::default());
        app.insert_resource(crate::PerformanceStats::default());
        app.insert_resource(crate::ScriptLoadStates::default());
        app.insert_resource(crate::PendingModelLoads::default());
        app.insert_resource(crate::ModelLoadStates::default());
        app.insert_resource(crate::PendingScripts::default());
        app.insert_resource(crate::IncludeLoadStates::default());
        app.insert_resource(crate::SkyboxEntity::default());
        app.insert_resource(crate::TokioRuntime(
            tokio::runtime::Runtime::new().expect("tokio rt"),
        ));
        app.insert_resource(crate::IoService::default());

        // Recursos compartidos: meshes/materials triviales.
        let (cube, plane, sphere, cylinder, default_mat) = {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let cube = meshes.add(bevy::math::primitives::Cuboid::new(1.0, 1.0, 1.0));
            let plane = meshes.add(bevy::math::primitives::Rectangle::new(1.0, 1.0));
            let sphere = meshes.add(bevy::math::primitives::Sphere::new(0.5).mesh());
            let cylinder = meshes.add(bevy::math::primitives::Cylinder::new(0.5, 1.0).mesh());
            let mut mats = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            let default_mat = mats.add(StandardMaterial::default());
            (cube, plane, sphere, cylinder, default_mat)
        };
        app.insert_resource(SharedResources {
            cube_mesh: cube,
            plane_mesh: plane,
            sphere_mesh: sphere,
            cylinder_mesh: cylinder,
            default_material: default_mat,
        });
        app.insert_resource(TextMaterialCache::default());
        app.insert_resource(PrimitiveMaterialCache::default());

        // Schedule: el orden del binario real, con .chain() para forzar
        // determinismo. Si .chain() se rompe, este test debería fallar.
        app.add_systems(
            Update,
            (
                apply_transform_updates,
                apply_attribute_updates.run_if(|a: Res<crate::AttributeUpdates>| !a.0.is_empty()),
                mark_dirty_system,
                dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
            )
                .chain(),
        );

        // Simula JS: posición nueva en TransformUpdates (como apply_js_tick).
        {
            let mut upd = app.world_mut().resource_mut::<TransformUpdates>();
            upd.positions
                .push((nid, js_runtime::Vec3 { x: 7.0, y: 2.0, z: -3.0 }));
        }

        app.update();

        // Bevy Transform debe reflejar la nueva posición EN ESTE FRAME.
        let bevy_tr = app.world().entity(bevy_ent).get::<Transform>().unwrap();
        assert!(
            (bevy_tr.translation.x - 7.0).abs() < 1e-5,
            "Transform.x = {} (esperado 7.0). Bullet stuck regression — orden de sistemas",
            bevy_tr.translation.x
        );
        assert!((bevy_tr.translation.y - 2.0).abs() < 1e-5);
        assert!((bevy_tr.translation.z - (-3.0)).abs() < 1e-5);
    }

    /// Múltiples frames de animación: cada frame inyecta una posición nueva,
    /// y cada frame el Bevy Transform debe quedar al día. Replica el patrón
    /// de `requestAnimationFrame(animate)` de zombies.js.
    #[test]
    fn js_position_animation_streams_each_frame_to_bevy() {
        use crate::{
            EntityMap, SharedResources, TextMaterialCache, PrimitiveMaterialCache,
            TransformOnlyDirtyNodes, TransformUpdates,
        };
        use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<Image>();
        app.init_asset::<Scene>();

        let mut specs_world = virtual_dom::dom::element::build_world();
        let nid = {
            let entities = specs_world.entities();
            let mut tags = specs_world.write_storage::<Tag>();
            let mut trs = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let e = entities.create();
            tags.insert(e, Tag("sphere".into())).ok();
            trs.insert(
                e,
                Transform2 {
                    position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            )
            .ok();
            hier.insert(e, Hierarchy { parent: None, children: vec![] }).ok();
            attrs.insert(e, Attrs(HashMap::new())).ok();
            e.id()
        };
        specs_world.maintain();
        let nspec = specs_world.entities().entity(nid);

        let bevy_ent = app
            .world_mut()
            .spawn(SpatialBundle {
                transform: Transform::from_xyz(0.0, 0.0, 0.0),
                ..Default::default()
            })
            .id();

        app.insert_resource(ElemenetWorld(specs_world));
        let mut dom_data = VirtualDomData::default();
        dom_data.nodes.insert(nid, nspec);
        app.insert_resource(dom_data);
        let mut entity_map = EntityMap::default();
        entity_map.0.insert(nid, bevy_ent);
        app.insert_resource(entity_map);
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(TransformOnlyDirtyNodes::default());
        app.insert_resource(crate::AttributeUpdates::default());
        app.insert_resource(crate::JsSnapshotState::default());
        app.insert_resource(crate::permissions::SpacePolicies::default());
        app.insert_resource(crate::CurrentUrl("luna://test".to_string()));
        app.insert_resource(LogPanel::default());
        app.insert_resource(crate::PerformanceStats::default());
        app.insert_resource(crate::ScriptLoadStates::default());
        app.insert_resource(crate::PendingModelLoads::default());
        app.insert_resource(crate::ModelLoadStates::default());
        app.insert_resource(crate::PendingScripts::default());
        app.insert_resource(crate::IncludeLoadStates::default());
        app.insert_resource(crate::SkyboxEntity::default());
        app.insert_resource(crate::TokioRuntime(
            tokio::runtime::Runtime::new().expect("tokio rt"),
        ));
        app.insert_resource(crate::IoService::default());

        let (cube, plane, sphere, cylinder, default_mat) = {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let cube = meshes.add(bevy::math::primitives::Cuboid::new(1.0, 1.0, 1.0));
            let plane = meshes.add(bevy::math::primitives::Rectangle::new(1.0, 1.0));
            let sphere = meshes.add(bevy::math::primitives::Sphere::new(0.5).mesh());
            let cylinder = meshes.add(bevy::math::primitives::Cylinder::new(0.5, 1.0).mesh());
            let mut mats = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            let default_mat = mats.add(StandardMaterial::default());
            (cube, plane, sphere, cylinder, default_mat)
        };
        app.insert_resource(SharedResources {
            cube_mesh: cube,
            plane_mesh: plane,
            sphere_mesh: sphere,
            cylinder_mesh: cylinder,
            default_material: default_mat,
        });
        app.insert_resource(TextMaterialCache::default());
        app.insert_resource(PrimitiveMaterialCache::default());

        app.add_systems(
            Update,
            (
                apply_transform_updates,
                apply_attribute_updates.run_if(|a: Res<crate::AttributeUpdates>| !a.0.is_empty()),
                mark_dirty_system,
                dom_sync_system.run_if(|d: Res<DirtyNodes>| !d.0.is_empty()),
            )
                .chain(),
        );

        // 30 frames de animación parabólica.
        for frame in 0..30 {
            let t = frame as f32 * 0.016;
            let x = 80.0 * t;
            let y = 1.6 - 4.0 * t * t;
            let z = -100.0 * t;
            {
                let mut upd = app.world_mut().resource_mut::<TransformUpdates>();
                upd.positions.push((nid, js_runtime::Vec3 { x, y, z }));
            }
            app.update();

            let bevy_tr = app.world().entity(bevy_ent).get::<Transform>().unwrap();
            assert!(
                (bevy_tr.translation.x - x).abs() < 1e-3,
                "frame {}: Transform.x = {} (esperado {})",
                frame,
                bevy_tr.translation.x,
                x
            );
            assert!(
                (bevy_tr.translation.y - y).abs() < 1e-3,
                "frame {}: Transform.y = {} (esperado {})",
                frame,
                bevy_tr.translation.y,
                y
            );
            assert!(
                (bevy_tr.translation.z - z).abs() < 1e-3,
                "frame {}: Transform.z = {} (esperado {})",
                frame,
                bevy_tr.translation.z,
                z
            );
        }
    }

    // ─── Regression: "Blocked invalid local position write" (zombies) ───────
    //
    // Bug shape: tras `appendChild` desde JS, `apply_js_tick` retiraba el
    // node_id de `detached_globals` aunque dom_data aún no lo tuviera. En el
    // siguiente frame, si `js_update_snapshots_system` corría antes que
    // `commit_pending_js_attaches_system`, `sync_space_handle_table` veía el
    // node_id ni en `allowed` ni en `detached_globals` → poda mapping local→
    // global. Próximas escrituras de pos/rot del JS para ese local_id fallan
    // con "Blocked invalid local position write" (logged por miles).
    //
    // Fix: `apply_js_tick` solo retira de `detached_globals` para nodos ya
    // attached antes (`already_attached_dirty_ids`). El retiro para
    // `newly_attached_ids` lo hace `commit_pending_js_attaches_system`.

    /// Pinchar invariante: tras `commit_pending_js_attaches_system`, los nodos
    /// recién attached deben quedar fuera de `detached_globals` para todos los
    /// space tables que los conozcan.
    #[test]
    fn commit_pending_js_attaches_clears_detached_globals() {
        use crate::{PendingJsAttachNodes, PendingJsFirstRenderNodes, SpaceHandleTables};
        use crate::types::SpaceHandleTable;
        use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

        let mut specs_world = virtual_dom::dom::element::build_world();
        let nid = {
            let entities = specs_world.entities();
            let mut tags = specs_world.write_storage::<Tag>();
            let mut trs = specs_world.write_storage::<Transform2>();
            let mut hier = specs_world.write_storage::<Hierarchy>();
            let mut attrs = specs_world.write_storage::<Attrs>();
            let e = entities.create();
            tags.insert(e, Tag("sphere".into())).ok();
            trs.insert(
                e,
                Transform2 {
                    position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            )
            .ok();
            hier.insert(e, Hierarchy { parent: None, children: vec![] }).ok();
            attrs.insert(e, Attrs(HashMap::new())).ok();
            e.id()
        };
        specs_world.maintain();

        let mut tables = SpaceHandleTables::default();
        let space_id = 100u32;
        let mut t = SpaceHandleTable {
            runtime_id: 1,
            next_local_id: 5,
            ..Default::default()
        };
        // Bullet asignado pero todavía NO attached → en detached_globals.
        t.global_to_local.insert(nid, 5);
        t.local_to_global.insert(5, nid);
        t.detached_globals.insert(nid);
        tables.by_space.insert(space_id, t);

        let mut app = App::new();
        app.insert_resource(ElemenetWorld(specs_world));
        app.insert_resource(VirtualDomData::default());
        app.insert_resource(PendingJsFirstRenderNodes::default());
        let mut pending = PendingJsAttachNodes::default();
        pending.0.push(nid);
        app.insert_resource(pending);
        app.insert_resource(tables);
        app.add_systems(Update, commit_pending_js_attaches_system);
        app.update();

        // dom_data ahora contiene el nodo.
        let dom_data = app.world().resource::<VirtualDomData>();
        assert!(dom_data.nodes.contains_key(&nid));

        // detached_globals limpio para ese nodo.
        let tables = app.world().resource::<SpaceHandleTables>();
        let table = tables.by_space.get(&space_id).unwrap();
        assert!(
            !table.detached_globals.contains(&nid),
            "commit no retiró nodo de detached_globals: regresión bullets"
        );
        // Mapping local→global preservado (lo que evita "Blocked invalid local position write").
        assert_eq!(table.local_to_global.get(&5), Some(&nid));
    }

    // ─── Navigation stress: stale state corrupts new space ───────────────────
    //
    // Repro user-reported: "si uso la demo de zombies un rato, luego voy a
    // luna://home, no se carga ningún elemento".
    //
    // Causa: tras spam de createElement/remove desde JS, los Resources
    // `TransformUpdates`, `TransformOnlyDirtyNodes`, `PendingJsAttachNodes`,
    // `PendingJsFirstRenderNodes` acumulan node_ids del SPECS world viejo.
    // En `commit_pending_document_load_system`, `world.0 = new_world;` (de
    // `build_world()`) reinicia los slot ids desde 0. Las colas stale tienen
    // ids como 5, 6, 7, ... que ahora coinciden con entidades del nuevo HSML
    // (botones, textos, etc.) → escritura corrupta sobre el nuevo árbol.
    //
    // Fix: drenar todas esas colas en commit_pending_document_load_system.

    fn build_dummy_bundle(xml: &str) -> crate::LoadedDocumentBundle {
        crate::LoadedDocumentBundle {
            root_xml: xml.to_string(),
            includes: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    fn populate_specs_with_bullets(specs_world: &mut SpecWorld, count: usize) -> Vec<u32> {
        use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};
        let mut ids = Vec::new();
        let entities = specs_world.entities();
        let mut tags = specs_world.write_storage::<Tag>();
        let mut trs = specs_world.write_storage::<Transform2>();
        let mut hier = specs_world.write_storage::<Hierarchy>();
        let mut attrs = specs_world.write_storage::<Attrs>();
        for _ in 0..count {
            let e = entities.create();
            tags.insert(e, Tag("sphere".into())).ok();
            trs.insert(
                e,
                Transform2 {
                    position: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    rotation: virtual_dom::dom::element::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scale: virtual_dom::dom::element::Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            )
            .ok();
            hier.insert(e, Hierarchy { parent: None, children: vec![] }).ok();
            attrs.insert(e, Attrs(HashMap::new())).ok();
            ids.push(e.id());
        }
        ids
    }

    fn install_navigation_resources(app: &mut App) {
        use crate::*;
        app.insert_resource(VirtualDomData::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(DirtyNodes::default());
        app.insert_resource(TransformUpdates::default());
        app.insert_resource(TransformOnlyDirtyNodes::default());
        app.insert_resource(AttributeUpdates::default());
        app.insert_resource(JsSnapshotState::default());
        app.insert_resource(crate::permissions::SpacePolicies::default());
        app.insert_resource(CurrentUrl("luna://test".to_string()));
        app.insert_resource(LogPanel::default());
        app.insert_resource(PerformanceStats::default());
        app.insert_resource(ScriptLoadStates::default());
        app.insert_resource(PendingModelLoads::default());
        app.insert_resource(ModelLoadStates::default());
        app.insert_resource(PendingScripts::default());
        app.insert_resource(IncludeLoadStates::default());
        app.insert_resource(SkyboxEntity::default());
        app.insert_resource(PendingDocumentLoads::default());
        app.insert_resource(DocumentLoadState::default());
        app.insert_resource(NavigationEpoch::default());
        app.insert_resource(PendingJsAttachNodes::default());
        app.insert_resource(PendingJsFirstRenderNodes::default());
        app.insert_resource(PendingIncludes::default());
        app.insert_resource(SpaceHandleTables::default());
        app.insert_resource(DeleteRequests::default());
        app.insert_resource(TextMaterialCache::default());
        app.insert_resource(PrimitiveMaterialCache::default());
        app.insert_resource(TokioRuntime(
            tokio::runtime::Runtime::new().expect("tokio rt"),
        ));
        app.insert_resource(IoService::default());
        app.insert_non_send_resource(crate::js::ScriptRuntimeManager::default());

        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        let cube = meshes.add(bevy::math::primitives::Cuboid::new(1.0, 1.0, 1.0));
        let plane = meshes.add(bevy::math::primitives::Rectangle::new(1.0, 1.0));
        let sphere = meshes.add(bevy::math::primitives::Sphere::new(0.5).mesh());
        let cylinder = meshes.add(bevy::math::primitives::Cylinder::new(0.5, 1.0).mesh());
        let mut mats = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        let default_mat = mats.add(StandardMaterial::default());
        app.insert_resource(SharedResources {
            cube_mesh: cube,
            plane_mesh: plane,
            sphere_mesh: sphere,
            cylinder_mesh: cylinder,
            default_material: default_mat,
        });
    }

    /// Stress: 50 ciclos de create-many+delete, luego navegación. Las colas
    /// JS→DOM stale del espacio viejo NO deben sobrevivir el commit; si
    /// sobreviven, se aplican a los slot ids del SPECS world nuevo.
    #[test]
    fn navigation_clears_stale_js_queues_after_heavy_churn() {
        use crate::*;
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<Image>();
        app.init_asset::<Scene>();
        install_navigation_resources(&mut app);

        // Espacio inicial: SPECS world con un montón de bullets.
        {
            let mut specs_world = virtual_dom::dom::element::build_world();
            let bullet_ids = populate_specs_with_bullets(&mut specs_world, 50);
            specs_world.maintain();

            // Simular spam JS→DOM: pos updates, attaches, first-render pendings.
            {
                let mut updates = app.world_mut().resource_mut::<TransformUpdates>();
                for id in &bullet_ids {
                    updates.positions.push((*id, js_runtime::Vec3 { x: 1.0, y: 0.0, z: 0.0 }));
                    updates.rotations.push((*id, js_runtime::Vec3 { x: 0.0, y: 0.0, z: 0.0 }));
                }
            }
            {
                let mut to_dirty = app.world_mut().resource_mut::<TransformOnlyDirtyNodes>();
                to_dirty.0.extend(bullet_ids.iter().copied());
            }
            {
                let mut attaches = app.world_mut().resource_mut::<PendingJsAttachNodes>();
                attaches.0.extend(bullet_ids.iter().copied());
            }
            {
                let mut first = app.world_mut().resource_mut::<PendingJsFirstRenderNodes>();
                for id in &bullet_ids {
                    first.0.push((*id, 1));
                }
            }
            {
                let mut attrs = app.world_mut().resource_mut::<AttributeUpdates>();
                for id in &bullet_ids {
                    attrs.0.push((*id, "color".into(), "#FFD700".into()));
                }
            }
            // Instalar el SPECS world viejo.
            app.insert_resource(ElemenetWorld(specs_world));
        }

        // Sanity: las colas tienen contenido stale.
        assert!(!app.world().resource::<TransformUpdates>().positions.is_empty());
        assert!(!app.world().resource::<TransformOnlyDirtyNodes>().0.is_empty());
        assert!(!app.world().resource::<PendingJsAttachNodes>().0.is_empty());
        assert!(!app.world().resource::<PendingJsFirstRenderNodes>().0.is_empty());
        assert!(!app.world().resource::<AttributeUpdates>().0.is_empty());

        // Disparar navegación: poner pending document load con HSML mínimo.
        app.world_mut().resource_mut::<NavigationEpoch>().0 += 1;
        let epoch = app.world().resource::<NavigationEpoch>().0;
        let url = "luna://home".to_string();
        app.world_mut().resource_mut::<DocumentLoadState>().0 =
            Some(ActiveDocumentLoad { epoch, url: url.clone() });
        app.world_mut().resource_mut::<PendingDocumentLoads>().0.push(
            CompletedDocumentLoad {
                epoch,
                url,
                result: Ok(build_dummy_bundle(
                    r#"<hsml><space><box id="home_btn" sx="1" sy="1" sz="1" /><text value="Home"/></space></hsml>"#,
                )),
            },
        );

        app.add_systems(Update, commit_pending_document_load_system);
        app.update();

        // Tras commit: TODAS las colas JS→DOM stale deben quedar drenadas.
        let upd = app.world().resource::<TransformUpdates>();
        assert!(upd.positions.is_empty(), "TransformUpdates.positions stale");
        assert!(upd.rotations.is_empty(), "TransformUpdates.rotations stale");
        assert!(upd.scales.is_empty(), "TransformUpdates.scales stale");
        assert!(
            app.world().resource::<TransformOnlyDirtyNodes>().0.is_empty(),
            "TransformOnlyDirtyNodes stale"
        );
        assert!(
            app.world().resource::<PendingJsAttachNodes>().0.is_empty(),
            "PendingJsAttachNodes stale tras navegación: regresión home no carga"
        );
        assert!(
            app.world().resource::<PendingJsFirstRenderNodes>().0.is_empty(),
            "PendingJsFirstRenderNodes stale"
        );
        assert!(
            app.world().resource::<AttributeUpdates>().0.is_empty(),
            "AttributeUpdates stale"
        );

        // Y el HSML nuevo está cargado.
        let dom_data = app.world().resource::<VirtualDomData>();
        assert!(
            !dom_data.nodes.is_empty(),
            "Tras navegación, dom_data.nodes vacío: home no cargó"
        );
        let dirty = app.world().resource::<DirtyNodes>();
        assert!(
            !dirty.0.is_empty(),
            "Tras navegación, dirty_nodes vacío: nodos del nuevo HSML no se procesarán"
        );
    }

    /// Round-trip: A → B → A, con churn JS entre cada navegación. Debe quedar
    /// limpio cada vez. Replica patrón "demo zombies → home → demo zombies".
    #[test]
    fn navigation_round_trip_with_churn_keeps_state_clean() {
        use crate::*;
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<Image>();
        app.init_asset::<Scene>();
        install_navigation_resources(&mut app);

        // Necesitamos ElemenetWorld. Empezar con uno fresco.
        {
            let mut specs_world = virtual_dom::dom::element::build_world();
            specs_world.maintain();
            app.insert_resource(ElemenetWorld(specs_world));
        }

        app.add_systems(Update, commit_pending_document_load_system);

        let pages = vec![
            ("luna://zombies", r#"<hsml><space id="z1"><box id="fire" sx="1" sy="1" sz="1"/></space></hsml>"#),
            ("luna://home",    r#"<hsml><space id="h1"><box id="btn1" sx="1" sy="1" sz="1"/><text value="A"/></space></hsml>"#),
            ("luna://zombies", r#"<hsml><space id="z2"><box id="fire" sx="1" sy="1" sz="1"/></space></hsml>"#),
            ("luna://home",    r#"<hsml><space id="h2"><box id="btn2" sx="1" sy="1" sz="1"/><text value="B"/></space></hsml>"#),
        ];

        for (i, (url, xml)) in pages.iter().enumerate() {
            // Simular churn JS antes de navegar.
            let mut churn_ids: Vec<u32> = Vec::new();
            {
                let mut world = app.world_mut().resource_mut::<ElemenetWorld>();
                churn_ids = populate_specs_with_bullets(&mut world.0, 30);
                world.0.maintain();
            }
            {
                let mut updates = app.world_mut().resource_mut::<TransformUpdates>();
                for id in &churn_ids {
                    updates.positions.push((*id, js_runtime::Vec3 { x: 5.0, y: 1.0, z: -2.0 }));
                }
            }
            {
                let mut attaches = app.world_mut().resource_mut::<PendingJsAttachNodes>();
                attaches.0.extend(churn_ids.iter().copied());
            }
            {
                let mut first = app.world_mut().resource_mut::<PendingJsFirstRenderNodes>();
                for id in &churn_ids {
                    first.0.push((*id, 1));
                }
            }

            // Navegar.
            app.world_mut().resource_mut::<NavigationEpoch>().0 += 1;
            let epoch = app.world().resource::<NavigationEpoch>().0;
            app.world_mut().resource_mut::<DocumentLoadState>().0 =
                Some(ActiveDocumentLoad { epoch, url: url.to_string() });
            app.world_mut().resource_mut::<PendingDocumentLoads>().0.push(
                CompletedDocumentLoad {
                    epoch,
                    url: url.to_string(),
                    result: Ok(build_dummy_bundle(xml)),
                },
            );
            app.update();

            // Cada navegación debe limpiar y cargar el nuevo HSML.
            let upd = app.world().resource::<TransformUpdates>();
            assert!(
                upd.positions.is_empty() && upd.rotations.is_empty() && upd.scales.is_empty(),
                "page {}: TransformUpdates stale post-nav", i
            );
            assert!(
                app.world().resource::<PendingJsAttachNodes>().0.is_empty(),
                "page {}: PendingJsAttachNodes stale post-nav", i
            );
            assert!(
                app.world().resource::<PendingJsFirstRenderNodes>().0.is_empty(),
                "page {}: PendingJsFirstRenderNodes stale", i
            );
            assert!(
                !app.world().resource::<VirtualDomData>().nodes.is_empty(),
                "page {} ({}): dom_data vacío — HSML no cargó", i, url
            );
            assert!(
                !app.world().resource::<DirtyNodes>().0.is_empty(),
                "page {} ({}): dirty vacío — render no procesará", i, url
            );
        }
    }

    /// Pinchar la causa específica: un node_id stale en `PendingJsAttachNodes`
    /// del espacio viejo NO debe acabar inyectado en `dom_data.nodes` del
    /// espacio nuevo si ese slot id coincide con una entidad real del nuevo
    /// SPECS world. Es el camino exacto del crash "luna://home no carga".
    #[test]
    fn navigation_does_not_leak_stale_attaches_into_new_dom_data() {
        use crate::*;
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy::asset::AssetPlugin::default());
        app.init_asset::<Mesh>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<Image>();
        app.init_asset::<Scene>();
        install_navigation_resources(&mut app);

        {
            let mut specs_world = virtual_dom::dom::element::build_world();
            specs_world.maintain();
            app.insert_resource(ElemenetWorld(specs_world));
        }

        // Inyectar un set "stale" de ids como si quedaran de un espacio anterior.
        {
            let mut attaches = app.world_mut().resource_mut::<PendingJsAttachNodes>();
            attaches.0.extend([0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        }
        {
            let mut first = app.world_mut().resource_mut::<PendingJsFirstRenderNodes>();
            for id in 0u32..11 {
                first.0.push((id, 1));
            }
        }

        // Navegar a HSML nuevo (el commit reinicia ElemenetWorld desde build_world).
        app.world_mut().resource_mut::<NavigationEpoch>().0 += 1;
        let epoch = app.world().resource::<NavigationEpoch>().0;
        let url = "luna://home".to_string();
        app.world_mut().resource_mut::<DocumentLoadState>().0 =
            Some(ActiveDocumentLoad { epoch, url: url.clone() });
        app.world_mut().resource_mut::<PendingDocumentLoads>().0.push(
            CompletedDocumentLoad {
                epoch,
                url,
                result: Ok(build_dummy_bundle(
                    r#"<hsml><space><box id="btn" sx="1" sy="1" sz="1"/><text value="OK"/></space></hsml>"#,
                )),
            },
        );

        app.add_systems(Update, commit_pending_document_load_system);
        app.update();

        // El nuevo dom_data está poblado solo por los nodos del nuevo HSML.
        let dom_data = app.world().resource::<VirtualDomData>();
        let world = &app.world().resource::<ElemenetWorld>().0;
        let entities = world.entities();
        for &id in dom_data.nodes.keys() {
            let ent = entities.entity(id);
            assert!(
                entities.is_alive(ent),
                "dom_data tiene id {} que no existe en el SPECS world nuevo (leak de attaches stale)",
                id
            );
        }
        assert!(
            app.world().resource::<PendingJsAttachNodes>().0.is_empty(),
            "PendingJsAttachNodes con stale ids tras commit — corromperán el próximo frame"
        );
    }
}
