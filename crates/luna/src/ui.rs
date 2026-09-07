use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use specs::{Entity as SpecEntity, Join, World as SpecWorld, WorldExt};
use std::collections::HashMap;
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

use crate::{
    ActiveSpaceIndex, AttributeUpdates, CurrentUrl, DeleteRequests, DevtoolParams, DevtoolTab,
    DeferredSpaceMount, DeferredSpaceMounts, EntityMap, GlobalDevtoolVisible, IoService, LogLevel, LogPanel, MountedSpaceEntry,
    PreferredRenderMode, RenderMode, RootConfig, SpaceMountRequest, SpaceParams,
    SpaceUnmountRequest, UiSystemParams, VirtualDomData,
};

pub fn ui_system(
    mut contexts: EguiContexts,
    root_url: Res<CurrentUrl>,
    mut address_bar: ResMut<crate::AddressBarState>,
    world: Res<crate::ElemenetWorld>,
    entity_map: Res<EntityMap>,
    mut commands: Commands,
    mut camera_query: Query<&mut Transform, With<crate::DesktopCamera>>,
    dom_data: Res<VirtualDomData>,
    mut log_panel: ResMut<LogPanel>,
    mut ui_params: UiSystemParams,
    mut render_mode: ResMut<RenderMode>,
    io_service: Res<IoService>,
    mut space_params: SpaceParams,
    mut devtool: DevtoolParams,
    mut active_space: ResMut<ActiveSpaceIndex>,
    mut global_devtool: ResMut<GlobalDevtoolVisible>,
) {
    let _profile = crate::profiling::span("ui_system");
    let mut pending_unmounts: Vec<usize> = Vec::new();
    let prev_active = active_space.0;
    let prev_active_url = active_space
        .0
        .and_then(|idx| space_params.mounted_spaces.0.get(idx))
        .map(|entry| entry.url.clone());

    if ui_params.dom_mirror.is_changed() {
        sync_mounted_spaces_from_mirror(&ui_params.dom_mirror, &mut space_params.mounted_spaces.0);
    }

    if let (Some(idx), Some(prev_url)) = (active_space.0, prev_active_url) {
        if let Some(entry) = space_params.mounted_spaces.0.get(idx) {
            if entry.url != prev_url {
                address_bar.0 = entry.url.clone();
            }
        }
    }

    egui::Window::new("Navegador")
        .default_width(600.0)
        .show(contexts.ctx_mut(), |ui| {
            // ═══ Row 1: Tab bar ═══
            ui.horizontal_wrapped(|ui| {
                let tab_count = space_params.mounted_spaces.0.len();
                for (idx, entry) in space_params.mounted_spaces.0.iter().enumerate() {
                    let is_active = active_space.0 == Some(idx);
                    let label = short_title(&entry.title, 20);

                    ui.horizontal(|ui| {
                        let btn = ui.selectable_label(is_active, &label);
                        if btn.clicked() {
                            active_space.0 = Some(idx);
                        }
                        if ui.small_button("x").clicked() {
                            pending_unmounts.push(idx);
                        }
                    });
                }

                // [+] creates an empty tab
                if ui.small_button("+").clicked() {
                    let tab_id = space_params.next_tab_id.0;
                    space_params.next_tab_id.0 += 1;
                    space_params.mounted_spaces.0.push(MountedSpaceEntry {
                        tab_id,
                        url: String::new(),
                        title: "New Tab".to_string(),
                    });
                    active_space.0 = Some(tab_count);
                    address_bar.0 = String::new();
                }
            });

            ui.separator();

            // ═══ Row 2: [Home] [URL bar] [Go] [Reload] [Set Home] [Config]═══
            ui.horizontal(|ui| {
                if ui.button("Home").clicked() {
                    let target = ui_params.root_config.home_url.clone();
                    if let Some(idx) = active_space.0 {
                        navigate_tab_to_url(
                            &mut space_params.mounted_spaces.0,
                            &mut space_params.mount_queue.0,
                            &mut space_params.unmount_queue.0,
                             &mut space_params.deferred_mounts.0,
                            idx,
                            target.clone(),
                        );
                        address_bar.0 = target.clone();
                        log_panel.push_info(format!("Navigating to home: {}", target));
                    }
                }

                // Right-side buttons rendered first so available_width() is accurate for the URL bar
                let mut go_clicked = false;
                let mut reload_clicked = false;
                let mut set_home_clicked = false;
                let mut config_clicked = false;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Config").on_hover_text("Settings").clicked() {
                        config_clicked = true;
                    }
                    if ui.button("Set Home").on_hover_text("Set current URL as home").clicked() {
                        set_home_clicked = true;
                    }
                    if ui.button("Reload").clicked() {
                        reload_clicked = true;
                    }
                    if ui.button("Go").clicked() {
                        go_clicked = true;
                    }

                    // URL bar fills remaining space
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut address_bar.0)
                            .desired_width(ui.available_width()),
                    );
                    if resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
                        go_clicked = true;
                    }
                });

                if go_clicked {
                    if let Some(idx) = active_space.0 {
                        let new_url = address_bar.0.trim().to_string();
                        if !new_url.is_empty() {
                            navigate_tab_to_url(
                                &mut space_params.mounted_spaces.0,
                                &mut space_params.mount_queue.0,
                                &mut space_params.unmount_queue.0,
                                 &mut space_params.deferred_mounts.0,
                                idx,
                                new_url.clone(),
                            );
                            log_panel.push_info(format!("Navigating tab to: {}", new_url));
                        }
                    }
                }

                if reload_clicked {
                    if let Some(idx) = active_space.0 {
                        let tab = space_params.mounted_spaces.0[idx].clone();
                        if !devtool.keep_logs.0 {
                            if let Some(space_id) =
                                find_mounted_space_by_tab_id(&world.0, tab.tab_id)
                            {
                                log_panel.clear_for_space(space_id);
                            }
                        }
                        reload_tab(
                            &space_params.mounted_spaces.0,
                            &mut space_params.mount_queue.0,
                            &mut space_params.unmount_queue.0,
                            &mut space_params.deferred_mounts.0,
                            idx,
                        );
                        if !tab.url.is_empty() {
                            log_panel.push_info(format!("Reloading tab: {}", tab.url));
                        }
                    }
                }

                if set_home_clicked {
                    let new_home = address_bar.0.trim().to_string();
                    if !new_home.is_empty() {
                        ui_params.root_config.home_url = new_home.clone();
                        match ui_params.root_config.save() {
                            Ok(path) => log_panel
                                .push_info(format!("Home tab URL saved to {}", path.display())),
                            Err(e) => log_panel.push_error(format!("Failed saving home URL: {e}")),
                        }
                    }
                }

                if config_clicked {
                    devtool.config_visible.0 = !devtool.config_visible.0;
                }
            });

            ui.separator();

            // ═══ Row 3: Mode + devtools ═══
            ui.horizontal(|ui| {
                let label = if render_mode.is_vr {
                    "Desktop"
                } else {
                    "VR"
                };
                if ui.button(label).on_hover_text("Switch render mode").clicked() {
                    render_mode.is_vr = !render_mode.is_vr;
                    log_panel.push_info(format!(
                        "Mode: {}",
                        if render_mode.is_vr { "VR" } else { "Desktop" }
                    ));
                }

                if ui.button("Devtool").on_hover_text("Devtool for active space").clicked() {
                    devtool.visible.0 = !devtool.visible.0;
                }

                if ui
                    .button("Global")
                    .on_hover_text("Global devtool (full DOM)")
                    .clicked()
                {
                    global_devtool.0 = !global_devtool.0;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "FPS: {} | Ent: {}",
                        ui_params.fps_counter.fps, ui_params.entity_counter.count
                    ));
                });
            });
        });

    // When switching tabs, load the tab's committed URL into the draft (discard edits)
    if active_space.0 != prev_active {
        if let Some(idx) = active_space.0 {
            if let Some(entry) = space_params.mounted_spaces.0.get(idx) {
                address_bar.0 = entry.url.clone();
            }
        }
    }

    // Process unmounts (reverse order to keep indices valid)
    pending_unmounts.sort_unstable();
    pending_unmounts.dedup();
    for idx in pending_unmounts.into_iter().rev() {
        if idx < space_params.mounted_spaces.0.len() {
            let removed = space_params.mounted_spaces.0.remove(idx);
            enqueue_tab_unmount(&mut space_params.unmount_queue.0, &removed);
            if !removed.url.trim().is_empty() {
                log_panel.push_info(format!("Unmounting space: {}", removed.url));
            }
            // Adjust active index
            match active_space.0 {
                Some(a) if a == idx => {
                    if space_params.mounted_spaces.0.is_empty() {
                        active_space.0 = None;
                    } else {
                        active_space.0 = Some(a.min(space_params.mounted_spaces.0.len() - 1));
                    }
                }
                Some(a) if a > idx => active_space.0 = Some(a - 1),
                _ => {}
            }
        }
    }

    // ═══ Per-space Devtool window ═══
    if devtool.visible.0 {
        let devtool_title = if let Some(idx) = active_space.0 {
            if let Some(entry) = space_params.mounted_spaces.0.get(idx) {
                format!("Devtool - {}", short_title(&entry.title, 30))
            } else {
                "Devtool".to_string()
            }
        } else {
            "Devtool (no tab selected)".to_string()
        };

        egui::Window::new(devtool_title)
            .id(egui::Id::new("devtool_space_window"))
            .show(contexts.ctx_mut(), |ui| {
                // Tab bar
                ui.horizontal(|ui| {
                    if ui.selectable_label(devtool.state.active_tab == DevtoolTab::Status, "Status").clicked() {
                        devtool.state.active_tab = DevtoolTab::Status;
                    }
                    if ui.selectable_label(devtool.state.active_tab == DevtoolTab::Hsml, "HSML").clicked() {
                        devtool.state.active_tab = DevtoolTab::Hsml;
                    }
                    if ui.selectable_label(devtool.state.active_tab == DevtoolTab::Logs, "Console").clicked() {
                        devtool.state.active_tab = DevtoolTab::Logs;
                    }
                    if ui.selectable_label(devtool.state.active_tab == DevtoolTab::Redes, "Network").clicked() {
                        devtool.state.active_tab = DevtoolTab::Redes;
                    }
                    if ui.selectable_label(devtool.state.active_tab == DevtoolTab::Resources, "Resources").clicked() {
                        devtool.state.active_tab = DevtoolTab::Resources;
                    }
                });
                ui.separator();

                let active_tab_space_id: Option<u32> = active_space.0
                    .and_then(|idx| space_params.mounted_spaces.0.get(idx))
                    .and_then(|entry| find_mounted_space_by_tab_id(&world.0, entry.tab_id));

                match devtool.state.active_tab {
                    DevtoolTab::Status => {
                        ui.label(format!("Entities: {}", ui_params.entity_counter.count));
                        ui.label(format!("FPS: {}", ui_params.fps_counter.fps));
                        let p = &*ui_params.perf_stats;
                        ui.label(format!(
                            "dom_sync: {:.2} ms  (dirty {} · fast {} · requeue {})",
                            p.dom_sync_ms, p.dom_sync_dirty_in, p.dom_sync_fastlane, p.dom_sync_requeued
                        ));
                        ui.label(format!("transform_only set: {}", p.transform_only_len));
                        ui.label(format!(
                            "js_snapshot: {:.2} ms  (full_rebuild {} · mirror {} · sent {} · waiting_ack {})",
                            p.js_snapshot_ms, p.mirror_full_rebuild, p.mirror_nodes, p.snapshots_sent, p.waiting_on_ack
                        ));
                    }
                    DevtoolTab::Hsml => {
                        let w = ui.available_width();
                        egui::ScrollArea::vertical()
                            .id_source("tree_space_scroll")
                            .max_width(w)
                            .max_height(300.0)
                            .show(ui, |ui| {
                                let space_ent = active_tab_space_id.map(|id| {
                                    world.0.entities().entity(id)
                                });
                                if let Some(root) = space_ent {
                                    show_element_tree(
                                        ui,
                                        root,
                                        &world.0,
                                        &entity_map,
                                        &mut commands,
                                        &mut camera_query,
                                        &dom_data,
                                        &mut devtool.attribute_updates,
                                        &mut devtool.delete_requests,
                                        &mut log_panel,
                                    );
                                } else {
                                    ui.label("No hay espacio activo.");
                                }
                            });
                    }
                    DevtoolTab::Logs => {
                        render_logs(ui, &log_panel, active_tab_space_id);
                        ui.horizontal(|ui| {
                            if ui.button("Clear").clicked() {
                                if let Some(space_id) = active_tab_space_id {
                                    log_panel.clear_for_space(space_id);
                                } else {
                                    log_panel.clear();
                                }
                            }
                            if ui.button("Copy").clicked() {
                                let text = copy_logs(&log_panel, active_tab_space_id);
                                ui.output_mut(|o| o.copied_text = text);
                            }
                            ui.checkbox(&mut devtool.keep_logs.0, "Mantener registros");
                        });
                    }
                    DevtoolTab::Redes => {
                        render_network(ui, &io_service, active_tab_space_id);
                    }
                    DevtoolTab::Resources => {
                        if let Some(idx) = active_space.0 {
                            if let Some(entry) = space_params.mounted_spaces.0.get(idx) {
                                ui.label(format!("Tab URL: {}", entry.url));
                                if let Some(space_id) =
                                    find_mounted_space_by_tab_id(&world.0, entry.tab_id)
                                {
                                    ui.label(format!("DOM Space ID: {}", space_id));
                                    if let Some(spaces) = find_root_managed_spaces(&world.0).iter().find(|s| s.id() == space_id) {
                                        ui.label(format!("Space Title: {}", get_space_debug_title(&world.0, *spaces)));
                                    }
                                } else {
                                    ui.label("Space not found in DOM");
                                }
                            }
                        } else {
                            ui.label("No tab selected");
                        }
                    }
                }
            });
    }

    // ═══ Global Devtool window ═══
    if global_devtool.0 {
        egui::Window::new("Global Devtool")
            .id(egui::Id::new("devtool_global_window"))
            .show(contexts.ctx_mut(), |ui| {
                egui::CollapsingHeader::new("Full HSML Tree").show(ui, |ui| {
                    let w = ui.available_width();
                    egui::ScrollArea::vertical()
                        .id_source("tree_global_scroll")
                        .max_width(w)
                        .max_height(300.0)
                        .show(ui, |ui| {
                            if let Some(root) = get_root_entity(&world.0) {
                                show_element_tree(
                                    ui,
                                    root,
                                    &world.0,
                                    &entity_map,
                                    &mut commands,
                                    &mut camera_query,
                                    &dom_data,
                                    &mut devtool.attribute_updates,
                                    &mut devtool.delete_requests,
                                    &mut log_panel,
                                );
                            } else {
                                ui.label("No elements in scene.");
                            }
                        });
                });

                egui::CollapsingHeader::new("All Logs").show(ui, |ui| {
                    render_logs(ui, &log_panel, None);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            log_panel.clear();
                        }
                        if ui.button("Copy").clicked() {
                            let text = copy_logs(&log_panel, None);
                            ui.output_mut(|o| o.copied_text = text);
                        }
                    });
                });

                egui::CollapsingHeader::new("All Network").show(ui, |ui| {
                    render_network(ui, &io_service, None);
                });

                egui::CollapsingHeader::new("Mounted Spaces / Policies").show(ui, |ui| {
                    ui.label(format!("Total tabs in UI: {}", space_params.mounted_spaces.0.len()));
                    for (idx, entry) in space_params.mounted_spaces.0.iter().enumerate() {
                        let is_active = active_space.0 == Some(idx);
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{}[{}|tab:{}] {}",
                                if is_active { "▶ " } else { "  " },
                                idx,
                                entry.tab_id,
                                short_title(&entry.url, 40),
                            ));
                        });
                        if let Some(space_id) =
                            find_mounted_space_by_tab_id(&world.0, entry.tab_id)
                        {
                            ui.indent(format!("mounted_{}", idx), |ui| {
                                ui.label(format!("  DOM space_id: {}", space_id));
                            });
                        } else if !entry.url.is_empty() {
                            ui.indent(format!("mounted_{}", idx), |ui| {
                                ui.colored_label(egui::Color32::YELLOW, "  ⚠ Not found in DOM");
                            });
                        }
                    }
                });
            });
    }

    // ═══ Permission prompt ═══
    if let Some(prompt) = ui_params.permission_prompts.pending.front().cloned() {
        let mut decision = None;
        egui::Window::new("Permission request")
            .id(egui::Id::new("permission_prompt_window"))
            .collapsible(false)
            .resizable(false)
            .show(contexts.ctx_mut(), |ui| {
                ui.label(format!("Origin: {}", prompt.origin));
                ui.label(format!("Capability: {}", prompt.capability_label()));
                ui.label(format!("Requested by spaces: {:?}", prompt.space_ids));
                ui.separator();
                ui.label("This decision is saved for this origin and capability.");
                ui.horizontal(|ui| {
                    if ui.button("Allow").clicked() {
                        decision = Some(crate::permissions::PermissionDecision::Allow);
                    }
                    if ui.button("Deny").clicked() {
                        decision = Some(crate::permissions::PermissionDecision::Deny);
                    }
                });
            });

        if let Some(decision) = decision {
            ui_params.permission_decisions.set_decision(
                prompt.origin.clone(),
                prompt.capability,
                decision,
            );
            ui_params
                .permission_prompts
                .resolve(&prompt.origin, prompt.capability);
            ui_params.space_policies.dirty = true;
            match ui_params.permission_decisions.save() {
                Ok(path) => log_panel.push_info(format!(
                    "[perm] {:?} {} for {}; saved to {}",
                    decision,
                    prompt.capability_label(),
                    prompt.origin,
                    path.display()
                )),
                Err(err) => log_panel.push_error(format!(
                    "[perm] decision applied for this session but could not be saved: {err}"
                )),
            }
        }
    }

    // ═══ Config window ═══
    if devtool.config_visible.0 {
        egui::Window::new("Config")
            .id(egui::Id::new("config_window"))
            .show(contexts.ctx_mut(), |ui| {
                let mut auto_load_home = ui_params.root_config.auto_load_home;
                ui.checkbox(&mut ui_params.agent.enabled, "Enable local MCP");
                ui.label(ui_params.agent.connection_label());
                ui.separator();
                if ui
                    .checkbox(&mut auto_load_home, "Auto-load home on startup")
                    .changed()
                {
                    ui_params.root_config.auto_load_home = auto_load_home;
                }
                ui.horizontal(|ui| {
                    ui.label("Home URL:");
                    ui.text_edit_singleline(&mut ui_params.root_config.home_url);
                });
                egui::ComboBox::from_label("Preferred render mode")
                    .selected_text(match ui_params.root_config.preferred_render_mode {
                        PreferredRenderMode::Desktop => "Desktop",
                        PreferredRenderMode::Vr => "VR",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut ui_params.root_config.preferred_render_mode,
                            PreferredRenderMode::Desktop,
                            "Desktop",
                        );
                        ui.selectable_value(
                            &mut ui_params.root_config.preferred_render_mode,
                            PreferredRenderMode::Vr,
                            "VR",
                        );
                    });
                ui.separator();
                egui::CollapsingHeader::new("Site permissions").show(ui, |ui| {
                    let entries = ui_params.permission_decisions.entries();
                    if entries.is_empty() {
                        ui.label("No saved permission decisions.");
                    }

                    let mut update = None;
                    for (origin, capability, decision) in entries {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{} · {} · {:?}",
                                origin,
                                crate::permissions::describe_capability_bits(capability),
                                decision
                            ));
                            match decision {
                                crate::permissions::PermissionDecision::Allow => {
                                    if ui.button("Revoke").clicked() {
                                        update = Some((
                                            origin.clone(),
                                            capability,
                                            Some(crate::permissions::PermissionDecision::Deny),
                                        ));
                                    }
                                }
                                crate::permissions::PermissionDecision::Deny => {
                                    if ui.button("Ask again").clicked() {
                                        update = Some((origin.clone(), capability, None));
                                    }
                                }
                            }
                        });
                    }

                    if let Some((origin, capability, decision)) = update {
                        match decision {
                            Some(decision) => {
                                ui_params.permission_decisions.set_decision(
                                    origin.clone(),
                                    capability,
                                    decision,
                                );
                            }
                            None => {
                                ui_params
                                    .permission_decisions
                                    .forget_decision(&origin, capability);
                            }
                        }
                        ui_params.space_policies.dirty = true;
                        match ui_params.permission_decisions.save() {
                            Ok(path) => log_panel.push_info(format!(
                                "[perm] updated {} for {}; saved to {}",
                                crate::permissions::describe_capability_bits(capability),
                                origin,
                                path.display()
                            )),
                            Err(err) => log_panel.push_error(format!(
                                "[perm] permission updated for this session but could not be saved: {err}"
                            )),
                        }
                    }
                });
                ui.separator();
                ui.label(format!("Root shell URL: {}", root_url.0));
                ui.label(format!("Config path: {}", RootConfig::path().display()));
                if ui.button("Save Config").clicked() {
                    match ui_params.root_config.save() {
                        Ok(path) => {
                            log_panel.push_info(format!("Root config saved to {}", path.display()))
                        }
                        Err(e) => log_panel.push_error(format!("Failed saving root config: {e}")),
                    }
                }
            });
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn navigate_tab_to_url(
    mounted_spaces: &mut Vec<MountedSpaceEntry>,
    mount_queue: &mut Vec<SpaceMountRequest>,
    unmount_queue: &mut Vec<SpaceUnmountRequest>,
    deferred_mounts: &mut Vec<DeferredSpaceMount>,
    idx: usize,
    new_url: String,
) {
    if new_url.trim().is_empty() {
        return;
    }

    let old = mounted_spaces[idx].clone();

    if !old.url.is_empty() {
        if old.url != new_url {
            enqueue_tab_unmount(unmount_queue, &old);
            deferred_mounts.push(DeferredSpaceMount {
                wait_gone_tab_id: old.tab_id,
                mount: SpaceMountRequest::new(old.tab_id, new_url.clone()),
            });
        }
    } else {
        mount_queue.push(SpaceMountRequest::new(old.tab_id, new_url.clone()));
    }

    mounted_spaces[idx].url = new_url.clone();
    mounted_spaces[idx].title = new_url;
}

