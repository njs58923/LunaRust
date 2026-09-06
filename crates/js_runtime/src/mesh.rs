//! Validated, bounded mesh uploads. Geometry crosses the isolate as binary buffers.
use deno_core::{op2, OpState};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

pub const MAX_VERTICES: usize = 262_144;
pub const MAX_INDICES: usize = 786_432;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_RESOURCES: usize = 128;

#[derive(Debug)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 4]>,
}

pub enum MeshCommand {
    Upload(String, MeshData),
    Dispose(String),
}

pub struct MeshQueue {
    pub scope: u64,
    next: u64,
    sizes: HashMap<String, usize>,
    pending: HashMap<String, MeshCommand>,
}
impl Default for MeshQueue {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            scope: NEXT.fetch_add(1, Ordering::Relaxed),
            next: 0,
            sizes: HashMap::new(),
            pending: HashMap::new(),
        }
    }
}
impl MeshQueue {
    pub fn drain(&mut self) -> (u64, Vec<MeshCommand>) {
        (self.scope, self.pending.drain().map(|(_, c)| c).collect())
    }
    fn submit(&mut self, action: &str, src: &str, buffers: [&[u8]; 5]) -> Result<String, String> {
        if action != "create" && !self.sizes.contains_key(src) {
            return Err("Unknown or disposed mesh in this isolate".into());
        }
        if action == "dispose" {
            self.sizes.remove(src);
            self.pending
                .insert(src.into(), MeshCommand::Dispose(src.into()));
            return Ok(src.into());
        }
        if !matches!(action, "create" | "update") {
            return Err("Unknown mesh operation".into());
        }
        if action == "create"
            && (self.sizes.len() >= MAX_RESOURCES || self.pending.len() >= MAX_RESOURCES * 2)
        {
            return Err("Mesh resource/queue limit reached".into());
        }
        // Account for generated normals and implicit indices as well as uploads.
        let bytes: usize = buffers.iter().map(|b| b.len()).sum::<usize>()
            + if buffers[2].is_empty() {
                buffers[0].len()
            } else {
                0
            }
            + if buffers[1].is_empty() {
                buffers[0].len() / 3
            } else {
                0
            };
        if bytes > MAX_BYTES
            || self.sizes.values().sum::<usize>() - self.sizes.get(src).copied().unwrap_or(0)
                + bytes
                > MAX_BYTES
        {
            return Err("Mesh memory quota exceeded".into());
        }
        let data = decode(buffers)?;
        let src = if action == "create" {
            self.next += 1;
            format!("mesh://{}/{}", self.scope, self.next)
        } else {
            src.into()
        };
        self.sizes.insert(src.clone(), bytes);
        self.pending
            .insert(src.clone(), MeshCommand::Upload(src.clone(), data));
        Ok(src)
    }
}

fn vectors<const N: usize>(bytes: &[u8]) -> Result<Vec<[f32; N]>, String> {
    if bytes.len() % (N * 4) != 0 {
        return Err("Invalid mesh attribute length".into());
    }
    bytes
        .chunks_exact(N * 4)
        .map(|chunk| {
            let v = std::array::from_fn(|i| {
                f32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap())
            });
            if v.iter().any(|n| !n.is_finite() || n.abs() > 1.0e6) {
                return Err("Mesh attributes must be finite and bounded".into());
            }
            Ok(v)
        })
        .collect()
}

