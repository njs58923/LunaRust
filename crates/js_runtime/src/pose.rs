//! Binary local poses. Independent of transport so a native stream can reuse them.
use deno_core::{op2, OpState};

pub const MAX_JOINTS: usize = 4096;
pub const MAX_QUEUED_JOINTS: usize = 65536;

#[derive(Clone, Debug)]
pub struct JointPose {
    pub index: usize,
    pub rotation: [f32; 4],
    pub translation: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
}

#[derive(Debug)]
pub struct PoseBatch {
    pub node: i32,
    pub binding: String,
    pub joints: Vec<JointPose>,
}

#[derive(Default)]
pub struct PoseQueue {
    pending: Vec<PoseBatch>,
    count: usize,
}
impl PoseQueue {
    pub fn drain(&mut self) -> Vec<PoseBatch> {
        self.count = 0;
        std::mem::take(&mut self.pending)
    }
}

pub fn decode(bytes: &[u8], trs: bool) -> Result<Vec<JointPose>, anyhow::Error> {
    let stride = if trs { 11 } else { 5 };
    anyhow::ensure!(
        bytes.len() % (stride * 4) == 0,
        "Invalid joint batch length"
    );
    anyhow::ensure!(
        bytes.len() / (stride * 4) <= MAX_JOINTS,
        "Joint batch exceeds 4096 entries"
    );
    let mut out = Vec::with_capacity(bytes.len() / (stride * 4));
    for row in bytes.chunks_exact(stride * 4) {
        let mut v = [0.0f32; 11];
        for (i, b) in row.chunks_exact(4).enumerate() {
            v[i] = f32::from_ne_bytes(b.try_into().unwrap());
            anyhow::ensure!(v[i].is_finite(), "Joint pose must be finite");
        }
        anyhow::ensure!(
            v[0] >= 0.0 && v[0] < MAX_JOINTS as f32 && v[0].fract() == 0.0,
            "Invalid joint index"
        );
        let q = if trs { &v[4..8] } else { &v[1..5] };
        let norm = q.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
        anyhow::ensure!(norm > 1e-12, "Quaternion must be nonzero");
        out.push(JointPose {
            index: v[0] as usize,
            rotation: std::array::from_fn(|i| (q[i] as f64 / norm) as f32),
            translation: trs.then(|| [v[1], v[2], v[3]]),
            scale: trs.then(|| [v[8], v[9], v[10]]),
        });
    }
    Ok(out)
}

#[op2(fast)]
pub fn op_set_joint_batch(
    state: &mut OpState,
    #[smi] node: i32,
    #[string] binding: String,
    #[buffer] bytes: &[u8],
    trs: bool,
) -> Result<(), anyhow::Error> {
    anyhow::ensure!(
        node > 0 && !binding.is_empty() && binding.len() <= 128,
        "Invalid model/binding"
    );
    let stride = if trs { 44 } else { 20 };
    let queue = state.borrow_mut::<PoseQueue>();
    anyhow::ensure!(
        queue.count + bytes.len() / stride <= MAX_QUEUED_JOINTS,
        "Joint queue quota exceeded"
    );
    let joints = decode(bytes, trs)?;
    if !joints.is_empty() {
        queue.count += joints.len();
        queue.pending.push(PoseBatch {
            node,
            binding,
            joints,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bytes(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_ne_bytes()).collect()
    }
    #[test]
    fn validates_and_normalizes_atomic_batches() {
        let p = decode(&bytes(&[2., 0., 0., 0., 2.]), false).unwrap();
        assert_eq!(p[0].rotation, [0., 0., 0., 1.]);
        assert!(p[0].translation.is_none());
        for v in [
            vec![0., 0., 0., 0., 0.],
            vec![0.5, 0., 0., 0., 1.],
            vec![0., f32::NAN, 0., 0., 1.],
            vec![4096., 0., 0., 0., 1.],
        ] {
            assert!(decode(&bytes(&v), false).is_err());
        }
        assert!(decode(&[0; 19], false).is_err());
        let p = decode(&bytes(&[0., 1., 2., 3., 0., 0., 0., 1., 2., 3., 4.]), true).unwrap();
        assert_eq!(p[0].translation, Some([1., 2., 3.]));
        assert_eq!(p[0].scale, Some([2., 3., 4.]));
    }
    #[test]
    fn javascript_buffers_copy_subviews_and_validate() {
        let mut engine = crate::Engine::new();
        engine
            .eval(
                r#"
            const root = hiperspace.dimention;
            const backing = new Float32Array([99, 1, 0, 0, 0, 2, 99]);
            root.setJointBatch(1, backing.subarray(1,6), {binding:'pose:1:1'});
            backing[2] = 99;
            root.setJointBatch(1, [2, 0, 0, 0, 1], {binding:'pose:1:1'});
        "#,
            )
            .unwrap();
        let batches = engine.drain_pose_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].joints[0].rotation, [0., 0., 0., 1.]);
        assert_eq!(batches[1].joints[0].index, 2);
        for code in [
            "root.setJointBatch(1, [0,0,0,0,1], {})",
            "root.setJointBatch(1, [0,0,0,0,0], {binding:'x'})",
            "root.setJointBatch(1, [0,0,0,0,1], {binding:'x',format:'oops'})",
            "root.setJointBatch(1, new Uint8Array(5), {binding:'x'})",
            "root.setJointBatch(-1, [], {binding:'x'})",
        ] {
            assert!(engine.eval(code).is_err(), "{code}");
        }
        assert!(engine.drain_pose_batches().is_empty());
    }
    #[test]
    fn javascript_queue_quota_is_released_on_drain() {
        let mut engine = crate::Engine::new();
        engine
            .eval(
                r#"
            const root = hiperspace.dimention;
            const pose = new Float32Array(4096 * 5);
            for (let i = 0; i < 4096; i++) { pose[i*5] = i; pose[i*5+4] = 1; }
            for (let i = 0; i < 16; i++) root.setJointBatch(1, pose, {binding:'x'});
        "#,
            )
            .unwrap();
        assert!(engine
            .eval("root.setJointBatch(1, pose, {binding:'x'})")
            .is_err());
        assert_eq!(engine.drain_pose_batches().len(), 16);
        engine
            .eval("root.setJointBatch(1, pose, {binding:'x'})")
            .unwrap();
        assert_eq!(engine.drain_pose_batches().len(), 1);
    }
}