fn reload_tab(
    mounted_spaces: &[MountedSpaceEntry],
    _mount_queue: &mut Vec<SpaceMountRequest>,
    unmount_queue: &mut Vec<SpaceUnmountRequest>,
    deferred_mounts: &mut Vec<DeferredSpaceMount>,
    idx: usize,
) {
    let tab = &mounted_spaces[idx];
    if tab.url.trim().is_empty() {
        return;
    }
    enqueue_tab_unmount(unmount_queue, tab);
    deferred_mounts.push(DeferredSpaceMount {
        wait_gone_tab_id: tab.tab_id,
        mount: SpaceMountRequest::new(tab.tab_id, tab.url.clone()),
    });
}

fn enqueue_tab_unmount(unmount_queue: &mut Vec<SpaceUnmountRequest>, entry: &MountedSpaceEntry) {
    if entry.url.trim().is_empty() {
        return;
    }
    unmount_queue.push(SpaceUnmountRequest {
        tab_id: entry.tab_id,
        url: entry.url.clone(),
    });
}

fn find_root_space_entity(world: &SpecWorld) -> Option<SpecEntity> {
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();

    for (ent, tag, attr) in (&world.entities(), &tags, &attrs).join() {
        if tag.0 == "space" && attr.0.get("id").map(|v| v.as_str()) == Some("luna_root") {
            return Some(ent);
        }
    }
    None
}

