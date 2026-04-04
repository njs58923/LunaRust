use std::{collections::{HashMap, HashSet}, time::Instant};

use anyhow::Result;
use bevy::prelude::*;
use specs::{Entity as SpecEntity, Join, ReadStorage, World as SpecWorld, WorldExt};
use tokio::runtime::Runtime;

use virtual_dom::{
    dom::{
        element::{build_world, Attrs, Hierarchy, Tag, Transform2},
        hsml::{Include, Model, Script},
        TRANSFORM_POSITION, TRANSFORM_ROTATION, TRANSFORM_SCALE,
    },
    load_xml_from_url, parse_xml,
};

use crate::{
    ActiveDocumentLoad, AsyncDomParams, AttributeUpdates, CompletedDocumentLoad, CurrentUrl,
    DeleteRequests, Dirty, DirtyNodes, DocumentLoadState, ElemenetWorld, EntityMap, IoService,
    LoadedDocumentBundle, LogPanel, ModelLoadState, ModelLoadStates, NavigationEpoch,
    PendingDocumentLoads, PendingModelLoads, PerformanceStats, ReloadTrigger, ScriptLoadState,
    ScriptLoadStates, SharedResources, TextRenderParams, TokioRuntime, VirtualDomData,
    VIRTUAL_ROUTES,
};
use crate::render::{apply_transform, build_text_transform, get_or_create_text_material, parse_hex_color, parse_text_attrs, resolve_remote_path};
use crate::io::{clear_async_node_state, request_document_load, request_model_prepare, request_script_load};

const DOM_SYNC_VERBOSE_LOGS: bool = false;
const ATTR_DELETE_SENTINEL: &str = "[DEL]";

fn primitive_color(attrs_storage: &ReadStorage<Attrs>, node: SpecEntity) -> Color {
    attrs_storage.get(node)
        .and_then(|a| a.0.get("color"))
        .and_then(|c| parse_hex_color(c))
        .unwrap_or(Color::srgb(0.5, 0.5, 0.5))
}