pub fn decode([positions, indices, normals, uvs, colors]: [&[u8]; 5]) -> Result<MeshData, String> {
    if positions.is_empty()
        || positions.len() > MAX_VERTICES * 12
        || indices.len() > MAX_INDICES * 4
    {
        return Err("Mesh vertex/index limit exceeded or positions empty".into());
    }
    let positions = vectors::<3>(positions)?;
    let count = positions.len();
    if indices.len() % 12 != 0 {
        return Err("Indices must describe triangles".into());
    }
    let indices: Vec<u32> = if indices.is_empty() {
        if count % 3 != 0 {
            return Err("Unindexed vertex count must be divisible by 3".into());
        }
        (0..count as u32).collect()
    } else {
        indices
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    };
    if indices.iter().any(|i| *i as usize >= count) {
        return Err("Mesh index out of bounds".into());
    }
    let normals = vectors::<3>(normals)?;
    let uvs = vectors::<2>(uvs)?;
    let colors = vectors::<4>(colors)?;
    if [normals.len(), uvs.len(), colors.len()]
        .iter()
        .any(|n| *n != 0 && *n != count)
    {
        return Err("Optional mesh attributes must match the vertex count".into());
    }
    if colors.iter().flatten().any(|n| !(0.0..=1.0).contains(n)) {
        return Err("Vertex colors must be in [0, 1]".into());
    }
    Ok(MeshData {
        positions,
        indices,
        normals,
        uvs,
        colors,
    })
}

#[op2]
#[serde]
pub fn op_mesh_resource(
    state: &mut OpState,
    #[string] action: &str,
    #[string] src: &str,
    #[buffer] positions: &[u8],
    #[buffer] indices: &[u8],
    #[buffer] normals: &[u8],
    #[buffer] uvs: &[u8],
    #[buffer] colors: &[u8],
) -> serde_json::Value {
    match state.borrow_mut::<MeshQueue>().submit(
        action,
        src,
        [positions, indices, normals, uvs, colors],
    ) {
        Ok(src) => serde_json::json!({"src":src}),
        Err(error) => serde_json::json!({"error":error}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle() -> Vec<u8> {
        [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect()
    }
    #[test]
    fn rejects_bad_geometry_and_bounds_queue() {
        let p = triangle();
        assert!(decode([&p, &[0, 0, 0, 0], &[], &[], &[]]).is_err());
        let out_of_bounds: Vec<u8> = [0u32, 1, 3]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(decode([&p, &out_of_bounds, &[], &[], &[]]).is_err());
        let mut nan = p.clone();
        nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode([&nan, &[], &[], &[], &[]]).is_err());
        assert!(decode([&p, &[], &[0; 12], &[], &[]]).is_err());
        let mut queue = MeshQueue::default();
        for _ in 0..MAX_RESOURCES {
            queue
                .submit("create", "", [&p, &[], &[], &[], &[]])
                .unwrap();
        }
        assert!(queue
            .submit("create", "", [&p, &[], &[], &[], &[]])
            .is_err());
        assert_eq!(queue.drain().1.len(), MAX_RESOURCES);
    }
    #[test]
    fn javascript_typed_upload_copies_and_coalesces_and_revokes() {
        let mut engine = crate::Engine::new();
        engine
            .eval(
                r#"
            const backing = new Float32Array([99, 0,0,0, 1,0,0, 0,1,0, 99]);
            const p = backing.subarray(1, 10);
            const mesh = MeshResource.create({ positions:p });
            p[0] = 2;
            mesh.update({ positions:p });
            p[0] = 99;
        "#,
            )
            .unwrap();
        let (_, mut commands) = engine.drain_mesh_commands();
        assert_eq!(commands.len(), 1);
        let MeshCommand::Upload(src, data) = commands.pop().unwrap() else {
            panic!();
        };
        assert_eq!(data.positions[0][0], 2.0);
        assert_eq!(data.indices, vec![0, 1, 2]);
        let mut other = MeshQueue::default();
        assert!(other
            .submit("update", &src, [&triangle(), &[], &[], &[], &[]])
            .is_err());
        engine.eval("mesh.dispose(); mesh.dispose();").unwrap();
        assert!(matches!(
            &engine.drain_mesh_commands().1[..],
            [MeshCommand::Dispose(_)]
        ));
        assert!(engine.eval("mesh.update({positions:p})").is_err());
    }
}
