//! El buzón de ajustes: lo que el host publica hacia el isolate.
//!
//! Hasta acá el host le contaba el estado al documento de ajustes **inyectando
//! JavaScript** que le escribía atributos a nodos de ids fijos. Funcionaba y era
//! un rodeo: tres conversiones (Rust → JSON → literal de JS → atributo → parse)
//! donde no hacía falta ninguna, un id acordado a mano entre `agent.rs` y el
//! documento, y una falla que no se nota — si alguien borraba el nodo, la
//! página simplemente no se enteraba nunca.
//!
//! Esto es el mismo mecanismo que ya usan `fetch`, las capturas, el pose y el
//! hover: el host deja el dato en una cola del `OpState` y el isolate lo saca
//! con un op. La diferencia con esos es que acá **no es una cola sino el último
//! valor**: la configuración no es una secuencia de eventos que haya que
//! consumir en orden, es un estado del que sólo importa el actual.
//!
//! La revisión es lo que evita copiar el JSON en cada cuadro. El isolate pasa
//! la que ya vio; si no cambió, la respuesta viene vacía.
use deno_core::{op2, OpState};
use serde::Serialize;

#[derive(Default)]
pub struct SettingsInbox {
    json: String,
    revision: u32,
}

impl SettingsInbox {
    /// Publicar. Sólo mueve la revisión si el contenido cambió de verdad, así
    /// el isolate no se despierta a releer lo mismo dos veces por segundo.
    pub fn publish(&mut self, json: String) {
        if self.json == json {
            return;
        }
        self.json = json;
        // La revisión 0 significa "todavía no publicó nadie", así que se saltea
        // al dar la vuelta.
        self.revision = match self.revision.wrapping_add(1) {
            0 => 1,
            n => n,
        };
    }
}

#[derive(Serialize)]
pub struct SettingsState {
    revision: u32,
    /// Vacío cuando no hay nada nuevo respecto de la revisión que trajo el
    /// llamador. No es `Option` para que el lado JS tenga una sola forma.
    json: String,
}

#[op2]
#[serde]
pub fn op_settings_read(state: &mut OpState, known: u32) -> SettingsState {
    let inbox = state.borrow::<SettingsInbox>();
    if inbox.revision == known || inbox.revision == 0 {
        return SettingsState {
            revision: inbox.revision,
            json: String::new(),
        };
    }
    SettingsState {
        revision: inbox.revision,
        json: inbox.json.clone(),
    }
}