fn spawn_colored_primitive(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    mesh: Handle<Mesh>,
    color: Color,
    transform: Transform,
    double_sided: bool,
) -> Entity {
    let material = materials.add(StandardMaterial {
        base_color: color,
        cull_mode: double_sided.then_some(None).flatten(),
        ..Default::default()
    });
    commands.spawn((PbrBundle {
        mesh,
        material,
        transform,
        ..Default::default()
    }, Dirty)).id()
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
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut delete_requests: ResMut<DeleteRequests>,
    mut pending_scripts: ResMut<crate::PendingScripts>,
    mut log_panel: ResMut<LogPanel>,
    mut manager: NonSendMut<crate::js::ScriptRuntimeManager>,
    mut script_load_states: ResMut<ScriptLoadStates>,
    mut pending_model_loads: ResMut<PendingModelLoads>,
    mut model_load_states: ResMut<ModelLoadStates>,
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
            log_panel.push_info(format!("Dropping stale document load (epoch {epoch}): {url}"));
            continue;
        }

        match result {
            Ok(bundle) => {
                let mut new_world = build_world();
                match flatten_loaded_document_bundle(&mut new_world, &url, &bundle, &mut log_panel) {
                    Ok((new_nodes, new_dirty)) => {
                        for worker in manager.contexts.values_mut() {
                            crate::js::stop_space_worker(worker);
                        }
                        manager.contexts.clear();
                        log_panel.push_info("[JS] All JS contexts cleared for committed navigation");

                        script_load_states.0.clear();
                        pending_model_loads.0.clear();
                        model_load_states.0.clear();
                        attribute_updates.0.clear();
                        delete_requests.0.clear();
                        pending_scripts.0.clear();

                        for bevy_ent in entity_map.0.drain().map(|(_, ent)| ent) {
                            commands.entity(bevy_ent).despawn_recursive();
                        }

                        world.0 = new_world;
                        dom_data.nodes = new_nodes;
                        dirty_nodes.0 = new_dirty;
                        document_load_state.0 = None;
                        log_panel.push_info(format!("Document commit complete (epoch {epoch}): {url}"));
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
    log_panel.push_info(format!("Content retrieved. Length: {} chars", xml_content.len()));

    let root_node = parse_xml(world, &xml_content)
        .map_err(|e| anyhow::anyhow!("Error parsing XML: {e}"))?;

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
            let Some(final_url) = resolve_remote_path(base_url, &src) else {
                log.push_warn(format!("include: cannot resolve src='{src}' against base='{base_url}'"));
                continue;
            };

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

            Hierarchy::add_child(world, parent_ent, child_root);
            collect_subtree_ids(world, child_root, &mut new_dirty);
        }
    }

    Ok(new_dirty)
}

// ─── Attribute updates ───────────────────────────────────────────────────────

pub fn apply_attribute_updates(
    mut attribute_updates: ResMut<AttributeUpdates>,
    mut world: ResMut<ElemenetWorld>,
    mut dirty_nodes: ResMut<DirtyNodes>,
) {
    let entities = world.0.entities();
    let mut attrs_storage = world.0.write_storage::<Attrs>();
    let mut tr_storage = world.0.write_storage::<Transform2>();
    let mut model_storage = world.0.write_storage::<Model>();
    let mut include_storage = world.0.write_storage::<Include>();

    let (px, py, pz) = (TRANSFORM_POSITION[0], TRANSFORM_POSITION[1], TRANSFORM_POSITION[2]);
    let (rx, ry, rz) = (TRANSFORM_ROTATION[0], TRANSFORM_ROTATION[1], TRANSFORM_ROTATION[2]);
    let (sx, sy, sz) = (TRANSFORM_SCALE[0], TRANSFORM_SCALE[1], TRANSFORM_SCALE[2]);

    for (ent_id, key, val) in attribute_updates.0.drain(..) {
        let ent = entities.entity(ent_id);
        if !entities.is_alive(ent) { continue; }

        if attrs_storage.get(ent).is_none() {
            let _ = attrs_storage.insert(ent, Attrs(HashMap::new()));
        }

        if let Some(a) = attrs_storage.get_mut(ent) {
            if val == ATTR_DELETE_SENTINEL {
                a.0.remove(&key);
                if let Some(tr) = tr_storage.get_mut(ent) {
                    match key.as_str() {
                        "x" => tr.position.x = 0.0, "y" => tr.position.y = 0.0, "z" => tr.position.z = 0.0,
                        "rx" => tr.rotation.x = 0.0, "ry" => tr.rotation.y = 0.0, "rz" => tr.rotation.z = 0.0,
                        "sx" => tr.scale.x = 1.0, "sy" => tr.scale.y = 1.0, "sz" => tr.scale.z = 1.0,
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
                k if k == px || k == "x"  => if let Some(f) = parse_f32() { tr.position.x = f; },
                k if k == py || k == "y"  => if let Some(f) = parse_f32() { tr.position.y = f; },
                k if k == pz || k == "z"  => if let Some(f) = parse_f32() { tr.position.z = f; },
                k if k == rx || k == "rx" => if let Some(f) = parse_f32() { tr.rotation.x = f; },
                k if k == ry || k == "ry" => if let Some(f) = parse_f32() { tr.rotation.y = f; },
                k if k == rz || k == "rz" => if let Some(f) = parse_f32() { tr.rotation.z = f; },
                "s" => if let Some(f) = parse_f32() { tr.scale.x = f; tr.scale.y = f; tr.scale.z = f; },
                k if k == sx || k == "sx" => if let Some(f) = parse_f32() { tr.scale.x = f; },
                k if k == sy || k == "sy" => if let Some(f) = parse_f32() { tr.scale.y = f; },
                k if k == sz || k == "sz" => if let Some(f) = parse_f32() { tr.scale.z = f; },
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
        }

        dirty_nodes.0.push(ent_id);
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
) {
    for ent_id in delete_requests.0.drain(..) {
        clear_async_node_state(
            ent_id,
            &mut script_load_states,
            &mut pending_model_loads,
            &mut model_load_states,
        );
        if let Some(bevy_ent) = entity_map.0.remove(&ent_id) {
            commands.entity(bevy_ent).despawn_recursive();
        }
        let sp_ent = world.0.entities().entity(ent_id);
        match world.0.delete_entity(sp_ent) {
            Ok(_) => log_panel.push_info(format!("Entity (ID={}) deleted.", ent_id)),
            Err(_) => log_panel.push_error(format!("Error deleting entity (ID={}).", ent_id)),
        }
    }
}

// ─── Mark dirty ──────────────────────────────────────────────────────────────

pub fn mark_dirty_system(
    world: Res<ElemenetWorld>,
    mut commands: Commands,
    entity_map: Res<EntityMap>,
    dirty_nodes: ResMut<DirtyNodes>,
) {
    for node_id in dirty_nodes.0.iter().copied() {
        if let Some(&ent) = entity_map.0.get(&node_id) {
            commands.entity(ent).insert(Dirty);
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
) {
    let Some(script_comp) = scripts_storage.get(node) else { return; };
    let Some(src) = script_comp.src.as_ref() else { return; };
    let Some(final_url) = resolve_remote_path(&current_url.0, src) else {
        log_panel.push_error(format!("Cannot resolve script src '{src}' against '{}'", current_url.0));
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
    mut query: Query<(Entity, &mut Transform, Option<&Dirty>, Option<&mut Handle<StandardMaterial>>)>,
    asset_server: Res<AssetServer>,
    mut log_panel: ResMut<LogPanel>,
    mut perf_stats: ResMut<PerformanceStats>,
    mut async_dom: AsyncDomParams,
    mut text_render: TextRenderParams,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let start_time = Instant::now();
    if dirty_nodes.0.is_empty() { return; }
    let tokio_rt = &async_dom.tokio_rt;
    let io_service = &async_dom.io_service;
    let current_url = &async_dom.current_url;
    let script_load_states = &mut async_dom.script_load_states;
    let pending_model_loads = &mut async_dom.pending_model_loads;
    let model_load_states = &mut async_dom.model_load_states;

    let tags = world.0.read_storage::<Tag>();
    let transforms = world.0.read_storage::<Transform2>();
    let hierarchies = world.0.read_storage::<Hierarchy>();
    let models = world.0.read_storage::<Model>();
    let scripts_storage = world.0.read_storage::<Script>();
    let attrs_storage = world.0.read_storage::<Attrs>();

    // log_panel.push_info(format!("dom_sync: processing {} dirty nodes...", dirty_nodes.0.len()));

    for node_id in dirty_nodes.0.drain(..) {
        let Some(node) = dom_data.nodes.get(&node_id) else {
            log_panel.push_error(format!("No dom node for id={}", node_id));
            continue;
        };

        let tag = tags.get(*node).map(|t| t.0.clone()).unwrap_or_default();
        let hierarchy = hierarchies.get(*node);
        let parent_id = hierarchy.and_then(|h| h.parent);

        let mut transform_b = Transform::default();
        if let Some(tr2) = transforms.get(*node) {
            apply_transform(tr2, &mut transform_b);
        }

        if let Some(&bevy_ent) = entity_map.0.get(&node_id) {
            // --- Update existing entity ---
            //
            // REGLA: para aplicar cambios de atributos (setAttribute desde JS) hay que
            // leer el valor directamente de `attrs_storage` en esta rama, no depender del
            // componente Bevy `Dirty`.
            //
            // Por qué: `mark_dirty_system` inserta `Dirty` via Commands, que son diferidas
            // (se aplican al final del schedule, no entre sistemas del mismo frame). Entonces
            // cuando `dom_sync_system` corre en el mismo frame, `Option<&Dirty>` siempre
            // llega como `None` para actualizaciones de JS, y el guard `if dirty.is_some()`
            // nunca se cumple. La señal correcta es que el nodo esté en `dirty_nodes.0`
            // (que `apply_attribute_updates` ya garantizó). El componente `Dirty` solo se
            // usa para limpiar el marcador si ya estaba presente por otro motivo.
            //
            // Patrón correcto para agregar soporte a un nuevo tag con atributos mutables:
            //   1. Leer los attrs desde `attrs_storage.get(*node)` directamente.
            //   2. Aplicar el cambio al asset/componente Bevy sin condicionarlo a `dirty`.
            //   3. Llamar `commands.entity(bevy_ent).remove::<Dirty>()` solo si `dirty.is_some()`.
            //   4. Agregar `continue` para no caer en el default de transform-only.
            if tag == "model" {
                let resolved_asset_path = models.get(*node)
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
                            Some(ModelLoadState::Ready { url, asset_path }) if *url == final_url => {
                                Some(asset_path.clone())
                            }
                            Some(ModelLoadState::Failed { url, error }) if *url == final_url => {
                                log_panel.push_warn(format!("Model load failed for {final_url}: {error}"));
                                None
                            }
                            _ => None,
                        }
                    });

                if let Some(asset_path) = resolved_asset_path {
                    commands.entity(bevy_ent).despawn_recursive();
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
                } else if let Ok((_, mut t, dirty, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                    if dirty.is_some() { commands.entity(bevy_ent).remove::<Dirty>(); }
                }
                continue;
            }

            if tag == "text" {
                let empty_map = HashMap::new();
                let attrs_map = attrs_storage.get(*node).map(|a| &a.0).unwrap_or(&empty_map);
                let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                let text_material = get_or_create_text_material(
                    &mut text_render.text_material_cache, &mut text_render.materials,
                    &mut text_render.images, &text_value, text_size, text_color,
                );
                let text_transform = build_text_transform(transform_b, &text_value, text_size);

                let mut updated_in_place = false;
                if let Ok((_, mut t, dirty, maybe_material)) = query.get_mut(bevy_ent) {
                    *t = text_transform;
                    if let Some(mut material_handle) = maybe_material {
                        *material_handle = text_material.clone();
                        if dirty.is_some() { commands.entity(bevy_ent).remove::<Dirty>(); }
                        updated_in_place = true;
                    }
                }
                if updated_in_place { continue; }

                commands.entity(bevy_ent).despawn_recursive();
                entity_map.0.remove(&node_id);
                let new_ent = commands.spawn((PbrBundle {
                    mesh: shared_resources.plane_mesh.clone(),
                    material: text_material,
                    transform: text_transform,
                    ..Default::default()
                }, Dirty)).id();
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
                );
                if let Ok((_, mut t, dirty, _)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                    if dirty.is_some() { commands.entity(bevy_ent).remove::<Dirty>(); }
                }
                continue;
            }

            if tag == "box" || tag == "sphere" || tag == "plane" || tag == "cylinder" {
                if let Ok((_, mut t, dirty, maybe_material)) = query.get_mut(bevy_ent) {
                    *t = transform_b;
                    if let Some(material_handle) = maybe_material {
                        let color = primitive_color(&attrs_storage, *node);
                        if let Some(mat) = text_render.materials.get_mut(&*material_handle) {
                            mat.base_color = color;
                        }
                    }
                    if dirty.is_some() { commands.entity(bevy_ent).remove::<Dirty>(); }
                }
                continue;
            }

            // Default: update transform if dirty
            if let Ok((_, mut t, dirty, _)) = query.get_mut(bevy_ent) {
                if dirty.is_some() {
                    *t = transform_b;
                    commands.entity(bevy_ent).remove::<Dirty>();
                }
            }
        } else {
            // --- Create new entity ---
            if DOM_SYNC_VERBOSE_LOGS { log_panel.push_info("  Creating new entity..."); }

            let new_ent = match tag.as_str() {
                "model" => {
                    let resolved_asset_path = models.get(*node)
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
                                Some(ModelLoadState::Ready { url, asset_path }) if *url == final_url => {
                                    Some(asset_path.clone())
                                }
                                Some(ModelLoadState::Failed { url, error }) if *url == final_url => {
                                    log_panel.push_warn(format!("Model load failed for {final_url}: {error}"));
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
                    );
                    commands.spawn((SpatialBundle { transform: transform_b, ..Default::default() }, Dirty)).id()
                }
                "space" | "include" => {
                    commands.spawn((SpatialBundle { transform: transform_b, ..Default::default() }, Dirty)).id()
                }
                "box" => {
                    let attrs_opt = attrs_storage.get(*node);
                    let color = primitive_color(&attrs_storage, *node);
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
                        mesh,
                        color,
                        transform_b,
                        false,
                    )
                }
                "sphere" => {
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        shared_resources.sphere_mesh.clone(),
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        false,
                    )
                }
                "plane" => {
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        shared_resources.plane_mesh.clone(),
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        true,
                    )
                }
                "cylinder" => {
                    spawn_colored_primitive(
                        &mut commands,
                        &mut text_render.materials,
                        shared_resources.cylinder_mesh.clone(),
                        primitive_color(&attrs_storage, *node),
                        transform_b,
                        false,
                    )
                }
                "text" => {
                    let empty_map = HashMap::new();
                    let attrs_map = attrs_storage.get(*node).map(|a| &a.0).unwrap_or(&empty_map);
                    let (text_value, text_size, text_color) = parse_text_attrs(attrs_map);
                    let text_material = get_or_create_text_material(
                        &mut text_render.text_material_cache, &mut text_render.materials,
                        &mut text_render.images, &text_value, text_size, text_color,
                    );
                    let text_transform = build_text_transform(transform_b, &text_value, text_size);
                    commands.spawn((PbrBundle {
                        mesh: shared_resources.plane_mesh.clone(),
                        material: text_material,
                        transform: text_transform,
                        ..Default::default()
                    }, Dirty)).id()
                }
                _other => {
                    commands.spawn((PbrBundle {
                        mesh: shared_resources.cube_mesh.clone(),
                        material: shared_resources.default_material.clone(),
                        transform: transform_b,
                        ..Default::default()
                    }, Dirty)).id()
                }
            };

            set_parent(&mut commands, new_ent, parent_id, &entity_map);
            entity_map.0.insert(node_id, new_ent);
        }
    }

    perf_stats.dom_sync_ms = start_time.elapsed().as_secs_f32() * 1000.0;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn set_parent(commands: &mut Commands, new_ent: Entity, parent_id: Option<u32>, entity_map: &EntityMap) {
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
    shared_resources: &SharedResources,
) -> Entity {
    if let Some(asset_path) = asset_path {
        let scene_handle = if asset_path.ends_with(".gltf") || asset_path.ends_with(".glb") {
            asset_server.load(format!("{asset_path}#Scene0"))
        } else {
            asset_server.load(asset_path.to_string())
        };
        return commands.spawn((
            SceneBundle {
                scene: scene_handle,
                transform: transform_b,
                ..Default::default()
            },
            Dirty,
        )).id();
    }

    commands.spawn((PbrBundle {
        mesh: shared_resources.cube_mesh.clone(),
        material: shared_resources.default_material.clone(),
        transform: transform_b,
        ..default()
    }, Dirty)).id()
}