/// Devuelve el space_id del shell ACTIVO (donde corre el script ux_*).
///
/// Diseño:
/// - root_api.js marca el outer wrapper del shell con `system-shell="true"`
///   al hacer `switchMode`. La attr está en el OUTER, no en el HSML inner.
/// - Buscamos ese outer, luego bajamos al primer `<space>` descendiente
///   (el inner cargado por `<include>` — donde vive el worker JS).
/// - Si hay varios outers con system-shell="true" (zombie de switch previo),
///   `switchMode` hace sweep antes de montar nuevo. Aquí devolvemos el primero.
pub fn find_system_shell_space(world: &SpecWorld) -> Option<u32> {
    let entities = world.entities();
    let attrs = world.read_storage::<Attrs>();
    let tags = world.read_storage::<Tag>();
    let hier = world.read_storage::<Hierarchy>();

    // Buscar TODOS los outers con `system-shell="true"` y quedarnos con el
    // de MAYOR id (= montado más recientemente). Si el sweep JS no llegó a
    // remover un wrapper zombi del switch anterior, prefiere el activo.
    // IDs en specs crecen monotónicamente con la creación.
    let mut outer_opt: Option<specs::Entity> = None;
    {
        use specs::Join;
        for (ent, attr, tag) in (&entities, &attrs, &tags).join() {
            if tag.0 != "space" {
                continue;
            }
            if attr.0.get("system-shell").map(|v| v.as_str()) == Some("true") {
                match outer_opt {
                    None => outer_opt = Some(ent),
                    Some(prev) if ent.id() > prev.id() => outer_opt = Some(ent),
                    _ => {}
                }
            }
        }
    }
    let outer = outer_opt?;

    // BFS bajando — devolver primer descendiente que sea `<space>` (= inner).
    let mut stack: Vec<specs::Entity> = Vec::new();
    if let Some(h) = hier.get(outer) {
        for &c in &h.children {
            stack.push(c);
        }
    }
    while let Some(ent) = stack.pop() {
        if !entities.is_alive(ent) {
            continue;
        }
        if let Some(t) = tags.get(ent) {
            if t.0 == "space" {
                return Some(ent.id());
            }
        }
        if let Some(h) = hier.get(ent) {
            for &c in &h.children {
                stack.push(c);
            }
        }
    }
    // Sin inner descendiente: devolvemos outer como fallback.
    Some(outer.id())
}

