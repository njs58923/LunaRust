//! La configuración raíz, vista desde el documento de ajustes.
//!
//! Dos direcciones, y las dos pasan por acá:
//!
//! - **hacia arriba**: `apply_root_patch` toma el parche JSON que manda
//!   `dimention.setRootSettings(...)`, aplica lo que reconoce y **guarda en
//!   disco en el acto**. No hay botón de aplicar: cada cambio se persiste solo.
//! - **hacia abajo**: `publish` arma el JSON que el host inyecta en el
//!   documento para que la interfaz sepa qué mostrar.
//!
//! Quién puede hacerlo no se decide acá: el llamador ya verificó que el
//! documento sea `luna://settings` (ver `agent::is_settings_document`). Este
//! módulo sólo valida el **contenido**.
use bevy::prelude::World;

use crate::permissions::{
    describe_capability_bits, elevated_capability_from_key, elevated_capability_key,
    PermissionDecision, PermissionDecisionStore,
};
use crate::types::{PreferredRenderMode, RootConfig};

/// Un `home_url` más largo que esto no es una URL, es un accidente.
const MAX_URL: usize = 2048;

/// Aplicar un parche. Devuelve el primer problema encontrado; los campos que sí
/// se entendieron ya quedaron aplicados y guardados.
///
/// Se aplica campo por campo y no reemplazando la configuración entera: el
/// documento manda sólo lo que tocó, así que un cliente viejo no puede pisar
/// una preferencia que todavía no conoce.
pub fn apply_root_patch(world: &mut World, patch: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(patch).map_err(|e| format!("patch no es JSON: {e}"))?;
    let Some(obj) = value.as_object() else {
        return Err("el patch tiene que ser un objeto".into());
    };

    // ── Las preferencias del arranque ───────────────────────────────────────
    let mut config_error = None;
    let mut toco_config = false;

    if let Some(mut config) = world.get_resource_mut::<RootConfig>() {
        let mut updated = config.clone();

        if let Some(v) = obj.get("autoLoadHome") {
            match v.as_bool() {
                Some(b) => { updated.auto_load_home = b; toco_config = true; }
                None => config_error = Some("autoLoadHome tiene que ser booleano".to_string()),
            }
        }

        if let Some(v) = obj.get("homeUrl") {
            match v.as_str() {
                // Se acepta cualquier esquema: el navegador es de mundos, no de
                // una lista blanca. Lo único que se rechaza es lo vacío y lo
                // absurdamente largo, que no serían una URL sino un error.
                Some(u) if !u.trim().is_empty() && u.len() <= MAX_URL => {
                    updated.home_url = u.trim().to_string();
                    toco_config = true;
                }
                Some(_) => config_error = Some("homeUrl vacío o demasiado largo".to_string()),
                None => config_error = Some("homeUrl tiene que ser texto".to_string()),
            }
        }

        if let Some(v) = obj.get("renderMode") {
            match v.as_str() {
                Some("desktop") => { updated.preferred_render_mode = PreferredRenderMode::Desktop; toco_config = true; }
                Some("vr") => { updated.preferred_render_mode = PreferredRenderMode::Vr; toco_config = true; }
                _ => config_error = Some("renderMode tiene que ser 'desktop' o 'vr'".to_string()),
            }
        }

        if toco_config {
            match updated.save() {
                Ok(_) => *config = updated,
                // No se aplica en memoria si no se pudo guardar: mostrar una
                // preferencia que se va a perder al reiniciar es peor que no
                // aceptarla.
                Err(e) => config_error = Some(format!("no se pudo guardar la configuración: {e}")),
            }
        }
    }

    // ── Los permisos por sitio ──────────────────────────────────────────────
    let mut permiso_error = None;

    if let Some(p) = obj.get("permission") {
        let origin = p.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let key = p.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let decision = p.get("decision").and_then(|v| v.as_str()).unwrap_or("");

        match (origin.is_empty(), elevated_capability_from_key(key)) {
            (true, _) => permiso_error = Some("permission.origin vacío".to_string()),
            (_, None) => permiso_error = Some(format!("permission.key desconocida: {key}")),
            (false, Some(capability)) => {
                let cambio = world
                    .get_resource_mut::<PermissionDecisionStore>()
                    .map(|mut decisions| match decision {
                        "allow" => decisions.set_decision(origin.clone(), capability, PermissionDecision::Allow),
                        "deny" => decisions.set_decision(origin.clone(), capability, PermissionDecision::Deny),
                        // "ask" es olvidar la decisión: la próxima vez vuelve a
                        // preguntar, que no es lo mismo que negar.
                        "ask" => decisions.forget_decision(&origin, capability),
                        _ => false,
                    });

                match cambio {
                    Some(true) => {
                        // Que las políticas se recalculen: sin esto el cambio
                        // no llega a los espacios ya montados.
                        if let Some(mut policies) = world.get_resource_mut::<crate::permissions::SpacePolicies>() {
                            policies.dirty = true;
                        }
                        let guardado = world
                            .get_resource::<PermissionDecisionStore>()
                            .map(|d| d.save());
                        if let Some(Err(e)) = guardado {
                            permiso_error = Some(format!(
                                "{} de {} vale para esta sesión pero no se pudo guardar: {e}",
                                describe_capability_bits(capability),
                                origin
                            ));
                        }
                    }
                    Some(false) => {
                        permiso_error = Some(format!("permission.decision inválida: {decision}"));
                    }
                    None => {}
                }
            }
        }
    }

    match (config_error, permiso_error) {
        (Some(a), Some(b)) => Err(format!("{a}; {b}")),
        (Some(a), None) | (None, Some(a)) => Err(a),
        (None, None) => Ok(()),
    }
}

/// El JSON que el host publica en el documento de ajustes.
///
/// Va como texto en un atributo porque es el mismo camino que ya usa el estado
/// del MCP: el host no puede llamar a una función del isolate, sólo escribir
/// atributos e inyectar scripts. Ver `agent.rs`.
pub fn publish(
    config: &RootConfig,
    decisions: &PermissionDecisionStore,
    root_url: &str,
) -> String {
    let permisos: Vec<serde_json::Value> = decisions
        .entries()
        .into_iter()
        .map(|(origin, capability, decision)| {
            serde_json::json!({
                "origin": origin,
                "capability": describe_capability_bits(capability),
                "key": elevated_capability_key(capability).unwrap_or(""),
                "decision": match decision {
                    PermissionDecision::Allow => "allow",
                    PermissionDecision::Deny => "deny",
                },
            })
        })
        .collect();

    serde_json::json!({
        "autoLoadHome": config.auto_load_home,
        "homeUrl": config.home_url,
        "renderMode": match config.preferred_render_mode {
            PreferredRenderMode::Desktop => "desktop",
            PreferredRenderMode::Vr => "vr",
        },
        "configPath": RootConfig::path().display().to_string(),
        "rootUrl": root_url,
        "permissions": permisos,
    })
    .to_string()
}
