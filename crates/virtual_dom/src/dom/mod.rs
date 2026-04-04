

pub mod hsml;
pub mod element;
pub mod tags;
use std::sync::atomic::{AtomicU32, Ordering};

// Contador estático global para los id de los nodos.
static NODE_COUNTER: AtomicU32 = AtomicU32::new(1);


pub fn create_new_id()-> u32{
    NODE_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub static TRANSFORM_POSITION: &[&'static str] = &["x", "y", "z"];
pub static TRANSFORM_ROTATION: &[&'static str] = &["rx", "ry", "rz"];
pub static TRANSFORM_SCALE: &[&'static str] = &["sx", "sy", "sz"];