/// Sube por la jerarquía buscando el ancestor con `data-luna-tab-id`.
/// Útil cuando un script corre en un space inner (cargado por include) que
/// no lleva la attr — la lleva el outer wrapper creado por dimension.luna.
pub fn find_tab_id_for_space(world: &SpecWorld, space_id: u32) -> Option<u64> {
    let entities = world.entities();
    let hier = world.read_storage::<Hierarchy>();
    let attrs = world.read_storage::<Attrs>();

    let mut current_opt = Some(entities.entity(space_id));
    while let Some(current) = current_opt {
        if !entities.is_alive(current) {
            break;
        }
        if let Some(a) = attrs.get(current) {
            if let Some(tid_str) = a.0.get("data-luna-tab-id") {
                if let Ok(tid) = tid_str.parse::<u64>() {
                    return Some(tid);
                }
            }
        }
        current_opt = hier.get(current).and_then(|h| h.parent);
    }
    None
}

/// Versión "active worker" de `find_mounted_space_by_tab_id`. Encuentra el
/// outer wrapper por tab_id, luego desciende al primer `<space>` descendiente
/// — donde vive el worker JS de la app. Si no hay inner, retorna outer.
pub fn find_app_worker_space_by_tab_id(world: &SpecWorld, tab_id: u64) -> Option<u32> {
    let outer_id = find_mounted_space_by_tab_id(world, tab_id)?;
    let entities = world.entities();
    let tags = world.read_storage::<Tag>();
    let hier = world.read_storage::<Hierarchy>();

    // BFS bajando desde outer buscando primer `<space>` descendiente.
    let outer_ent = entities.entity(outer_id);
    if !entities.is_alive(outer_ent) {
        return Some(outer_id);
    }
    let mut stack: Vec<specs::Entity> = Vec::new();
    if let Some(h) = hier.get(outer_ent) {
        for &c in &h.children {
            stack.push(c);
        }
    }
    while let Some(ent) = stack.pop() {
        if !entities.is_alive(ent) {
            continue;
        }
        if let Some(t) = tags.get(ent) {
            if t.0 == "space" {
                return Some(ent.id());
            }
        }
        if let Some(h) = hier.get(ent) {
            for &c in &h.children {
                stack.push(c);
            }
        }
    }
    Some(outer_id)
}

