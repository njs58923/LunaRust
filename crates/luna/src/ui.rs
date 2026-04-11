use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use specs::{Entity as SpecEntity, Join, World as SpecWorld, WorldExt};
use virtual_dom::dom::element::{Attrs, Hierarchy, Tag, Transform2};

use crate::{
    AttributeUpdates, CurrentUrl, DeleteRequests, DevtoolParams, DevtoolTab, EntityMap, IoService,
    LogLevel, LogPanel, MountedSpaceEntry, PreferredRenderMode, ReloadTrigger, RenderMode,
    RootConfig, SpaceParams, UiSystemParams, VirtualDomData,
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
) {
    // Track which spaces to unmount (collected during UI rendering)
    let mut pending_unmounts: Vec<usize> = Vec::new();

    egui::Window::new("Navegador").show(contexts.ctx_mut(), |ui| {
        // ── URL bar: mount a space ──
        ui.horizontal(|ui| {
            if ui.button("Home").clicked() {
                url.0 = ui_params.root_config.home_url.clone();
                reload_trigger.0 = true;
                log_panel.push_info("Reloading root document");
            }
            ui.label("URL:");
            let resp = ui.text_edit_singleline(&mut url.0);
            let enter_pressed =
                resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Mount").clicked() || enter_pressed {
                let space_url = url.0.trim().to_string();
                if !space_url.is_empty() {
                    space_params.mount_queue.0.push(space_url.clone());
                    space_params.mounted_spaces.0.push(MountedSpaceEntry {
                        url: space_url.clone(),
                        title: space_url.clone(),
                    });
                    log_panel.push_info(format!("Mounting space: {}", space_url));
                }
            }
            if ui.button("Set Current as Home").clicked() {
                ui_params.root_config.home_url = url.0.clone();
                match ui_params.root_config.save() {
                    Ok(path) => {
                        log_panel.push_info(format!("Home URL saved to {}", path.display()))
                    }
                    Err(error) => log_panel.push_error(format!("Failed saving home URL: {error}")),
                }
            }
        });

        // ── Mounted spaces (tab bar) ──
        if !space_params.mounted_spaces.0.is_empty() {
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for (idx, entry) in space_params.mounted_spaces.0.iter().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(&entry.title);
                            if ui.small_button("x").clicked() {
                                pending_unmounts.push(idx);
                            }
                        });
                    });
                }
            });
        }

        ui.separator();

        ui.horizontal(|ui| {
            let label = if render_mode.is_vr {
                "Switch to Desktop"
            } else {
                "Switch to VR"
            };
            if ui.button(label).clicked() {
                render_mode.is_vr = !render_mode.is_vr;
                log_panel.push_info(format!(
                    "Mode: {}",
                    if render_mode.is_vr { "VR" } else { "Desktop" }
                ));
            }

            if ui.button("Toggle Devtool").clicked() {
                devtool.visible.0 = !devtool.visible.0;
            }
        });
    });

    // Process unmounts (reverse order to keep indices valid)
    pending_unmounts.sort_unstable();
    for idx in pending_unmounts.into_iter().rev() {
        if idx < space_params.mounted_spaces.0.len() {
            let removed = space_params.mounted_spaces.0.remove(idx);
            space_params.unmount_queue.0.push(removed.url.clone());
            log_panel.push_info(format!("Unmounting space: {}", removed.url));
        }
    }

    if devtool.visible.0 {
        egui::Window::new("Devtool")
            .id(egui::Id::new("devtool_window"))
            .show(contexts.ctx_mut(), |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(devtool.state.active_tab == DevtoolTab::Status, "Status")
                        .clicked()
                    {
                        devtool.state.active_tab = DevtoolTab::Status;
                    }
                    if ui
                        .selectable_label(devtool.state.active_tab == DevtoolTab::Hsml, "HSML")
                        .clicked()
                    {
                        devtool.state.active_tab = DevtoolTab::Hsml;
                    }
                    if ui
                        .selectable_label(devtool.state.active_tab == DevtoolTab::Logs, "Console")
                        .clicked()
                    {
                        devtool.state.active_tab = DevtoolTab::Logs;
                    }
                    if ui
                        .selectable_label(devtool.state.active_tab == DevtoolTab::Redes, "Network")
                        .clicked()
                    {
                        devtool.state.active_tab = DevtoolTab::Redes;
                    }
                });
                ui.separator();

                match devtool.state.active_tab {
                    DevtoolTab::Status => {
                        ui.heading("General Status");
                        ui.separator();
                        ui.label(format!("Entities: {}", ui_params.entity_counter.count));
                        ui.label(format!("FPS: {}", ui_params.fps_counter.fps));
                        ui.label(format!(
                            "Last dom_sync: {:.2} ms",
                            ui_params.perf_stats.dom_sync_ms
                        ));
                        ui.separator();
                        ui.heading("Root Config");
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
                                Err(error) => log_panel
                                    .push_error(format!("Failed saving root config: {error}")),
                            }
                        }
                    }
                    DevtoolTab::Hsml => {
                        ui.heading("Element Tree (HSML)");
                        ui.separator();
                        let w = ui.available_width();
                        ui.set_width(w);
                        egui::ScrollArea::vertical()
                            .id_source("tree_scroll_area")
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
                        ui.heading("Console");
                        ui.separator();
                        let w2 = ui.available_width();
                        ui.set_width(w2);
                        egui::ScrollArea::vertical()
                            .id_source("logs_scroll_area")
                            .max_width(w2)
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for entry in &log_panel.logs {
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
                        ui.horizontal(|ui| {
                            if ui.button("Clear logs").clicked() {
                                log_panel.clear();
                            }
                            if ui.button("Copy logs").clicked() {
                                let logs_text: String = log_panel
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
                                    .join("\n");
                                ui.output_mut(|o| o.copied_text = logs_text);
                            }
                        });
                    }
                    DevtoolTab::Redes => {
                        ui.heading("Network");
                        ui.separator();
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
                                .id_source("network_scroll_area")
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
                        if ui.button("🗑").on_hover_text("Delete attribute").clicked() {
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
