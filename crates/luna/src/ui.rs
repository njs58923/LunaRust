use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use specs::{Entity as SpecEntity, Join, World as SpecWorld, WorldExt};
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

use crate::{
    ActiveSpaceIndex, AttributeUpdates, CurrentUrl, DeleteRequests, DevtoolParams, DevtoolTab,
    EntityMap, GlobalDevtoolVisible, IoService, LogLevel, LogPanel, MountedSpaceEntry,
    PreferredRenderMode, ReloadTrigger, RenderMode, RootConfig, SpaceParams, UiSystemParams,
    VirtualDomData,
};

pub fn ui_system(
    mut contexts: EguiContexts,
    mut url: ResMut<CurrentUrl>,
    mut reload_trigger: ResMut<ReloadTrigger>,
    world: Res<crate::ElemenetWorld>,
    entity_map: Res<EntityMap>,
    mut commands: Commands,
    mut camera_query: Query<&mut Transform, With<Camera3d>>,
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
    let mut pending_unmounts: Vec<usize> = Vec::new();

    egui::Window::new("Navegador")
        .default_width(600.0)
        .show(contexts.ctx_mut(), |ui| {
            // ═══ Row 1: Tab bar ═══
            ui.horizontal_wrapped(|ui| {
                let tab_count = space_params.mounted_spaces.0.len();
                for (idx, entry) in space_params.mounted_spaces.0.iter().enumerate() {
                    let is_active = active_space.0 == Some(idx);
                    let label = short_title(&entry.title, 20);

                    let btn = ui.selectable_label(is_active, &label);
                    if btn.clicked() {
                        active_space.0 = Some(idx);
                    }
                    // Right-click to close
                    if btn.secondary_clicked() {
                        pending_unmounts.push(idx);
                    }
                }

                // [+] button to mount new space
                if ui.small_button("+").clicked() {
                    let space_url = url.0.trim().to_string();
                    if !space_url.is_empty() {
                        space_params.mount_queue.0.push(space_url.clone());
                        space_params.mounted_spaces.0.push(MountedSpaceEntry {
                            url: space_url.clone(),
                            title: space_url.clone(),
                        });
                        active_space.0 = Some(tab_count);
                        log_panel.push_info(format!("Mounting space: {}", space_url));
                    }
                }
            });

            ui.separator();

            // ═══ Row 2: URL bar + actions (for active tab) ═══
            ui.horizontal(|ui| {
                if ui.button("Home").clicked() {
                    url.0 = ui_params.root_config.home_url.clone();
                    reload_trigger.0 = true;
                    log_panel.push_info("Reloading root document");
                }

                // Show URL of active space, or the global URL input
                if let Some(idx) = active_space.0 {
                    if let Some(entry) = space_params.mounted_spaces.0.get(idx) {
                        let mut active_url = entry.url.clone();
                        ui.add(
                            egui::TextEdit::singleline(&mut active_url)
                                .desired_width(ui.available_width() - 180.0),
                        );
                        // Update global url for [+] button
                        url.0 = active_url;
                    } else {
                        ui.text_edit_singleline(&mut url.0);
                    }
                } else {
                    let resp = ui.text_edit_singleline(&mut url.0);
                    if resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
                        let space_url = url.0.trim().to_string();
                        if !space_url.is_empty() {
                            let new_idx = space_params.mounted_spaces.0.len();
                            space_params.mount_queue.0.push(space_url.clone());
                            space_params.mounted_spaces.0.push(MountedSpaceEntry {
                                url: space_url.clone(),
                                title: space_url.clone(),
                            });
                            active_space.0 = Some(new_idx);
                            log_panel.push_info(format!("Mounting space: {}", space_url));
                        }
                    }
                }

                // Close active tab
                if active_space.0.is_some() {
                    if ui.button("x").on_hover_text("Close tab").clicked() {
                        pending_unmounts.push(active_space.0.unwrap());
                    }
                }

                if ui.button("Set Home").on_hover_text("Set current URL as home").clicked() {
                    ui_params.root_config.home_url = url.0.clone();
                    match ui_params.root_config.save() {
                        Ok(path) => {
                            log_panel.push_info(format!("Home URL saved to {}", path.display()))
                        }
                        Err(e) => log_panel.push_error(format!("Failed saving home URL: {e}")),
                    }
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

                // Status info on the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "FPS: {} | Ent: {}",
                        ui_params.fps_counter.fps, ui_params.entity_counter.count
                    ));
                });
            });
        });

    // Process unmounts (reverse order to keep indices valid)
    pending_unmounts.sort_unstable();
    pending_unmounts.dedup();
    for idx in pending_unmounts.into_iter().rev() {
        if idx < space_params.mounted_spaces.0.len() {
            let removed = space_params.mounted_spaces.0.remove(idx);
            space_params.unmount_queue.0.push(removed.url.clone());
            log_panel.push_info(format!("Unmounting space: {}", removed.url));
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
                });
                ui.separator();

                // TODO: filter by active space's space_id once we track it
                // For now, show all content (same as before) but with space_id filter on logs

                match devtool.state.active_tab {
                    DevtoolTab::Status => {
                        ui.label(format!("Entities: {}", ui_params.entity_counter.count));
                        ui.label(format!("FPS: {}", ui_params.fps_counter.fps));
                        ui.label(format!(
                            "Last dom_sync: {:.2} ms",
                            ui_params.perf_stats.dom_sync_ms
                        ));
                    }
                    DevtoolTab::Hsml => {
                        let w = ui.available_width();
                        egui::ScrollArea::vertical()
                            .id_source("tree_space_scroll")
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
                    }
                    DevtoolTab::Logs => {
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
                    }
                    DevtoolTab::Redes => {
                        render_network(ui, &io_service, None);
                    }
                }
            });
    }

    // ═══ Global Devtool window ═══
    if global_devtool.0 {
        egui::Window::new("Global Devtool")
            .id(egui::Id::new("devtool_global_window"))
            .show(contexts.ctx_mut(), |ui| {
                egui::CollapsingHeader::new("Root Config").show(ui, |ui| {
                    let mut auto_load_home = ui_params.root_config.auto_load_home;
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
                    ui.label(format!("Current URL: {}", url.0));
                    ui.label(format!("Config path: {}", RootConfig::path().display()));
                    if ui.button("Save Config").clicked() {
                        match ui_params.root_config.save() {
                            Ok(path) => log_panel
                                .push_info(format!("Root config saved to {}", path.display())),
                            Err(e) => {
                                log_panel.push_error(format!("Failed saving root config: {e}"))
                            }
                        }
                    }
                });

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
            });
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn short_title(title: &str, max: usize) -> String {
    if title.len() <= max {
        title.to_string()
    } else {
        format!("{}...", &title[..max - 3])
    }
}

fn render_logs(ui: &mut egui::Ui, log_panel: &LogPanel, _filter_space: Option<u32>) {
    let w = ui.available_width();
    egui::ScrollArea::vertical()
        .id_source(format!("logs_{:?}", _filter_space))
        .max_width(w)
        .max_height(200.0)
        .show(ui, |ui| {
            for entry in &log_panel.logs {
                // TODO: filter by _filter_space when space_id tracking is complete
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

fn copy_logs(log_panel: &LogPanel, _filter_space: Option<u32>) -> String {
    log_panel
        .logs
        .iter()
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

fn render_network(ui: &mut egui::Ui, io_service: &IoService, _filter_space: Option<u32>) {
    let entries = io_service.network_entries();
    ui.horizontal(|ui| {
        ui.label(format!("Requests: {}", entries.len()));
        if ui.button("Clear").clicked() {
            io_service.clear_network_entries();
        }
    });
    ui.separator();
    if entries.is_empty() {
        ui.label("No network activity yet.");
    } else {
        egui::ScrollArea::vertical()
            .id_source(format!("network_{:?}", _filter_space))
            .max_height(260.0)
            .show(ui, |ui| {
                for entry in entries.iter().rev() {
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
    camera_query: &mut Query<&mut Transform, With<Camera3d>>,
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
        ui.collapsing(format!("{} (ID: {:?})", tg.0, entity), |ui| {
            if let Some(a) = attrs.get(entity) {
                ui.label("Attributes:");
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