fn find_root_managed_spaces(world: &SpecWorld) -> Vec<SpecEntity> {
    let mut result = Vec::new();
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();

    if let Some(root) = find_root_space_entity(world) {
        if let Some(h) = hier.get(root) {
            for &child in &h.children {
                if let Some(tag) = tags.get(child) {
                    if tag.0 == "space" {
                        if let Some(attr) = attrs.get(child) {
                            if attr.0.get("managed-by").is_some() {
                                result.push(child);
                            }
                        }
                    }
                }
            }
        }
    }
    result
}

/// Read only root/tab/include metadata from the existing mirror. Animated nodes
/// update this mirror too, but must not trigger a scan of the entire Specs world.
fn sync_mounted_spaces_from_mirror(mirror: &crate::js::DomMirror, mounted: &mut [MountedSpaceEntry]) {
    let root = mirror.space_subtrees.keys().filter_map(|id| mirror.nodes.get(&(*id as i32)))
        .find(|node| node.attrs.get("id").is_some_and(|id| id == "luna_root"));
    let Some(root) = root else { return; };
    for child in &root.children {
        let Some(tab) = mirror.nodes.get(child).filter(|n| n.tag == "space" && n.attrs.contains_key("managed-by")) else { continue; };
        let Some(tab_id) = tab.attrs.get("data-luna-tab-id").and_then(|id| id.parse::<u64>().ok()) else { continue; };
        let Some(entry) = mounted.iter_mut().find(|e| e.tab_id == tab_id) else { continue; };
        let url = tab.children.iter().filter_map(|id| mirror.nodes.get(id)).find(|n| n.tag == "include")
            .and_then(|n| n.attrs.get("src")).map(String::as_str).unwrap_or("");
        let title = tab.attrs.get("title").filter(|t| !t.trim().is_empty()).map(String::as_str).unwrap_or(url);
        if entry.url != url { entry.url = url.into(); }
        if entry.title != title { entry.title = title.into(); }
    }
}

#[cfg(test)]
fn sync_mounted_spaces_from_dom(world: &SpecWorld, mounted_spaces: &mut [MountedSpaceEntry]) {
    let dom_spaces = collect_mounted_space_snapshots(world);

    for entry in mounted_spaces.iter_mut() {
        let Some(snapshot) = dom_spaces.get(&entry.tab_id) else {
            continue;
        };

        if entry.url != snapshot.url {
            entry.url = snapshot.url.clone();
        }

        if entry.title != snapshot.title {
            entry.title = snapshot.title.clone();
        }
    }
}

pub(crate) fn collect_mounted_space_snapshots(world: &SpecWorld) -> HashMap<u64, MountedSpaceEntry> {
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let mut snapshots = HashMap::new();

    for space_ent in find_root_managed_spaces(world) {
        let Some(space_attrs) = attrs.get(space_ent) else {
            continue;
        };
        let Some(tab_id) = space_attrs
            .0
            .get("data-luna-tab-id")
            .and_then(|raw| raw.parse::<u64>().ok())
        else {
            continue;
        };

        let mut url = String::new();
        if let Some(node) = hier.get(space_ent) {
            for &child in &node.children {
                if tags.get(child).map(|t| t.0.as_str()) != Some("include") {
                    continue;
                }
                if let Some(include_attrs) = attrs.get(child) {
                    url = include_attrs.0.get("src").cloned().unwrap_or_default();
                }
                break;
            }
        }

        let title = space_attrs
            .0
            .get("title")
            .cloned()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| url.clone());

        snapshots.insert(
            tab_id,
            MountedSpaceEntry {
                tab_id,
                url,
                title,
            },
        );
    }

    snapshots
}

pub fn find_mounted_space_by_tab_id(world: &SpecWorld, tab_id: u64) -> Option<u32> {
    let attrs = world.read_storage::<Attrs>();
    let tab_id = tab_id.to_string();

    for space_ent in find_root_managed_spaces(world) {
        if let Some(attr) = attrs.get(space_ent) {
            if attr.0.get("data-luna-tab-id").map(|v| v.as_str()) == Some(tab_id.as_str()) {
                return Some(space_ent.id());
            }
        }
    }

    None
}

pub fn flush_deferred_space_mounts_system(
    world: Res<crate::ElemenetWorld>,
    mut deferred_mounts: ResMut<DeferredSpaceMounts>,
    mut mount_queue: ResMut<crate::SpaceMountQueue>,
) {
    if deferred_mounts.0.is_empty() {
        return;
    }

    let pending = deferred_mounts.0.drain(..).collect::<Vec<_>>();
    for item in pending {
        let old_gone = find_mounted_space_by_tab_id(&world.0, item.wait_gone_tab_id).is_none();

        if old_gone {
            mount_queue.0.push(item.mount);
        } else {
            deferred_mounts.0.push(item);
        }
    }
}

fn get_space_debug_title(world: &SpecWorld, space: SpecEntity) -> String {
    let attrs = world.read_storage::<Attrs>();
    if let Some(attr) = attrs.get(space) {
        if let Some(id) = attr.0.get("id") {
            return id.clone();
        }
    }
    format!("space:{}", space.id())
}

fn short_title(title: &str, max: usize) -> String {
    if title.len() <= max {
        title.to_string()
    } else {
        format!("{}...", &title[..max - 3])
    }
}

fn render_logs(ui: &mut egui::Ui, log_panel: &LogPanel, filter_space: Option<u32>) {
    let w = ui.available_width();
    egui::ScrollArea::vertical()
        .id_source(format!("logs_{:?}", filter_space))
        .max_width(w)
        .max_height(200.0)
        .show(ui, |ui| {
            for entry in &log_panel.logs {
                if let Some(filter_id) = filter_space {
                    if entry.space_id != Some(filter_id) {
                        continue;
                    }
                }
                match entry.level {
                    LogLevel::Error => {
                        ui.colored_label(egui::Color32::RED, &entry.message);
                    }
                    LogLevel::Warn => {
                        ui.colored_label(egui::Color32::YELLOW, &entry.message);
                    }
                    LogLevel::Info => {
                        ui.label(&entry.message);
                    }
                }
            }
        });
}

fn copy_logs(log_panel: &LogPanel, filter_space: Option<u32>) -> String {
    log_panel
        .logs
        .iter()
        .filter(|entry| {
            if let Some(filter_id) = filter_space {
                entry.space_id == Some(filter_id) || entry.space_id.is_none()
            } else {
                true
            }
        })
        .map(|entry| {
            let prefix = match entry.level {
                LogLevel::Error => "[ERROR] ",
                LogLevel::Warn => "[WARN] ",
                LogLevel::Info => "[INFO] ",
            };
            format!("{}{}", prefix, entry.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_network(ui: &mut egui::Ui, io_service: &IoService, filter_space: Option<u32>) {
    let entries = io_service.network_entries();
    let filter_owner = filter_space.map(|id| format!("space:{}", id));
    let filtered: Vec<_> = if let Some(ref owner_pattern) = filter_owner {
        entries
            .iter()
            .filter(|e| e.owner.contains(owner_pattern))
            .collect()
    } else {
        entries.iter().collect()
    };

    ui.horizontal(|ui| {
        ui.label(format!("Requests: {}", filtered.len()));
        if ui.button("Clear").clicked() {
            io_service.clear_network_entries();
        }
    });
    ui.separator();
    if filtered.is_empty() {
        ui.label("No network activity yet.");
    } else {
        egui::ScrollArea::vertical()
            .id_source(format!("network_{:?}", filter_space))
            .max_height(260.0)
            .show(ui, |ui| {
                for entry in filtered.iter().rev() {
                    let elapsed_ms = entry
                        .finished_at
                        .unwrap_or_else(std::time::Instant::now)
                        .duration_since(entry.started_at)
                        .as_millis();
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("#{}", entry.id));
                            ui.label(entry.kind.label());
                            ui.label(entry.status.label());
                            ui.label(format!("{elapsed_ms} ms"));
                        });
                        ui.label(&entry.url);
                        ui.label(format!("Owner: {}", entry.owner));
                        if let Some(detail) = &entry.detail {
                            ui.label(format!("Detail: {detail}"));
                        }
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::App;
    use specs::{Join, WorldExt};
    use virtual_dom::{dom::element::build_world, parse_xml};

    fn mounted_entry(tab_id: u64, url: &str) -> MountedSpaceEntry {
        MountedSpaceEntry {
            tab_id,
            url: url.to_string(),
            title: url.to_string(),
        }
    }

    fn world_with_duplicate_urls() -> SpecWorld {
        let mut world = build_world();
        parse_xml(
            &mut world,
            r#"
            <hsml>
              <space id="luna_root">
                <space managed-by="dimension.luna" data-luna-tab-id="11">
                  <include src="luna://same" />
                </space>
                <space managed-by="dimension.luna" data-luna-tab-id="22">
                  <include src="luna://same" />
                </space>
              </space>
            </hsml>
            "#,
        )
        .expect("xml parse failed");
        world
    }

    fn expected_space_id_for_tab(world: &SpecWorld, tab_id: &str) -> u32 {
        let entities = world.entities();
        let tags = world.read_storage::<Tag>();
        let attrs = world.read_storage::<Attrs>();

        (&entities, &tags, &attrs)
            .join()
            .find_map(|(ent, tag, attrs)| {
                (tag.0 == "space"
                    && attrs.0.get("managed-by").is_some()
                    && attrs.0.get("data-luna-tab-id").map(|v| v.as_str()) == Some(tab_id))
                .then_some(ent.id())
            })
            .expect("tab space not found")
    }

    #[test]
    fn sync_mounted_spaces_from_dom_updates_url_after_self_navigation() {
        let mut world = build_world();
        parse_xml(
            &mut world,
            r#"
            <hsml>
              <space id="luna_root">
                <space managed-by="dimension.luna" data-luna-tab-id="11">
                  <include src="luna://updated" />
                </space>
              </space>
            </hsml>
            "#,
        )
        .expect("xml parse failed");

        let mut mounted = vec![MountedSpaceEntry {
            tab_id: 11,
            url: "luna://old".to_string(),
            title: "luna://old".to_string(),
        }];

        sync_mounted_spaces_from_dom(&world, &mut mounted);

        assert_eq!(mounted[0].url, "luna://updated");
        assert_eq!(mounted[0].title, "luna://updated");
    }

    #[test]
    fn mirror_tab_sync_ignores_geometry_and_tracks_navigation_and_titles() {
        use crate::js::{DomMirror, DomMirrorNode};
        let mut mirror = DomMirror::default();
        let node = |tag: &str, attrs: &[(&str, &str)], children: Vec<i32>| DomMirrorNode {
            tag: tag.into(), attrs: attrs.iter().map(|(k,v)| (k.to_string(),v.to_string())).collect(), children,
            position: Default::default(), rotation: Default::default(), scale: Default::default(), parent: -1,
        };
        mirror.nodes.insert(1, node("space", &[("id","luna_root")], vec![2]));
        mirror.nodes.insert(2, node("space", &[("managed-by","dimension.luna"),("data-luna-tab-id","11")], vec![3]));
        mirror.nodes.insert(3, node("include", &[("src","luna://updated")], vec![]));
        mirror.space_subtrees.insert(1, Default::default());
        mirror.space_subtrees.insert(2, Default::default());
        for id in 4..10_004 { mirror.nodes.insert(id, node("model", &[], vec![])); }
        let mut tabs = vec![mounted_entry(11, "luna://old"), mounted_entry(12, "luna://other")];
        sync_mounted_spaces_from_mirror(&mirror, &mut tabs);
        assert_eq!(tabs[0].url, "luna://updated");
        assert_eq!(tabs[0].title, "luna://updated");
        mirror.nodes.get_mut(&2).unwrap().attrs.insert("title".into(), "New title".into());
        sync_mounted_spaces_from_mirror(&mirror, &mut tabs);
        assert_eq!(tabs[0].title, "New title");
        assert_eq!(tabs[1].url, "luna://other");
    }

    #[test]
    fn navigate_empty_tab_queues_mount_with_tab_id() {
        let mut mounted = vec![mounted_entry(7, "")];
        let mut mount_queue = Vec::new();
        let mut unmount_queue = Vec::new();
        let mut deferred_mounts = Vec::new();

        navigate_tab_to_url(
            &mut mounted,
            &mut mount_queue,
            &mut unmount_queue,
            &mut deferred_mounts,
            0,
            "luna://home".to_string(),
        );

        assert_eq!(
            mount_queue,
            vec![SpaceMountRequest::new(7, "luna://home".to_string())]
        );
        assert!(unmount_queue.is_empty());
        assert!(deferred_mounts.is_empty());
        assert_eq!(mounted[0].url, "luna://home");
    }

    #[test]
    fn navigate_loaded_tab_to_new_url_unmounts_and_defers_same_tab_id() {
        let mut mounted = vec![mounted_entry(7, "luna://home")];
        let mut mount_queue = Vec::new();
        let mut unmount_queue = Vec::new();
        let mut deferred_mounts = Vec::new();

        navigate_tab_to_url(
            &mut mounted,
            &mut mount_queue,
            &mut unmount_queue,
            &mut deferred_mounts,
            0,
            "luna://about".to_string(),
        );

        assert!(mount_queue.is_empty());
        assert_eq!(
            unmount_queue,
            vec![SpaceUnmountRequest {
                tab_id: 7,
                url: "luna://home".to_string(),
            }]
        );
        assert_eq!(deferred_mounts.len(), 1);
        assert_eq!(deferred_mounts[0].wait_gone_tab_id, 7);
        assert_eq!(
            deferred_mounts[0].mount,
            SpaceMountRequest::new(7, "luna://about".to_string())
        );
        assert_eq!(mounted[0].url, "luna://about");
    }

    #[test]
    fn reload_tab_unmounts_and_defers_same_tab_id() {
        let mounted = vec![mounted_entry(9, "luna://same")];
        let mut mount_queue = Vec::new();
        let mut unmount_queue = Vec::new();
        let mut deferred_mounts = Vec::new();

        reload_tab(
            &mounted,
            &mut mount_queue,
            &mut unmount_queue,
            &mut deferred_mounts,
            0,
        );

        assert!(mount_queue.is_empty());
        assert_eq!(
            unmount_queue,
            vec![SpaceUnmountRequest {
                tab_id: 9,
                url: "luna://same".to_string(),
            }]
        );
        assert_eq!(deferred_mounts.len(), 1);
        assert_eq!(deferred_mounts[0].wait_gone_tab_id, 9);
        assert_eq!(
            deferred_mounts[0].mount,
            SpaceMountRequest::new(9, "luna://same".to_string())
        );
    }

    #[test]
    fn find_mounted_space_by_tab_id_disambiguates_duplicate_urls() {
        let world = world_with_duplicate_urls();

        assert_eq!(
            find_mounted_space_by_tab_id(&world, 11),
            Some(expected_space_id_for_tab(&world, "11"))
        );
        assert_eq!(
            find_mounted_space_by_tab_id(&world, 22),
            Some(expected_space_id_for_tab(&world, "22"))
        );
    }

    #[test]
    fn flush_deferred_mounts_releases_when_other_tab_keeps_same_url() {
        let mut app = App::new();
        app.insert_resource(crate::ElemenetWorld(world_with_duplicate_urls()));
        app.insert_resource(DeferredSpaceMounts(vec![DeferredSpaceMount {
            wait_gone_tab_id: 33,
            mount: SpaceMountRequest::new(33, "luna://next".to_string()),
        }]));
        app.insert_resource(crate::SpaceMountQueue::default());
        app.add_systems(Update, flush_deferred_space_mounts_system);

        app.update();

        assert!(app.world().resource::<DeferredSpaceMounts>().0.is_empty());
        assert_eq!(
            app.world().resource::<crate::SpaceMountQueue>().0,
            vec![SpaceMountRequest::new(33, "luna://next".to_string())]
        );
    }

    #[test]
    fn enqueue_tab_unmount_skips_blank_urls() {
        let mut unmount_queue = Vec::new();

        enqueue_tab_unmount(&mut unmount_queue, &mounted_entry(4, ""));

        assert!(unmount_queue.is_empty());
    }
}

fn get_root_entity(world: &SpecWorld) -> Option<SpecEntity> {
    let hier = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let mut roots = Vec::new();
    for (ent, h) in (&world.entities(), &hier).join() {
        if h.parent.is_none() {
            let tag_name = tags.get(ent).map(|t| t.0.as_str()).unwrap_or("???");
            roots.push((ent, tag_name.to_string()));
        }
    }
    if let Some((ent, _)) = roots.iter().find(|(_, tag)| tag == "hsml") {
        return Some(*ent);
    }
    if let Some((ent, _)) = roots.iter().find(|(_, tag)| tag == "space") {
        return Some(*ent);
    }
    roots.into_iter().next().map(|(ent, _)| ent)
}

fn show_element_tree(
    ui: &mut egui::Ui,
    entity: SpecEntity,
    world: &SpecWorld,
    entity_map: &EntityMap,
    commands: &mut Commands,
    camera_query: &mut Query<&mut Transform, With<crate::DesktopCamera>>,
    dom_data: &VirtualDomData,
    attribute_updates: &mut ResMut<AttributeUpdates>,
    delete_requests: &mut ResMut<DeleteRequests>,
    log_panel: &mut ResMut<LogPanel>,
) {
    let hierarchies = world.read_storage::<Hierarchy>();
    let tags = world.read_storage::<Tag>();
    let attrs = world.read_storage::<Attrs>();
    let transforms = world.read_storage::<Transform2>();

    if let Some(tg) = tags.get(entity) {
        let tag_name = tg.0.as_str();
        let label = if tag_name == "space" || tag_name == "include" {
            format!("[{}] (ID: {:?})", tag_name.to_uppercase(), entity)
        } else {
            format!("{} (ID: {:?})", tag_name, entity)
        };

        ui.collapsing(label, |ui| {
            if let Some(a) = attrs.get(entity) {
                ui.label("Attributes:");

                // Highlight important attributes for space/include
                if tag_name == "space" {
                    if let Some(id) = a.0.get("id") {
                        ui.colored_label(egui::Color32::from_rgb(128, 255, 255), format!("  id = {}", id));
                    }
                    if let Some(resources) = a.0.get("resources") {
                        ui.colored_label(egui::Color32::from_rgb(144, 238, 144), format!("  resources = {}", resources));
                    }
                    if let Some(managed_by) = a.0.get("managed-by") {
                        ui.colored_label(egui::Color32::from_rgb(255, 255, 153), format!("  managed-by = {}", managed_by));
                    }
                    if let Some(sys_space) = a.0.get("system-space") {
                        ui.colored_label(egui::Color32::from_rgb(255, 255, 153), format!("  system-space = {}", sys_space));
                    }
                } else if tag_name == "include" {
                    if let Some(src) = a.0.get("src") {
                        ui.colored_label(egui::Color32::from_rgb(173, 216, 230), format!("  src = {}", src));
                    }
                    if let Some(resources) = a.0.get("resources") {
                        ui.colored_label(egui::Color32::from_rgb(144, 238, 144), format!("  resources = {}", resources));
                    }
                }

                ui.separator();
                ui.label("All Attributes:");
                for (k, v) in &a.0 {
                    ui.horizontal(|ui| {
                        ui.label(k);
                        let mut val = v.clone();
                        if ui.text_edit_singleline(&mut val).changed() {
                            attribute_updates
                                .0
                                .push((entity.id(), k.clone(), val.clone()));
                            log_panel.push_info(format!(
                                "Attr change: Entity({:?}) [{}] = {}",
                                entity, k, val
                            ));
                        }
                        if ui.button("x").on_hover_text("Delete attribute").clicked() {
                            attribute_updates
                                .0
                                .push((entity.id(), k.clone(), "[DEL]".to_string()));
                            log_panel
                                .push_warn(format!("Delete attr: Entity({:?}) [{}]", entity, k));
                        }
                    });
                }
            }

            ui.horizontal(|ui| {
                ui.label("New attribute:");
                let id = ui.make_persistent_id(("new_attr_key", entity.id()));
                let mut key = ui.data_mut(|d| d.get_persisted::<String>(id).unwrap_or_default());
                let te_resp = ui.text_edit_singleline(&mut key);
                ui.data_mut(|d| d.insert_persisted(id, key.clone()));
                if ui.button("Create").clicked() && !key.trim().is_empty() {
                    attribute_updates
                        .0
                        .push((entity.id(), key.clone(), String::new()));
                    log_panel.push_info(format!("Create attr: Entity({:?}) [{}]", entity, key));
                    key.clear();
                    ui.data_mut(|d| d.insert_persisted(id, key));
                    te_resp.request_focus();
                }
            });

            ui.horizontal(|ui| {
                if ui.button("Focus").clicked() {
                    if let Some(tr) = transforms.get(entity) {
                        if let Ok(mut cam) = camera_query.get_single_mut() {
                            let pos = Vec3::new(tr.position.x, tr.position.y, tr.position.z);
                            *cam = Transform::from_translation(pos + Vec3::new(0.0, 3.0, 8.0))
                                .looking_at(pos, Vec3::Y);
                            log_panel.push_info(format!("Camera focused on entity: {:?}", entity));
                        }
                    }
                }
                if ui.button("Delete").clicked() {
                    delete_requests.0.push(entity.id());
                    log_panel.push_warn(format!("Delete request for entity {:?}", entity));
                }
            });

            if let Some(h) = hierarchies.get(entity) {
                for child in &h.children {
                    show_element_tree(
                        ui,
                        *child,
                        world,
                        entity_map,
                        commands,
                        camera_query,
                        dom_data,
                        attribute_updates,
                        delete_requests,
                        log_panel,
                    );
                }
            }
        });
    }
}
