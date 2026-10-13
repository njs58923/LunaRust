//! Bounded, typed straight-line pose programs. Parsing is never part of render.
use bevy::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const MAX_REGISTERS: usize = 64;
pub const MAX_INSTRUCTIONS: usize = 128;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub inputs: BTreeMap<String, Input>,
    #[serde(default)]
    pub constants: BTreeMap<String, serde_json::Value>,
    pub code: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Key(String),
    Options(InputOptions),
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InputOptions {
    pub anchor: String,
    #[serde(default)]
    pub hold_last: bool,
}
impl Input {
    pub fn key(&self) -> &str {
        match self {
            Self::Key(k) => k,
            Self::Options(o) => &o.anchor,
        }
    }
    pub fn hold(&self) -> bool {
        matches!(self, Self::Options(o) if o.hold_last)
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ty {
    Pose,
    Vec,
    Quat,
    Float,
    Bool,
}
#[derive(Clone, Copy, Debug)]
enum Value {
    Pose(Transform),
    Vec(Vec3),
    Quat(Quat),
    Float(f32),
    Bool(bool),
    Empty,
}
#[derive(Clone, Copy, Debug)]
struct Ref {
    slot: usize,
    field: u8,
}
#[derive(Clone, Copy, Debug)]
enum Op {
    Copy,
    Point,
    Add,
    Sub,
    Rotate,
    Aim,
    Length,
    Greater,
    Multiply,
    MultiplyQuat,
    InverseQuat,
    Normalize,
    Select,
}
#[derive(Debug)]
struct Instruction {
    op: Op,
    dst: Ref,
    args: Vec<Ref>,
}
#[derive(Debug)]
struct Register {
    name: String,
    ty: Ty,
    writable: bool,
    initialized: bool,
}
#[derive(Debug)]
pub struct Program {
    pub inputs: Vec<Input>,
    slots: Vec<usize>,
    seed: [Value; MAX_REGISTERS],
    instructions: Vec<Instruction>,
    output: usize,
}
fn ty(s: &str) -> Result<Ty, String> {
    match s {
        "pose" => Ok(Ty::Pose),
        "vec3" => Ok(Ty::Vec),
        "quat" => Ok(Ty::Quat),
        "float" => Ok(Ty::Float),
        "bool" => Ok(Ty::Bool),
        _ => Err(format!("Unknown type {s}")),
    }
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
}
fn reference(regs: &[Register], s: &str, write: bool) -> Result<(Ref, Ty), String> {
    let parts: Vec<_> = s.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("Invalid register {s}"));
    }
    let slot = regs
        .iter()
        .position(|r| r.name == parts[0])
        .ok_or_else(|| format!("Unknown register {s}"))?;
    let r = &regs[slot];
    if write && !r.writable {
        return Err(format!("Read-only register {s}"));
    }
    if !write && !r.initialized {
        return Err(format!("Uninitialized register {s}"));
    }
    let (field, t) = if parts.len() == 1 {
        (0, r.ty)
    } else if r.ty == Ty::Pose {
        match parts[1] {
            "position" => (1, Ty::Vec),
            "rotation" => (2, Ty::Quat),
            "scale" => (3, Ty::Vec),
            _ => return Err(format!("Unknown pose field {s}")),
        }
    } else {
        return Err(format!("Not a pose: {s}"));
    };
    // Partial writes require a fully initialized pose, avoiding undefined fields.
    if field != 0 && !r.initialized {
        return Err(format!("Uninitialized pose {s}"));
    }
    Ok((Ref { slot, field }, t))
}
fn constant(t: Ty, v: &serde_json::Value) -> Result<Value, String> {
    let array = |n: usize| -> Result<Vec<f32>, String> {
        let a = v
            .as_array()
            .filter(|a| a.len() == n)
            .ok_or("Wrong constant vector size")?;
        a.iter()
            .map(|x| {
                x.as_f64()
                    .map(|x| x as f32)
                    .filter(|x| x.is_finite())
                    .ok_or("Constant must be finite".into())
            })
            .collect()
    };
    match t {
        Ty::Vec => {
            let a = array(3)?;
            Ok(Value::Vec(Vec3::new(a[0], a[1], a[2])))
        }
        Ty::Quat => {
            let a = array(4)?;
            let q = Quat::from_xyzw(a[0], a[1], a[2], a[3]);
            if !q.length_squared().is_finite() || q.length_squared() < 1e-12 {
                return Err("Zero quaternion".into());
            }
            Ok(Value::Quat(q.normalize()))
        }
        Ty::Float => Ok(Value::Float(
            v.as_f64()
                .map(|x| x as f32)
                .filter(|x| x.is_finite())
                .ok_or("Invalid scalar constant")?,
        )),
        Ty::Bool => Ok(Value::Bool(v.as_bool().ok_or("Invalid bool constant")?)),
        Ty::Pose => Err("Pose constants are not supported; initialize from an input".into()),
    }
}
impl Program {
    pub fn compile(d: &Definition) -> Result<Self, String> {
        if d.kind != "program" || d.version != 1 {
            return Err("Expected program version 1".into());
        }
        if d.code.len() > 16384
            || d.inputs.is_empty()
            || d.inputs.len() > 16
            || d.constants.len() > 32
        {
            return Err("Program definition exceeds limits".into());
        }
        for input in d.inputs.values() {
            if !js_runtime::anchors::valid_key(input.key()) || input.key().starts_with("__") {
                return Err("Program inputs require public anchor keys".into());
            }
        }
        let mut p = Self {
            inputs: vec![],
            slots: vec![],
            seed: [Value::Empty; MAX_REGISTERS],
            instructions: vec![],
            output: usize::MAX,
        };
        let mut regs = Vec::<Register>::new();
        let mut operations_started = false;
        for (line, raw) in d.code.lines().enumerate() {
            let text = raw.split("//").next().unwrap().trim();
            if text.is_empty() {
                continue;
            }
            let result = (|| -> Result<(), String> {
                let tokens: Vec<_> = text.split_whitespace().collect();
                if matches!(tokens[0], "input" | "const" | "temp" | "output") {
                    if operations_started {
                        return Err("Declarations must precede instructions".into());
                    }
                    if tokens.len() < 3 || regs.len() == MAX_REGISTERS {
                        return Err("Invalid declaration or register limit".into());
                    }
                    let t = ty(tokens[1])?;
                    let n = tokens[2];
                    if !name(n) || regs.iter().any(|r| r.name == n) {
                        return Err(format!("Invalid or duplicate register {n}"));
                    }
                    let slot = regs.len();
                    let mut initialized = false;
                    match tokens[0] {
                        "input" => {
                            if tokens.len() != 3 || t != Ty::Pose {
                                return Err("Inputs must be poses".into());
                            }
                            let input = d.inputs.get(n).ok_or("Missing input binding")?;
                            p.inputs.push(input.clone());
                            p.slots.push(slot);
                            initialized = true;
                        }
                        "const" => {
                            if tokens.len() != 3 {
                                return Err("Invalid constant declaration".into());
                            }
                            p.seed[slot] =
                                constant(t, d.constants.get(n).ok_or("Missing constant")?)?;
                            initialized = true;
                        }
                        "temp" => {
                            if tokens.len() != 3 {
                                return Err("Invalid temporary declaration".into());
                            }
                        }
                        "output" => {
                            if tokens.len() != 5
                                || tokens[3] != "="
                                || t != Ty::Pose
                                || p.output != usize::MAX
                            {
                                return Err(
                                    "One output pose initialized from an input is required".into(),
                                );
                            }
                            let (src, st) = reference(&regs, tokens[4], false)?;
                            if st != Ty::Pose || !p.slots.contains(&src.slot) {
                                return Err("Output must initialize from an input pose".into());
                            }
                            p.output = slot;
                            initialized = true;
                            p.instructions.push(Instruction {
                                op: Op::Copy,
                                dst: Ref { slot, field: 0 },
                                args: vec![src],
                            });
                        }
                        _ => unreachable!(),
                    }
                    regs.push(Register {
                        name: n.into(),
                        ty: t,
                        writable: matches!(tokens[0], "temp" | "output"),
                        initialized,
                    });
                    return Ok(());
                }
                operations_started = true;
                if p.instructions.len() >= MAX_INSTRUCTIONS {
                    return Err("Instruction limit exceeded".into());
                }
                let (op, count) = match tokens[0] {
                    "copy" => (Op::Copy, 1),
                    "transformPoint" => (Op::Point, 2),
                    "add" => (Op::Add, 2),
                    "subtract" => (Op::Sub, 2),
                    "rotate" => (Op::Rotate, 2),
                    "aimRotation" => (Op::Aim, 4),
                    "length" => (Op::Length, 1),
                    "greater" => (Op::Greater, 2),
                    "multiply" => (Op::Multiply, 2),
                    "multiplyQuat" => (Op::MultiplyQuat, 2),
                    "inverseQuat" => (Op::InverseQuat, 1),
                    "normalize" => (Op::Normalize, 1),
                    "select" => (Op::Select, 3),
                    _ => return Err(format!("Unknown instruction {}", tokens[0])),
                };
                let body = text[tokens[0].len()..].trim();
                let names: Vec<_> = body.split(',').map(str::trim).collect();
                if names.len() != count + 1 {
                    return Err("Wrong operand count".into());
                }
                let (dst, dt) = reference(&regs, names[0], true)?;
                let mut args = vec![];
                let mut types = vec![];
                for n in &names[1..] {
                    let (r, t) = reference(&regs, n, false)?;
                    args.push(r);
                    types.push(t);
                }
                let valid = match op {
                    Op::Copy => types == [dt],
                    Op::Point => dt == Ty::Vec && types == [Ty::Pose, Ty::Vec],
                    Op::Add | Op::Sub => dt == Ty::Vec && types == [Ty::Vec, Ty::Vec],
                    Op::Rotate => dt == Ty::Vec && types == [Ty::Quat, Ty::Vec],
                    Op::Aim => dt == Ty::Quat && types == [Ty::Vec; 4],
                    Op::Length => dt == Ty::Float && types == [Ty::Vec],
                    Op::Greater => dt == Ty::Bool && types == [Ty::Float; 2],
                    Op::Multiply => {
                        (dt == Ty::Float && types == [Ty::Float; 2])
                            || (dt == Ty::Vec && types == [Ty::Vec, Ty::Float])
                    }
                    Op::MultiplyQuat => dt == Ty::Quat && types == [Ty::Quat; 2],
                    Op::InverseQuat => dt == Ty::Quat && types == [Ty::Quat],
                    Op::Normalize => dt == Ty::Vec && types == [Ty::Vec],
                    Op::Select => types == [Ty::Bool, dt, dt],
                };
                if !valid {
                    return Err("Operand type mismatch".into());
                }
                regs[dst.slot].initialized = true;
                p.instructions.push(Instruction { op, dst, args });
                Ok(())
            })();
            result.map_err(|e| format!("line {}: {e}", line + 1))?;
        }
        if p.output == usize::MAX
            || p.inputs.len() != d.inputs.len()
            || regs
                .iter()
                .filter(|r| !r.writable && r.ty != Ty::Pose)
                .count()
                != d.constants.len()
        {
            return Err("Missing output or undeclared input/constant".into());
        }
        Ok(p)
    }
    pub fn run(&self, inputs: &[GlobalTransform]) -> Option<GlobalTransform> {
        if inputs.len() != self.slots.len() {
            return None;
        }
        let mut regs = self.seed;
        for (&slot, pose) in self.slots.iter().zip(inputs) {
            regs[slot] = Value::Pose(pose.compute_transform());
        }
        for i in &self.instructions {
            let get = |n: usize| read(&regs, i.args[n]);
            let v = match i.op {
                Op::Copy => get(0),
                Op::Point => Value::Vec(pose(get(0)).transform_point(vec(get(1)))),
                Op::Add => Value::Vec(vec(get(0)) + vec(get(1))),
                Op::Sub => Value::Vec(vec(get(0)) - vec(get(1))),
                Op::Rotate => Value::Vec(quat(get(0)) * vec(get(1))),
                Op::Aim => Value::Quat(
                    look(vec(get(0)), vec(get(1))) * look(vec(get(2)), vec(get(3))).inverse(),
                ),
                Op::Length => Value::Float(vec(get(0)).length()),
                Op::Greater => Value::Bool(float(get(0)) > float(get(1))),
                Op::Multiply => match get(0) {
                    Value::Vec(v) => Value::Vec(v * float(get(1))),
                    v => Value::Float(float(v) * float(get(1))),
                },
                Op::MultiplyQuat => Value::Quat(quat(get(0)) * quat(get(1))),
                Op::InverseQuat => Value::Quat(quat(get(0)).inverse()),
                Op::Normalize => Value::Vec(vec(get(0)).normalize_or_zero()),
                Op::Select => {
                    if matches!(get(0), Value::Bool(true)) {
                        get(1)
                    } else {
                        get(2)
                    }
                }
            };
            if !finite(v) {
                return None;
            }
            write(&mut regs, i.dst, v);
        }
        let t = pose(regs[self.output]);
        if t.rotation.length_squared() < 1e-12 {
            return None;
        }
        Some(GlobalTransform::from(Transform {
            rotation: t.rotation.normalize(),
            ..t
        }))
    }
    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }
}
fn read(r: &[Value; MAX_REGISTERS], a: Ref) -> Value {
    match a.field {
        0 => r[a.slot],
        1 => Value::Vec(pose(r[a.slot]).translation),
        2 => Value::Quat(pose(r[a.slot]).rotation),
        3 => Value::Vec(pose(r[a.slot]).scale),
        _ => unreachable!(),
    }
}
fn write(r: &mut [Value; MAX_REGISTERS], a: Ref, v: Value) {
    if a.field == 0 {
        r[a.slot] = v;
    } else {
        let mut p = pose(r[a.slot]);
        match a.field {
            1 => p.translation = vec(v),
            2 => p.rotation = quat(v),
            3 => p.scale = vec(v),
            _ => unreachable!(),
        };
        r[a.slot] = Value::Pose(p);
    }
}
fn pose(v: Value) -> Transform {
    if let Value::Pose(v) = v {
        v
    } else {
        unreachable!()
    }
}
fn vec(v: Value) -> Vec3 {
    if let Value::Vec(v) = v {
        v
    } else {
        unreachable!()
    }
}
fn quat(v: Value) -> Quat {
    if let Value::Quat(v) = v {
        v
    } else {
        unreachable!()
    }
}
fn float(v: Value) -> f32 {
    if let Value::Float(v) = v {
        v
    } else {
        unreachable!()
    }
}
fn finite(v: Value) -> bool {
    match v {
        Value::Vec(v) => v.is_finite(),
        Value::Quat(q) => {
            q.is_finite() && q.length_squared().is_finite() && q.length_squared() > 1e-12
        }
        Value::Float(v) => v.is_finite(),
        Value::Pose(t) => {
            t.translation.is_finite()
                && t.scale.is_finite()
                && t.rotation.is_finite()
                && t.rotation.length_squared().is_finite()
                && t.rotation.length_squared() > 1e-12
        }
        Value::Bool(_) => true,
        Value::Empty => false,
    }
}
/// Same -Z/+Y convention and degenerate-up policy as comun/algebra.qMirando.
fn look(direction: Vec3, up: Vec3) -> Quat {
    if !direction.length_squared().is_finite() || !up.length_squared().is_finite() {
        return Quat::from_array([f32::NAN; 4]);
    }
    if direction.length_squared() < 1e-12 {
        return Quat::IDENTITY;
    }
    let z = -direction.normalize();
    let mut y = up - z * up.dot(z);
    if y.length() < 1e-3 {
        y = Vec3::Y - z * Vec3::Y.dot(z);
    }
    if y.length() < 1e-3 {
        y = Vec3::NEG_Z - z * Vec3::NEG_Z.dot(z);
    }
    y = y.normalize();
    Quat::from_mat3(&Mat3::from_cols(y.cross(z), y, z)).normalize()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> Definition {
        serde_json::from_value(serde_json::json!({"type":"program","version":1,"inputs":{"a":"left-grip","b":"right-grip"},"constants":{"offset":[0.,0.05,0.],"up":[0.,1.,0.],"forward":[0.,0.,-1.]},"code":"input pose a\ninput pose b\nconst vec3 offset\nconst vec3 up\nconst vec3 forward\ntemp vec3 target\ntemp vec3 direction\ntemp quat rotation\noutput pose result = a\ntransformPoint target, b, offset\nsubtract direction, target, a.position\naimRotation rotation, direction, up, forward, up\ncopy result.rotation, rotation"})).unwrap()
    }
    #[test]
    fn local_target_rotates_with_source_and_output_is_fresh() {
        let p = Program::compile(&definition()).unwrap();
        let a = GlobalTransform::IDENTITY;
        let b = GlobalTransform::from(Transform {
            translation: Vec3::new(0., 0., -1.),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ..default()
        });
        let out = p.run(&[a, b]).unwrap();
        let d = out.compute_transform().rotation * Vec3::NEG_Z;
        assert!((d - Vec3::new(-0.05, 0., -1.).normalize()).length() < 1e-5);
        assert_eq!(
            p.run(&[a, GlobalTransform::from_translation(Vec3::NEG_Z)])
                .unwrap()
                .translation(),
            Vec3::ZERO
        );
    }
    #[test]
    fn strict_compile_rejects_invalid_programs() {
        for code in ["input pose a\ninput pose b\noutput pose result = a\ncopy a.position, b.position","input pose a\ninput pose b\ntemp vec3 v\noutput pose result = a\ncopy result.position, v","input pose a\ninput pose b\noutput pose result = a\ncopy result.rotation, b.position","input pose a\ninput pose b\noutput pose result = a\nunknown result, a"]{let mut d=definition();d.constants.clear();d.code=code.into();assert!(Program::compile(&d).is_err());}
        let mut d = definition();
        d.code.push_str(&"\ncopy result, a".repeat(129));
        assert!(Program::compile(&d).is_err());
    }
    #[test]
    fn numerical_overflow_cannot_publish_a_corrupt_pose() {
        let mut d = definition();
        d.constants
            .insert("offset".into(), serde_json::json!([1e30, 1e30, 1e30]));
        let p = Program::compile(&d).unwrap();
        assert!(p
            .run(&[GlobalTransform::IDENTITY, GlobalTransform::IDENTITY])
            .is_none());
        d.constants
            .insert("q".into(), serde_json::json!([1e30, 0, 0, 1e30]));
        d.code = d
            .code
            .replace("const vec3 offset", "const quat q\nconst vec3 offset");
        assert!(Program::compile(&d).is_err());
    }
    #[test]
    fn bounded_program_has_small_execution_cost() {
        let p = Program::compile(&definition()).unwrap();
        let now = std::time::Instant::now();
        let inputs = [
            GlobalTransform::IDENTITY,
            GlobalTransform::from_translation(Vec3::NEG_Z),
        ];
        for _ in 0..100000 {
            std::hint::black_box(p.run(std::hint::black_box(&inputs)));
        }
        eprintln!(
            "motion program: {} ops, {:?}/100000 evaluations",
            p.instruction_count(),
            now.elapsed()
        );
    }

    #[test]
    fn actual_tiro_program_matches_calibrated_two_hand_solver_and_near_fallback() {
        let d: Definition =
            serde_json::from_str(include_str!("../tests/fixtures/tiro_motion.json")).unwrap();
        let p = Program::compile(&d).unwrap();
        let v = |key: &str| vec(constant(Ty::Vec, &d.constants[key]).unwrap());
        let mount = quat(constant(Ty::Quat, &d.constants["mountRotation"]).unwrap());
        let mut seed = 8273u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed as f32 / u32::MAX as f32
        };
        for n in 0..4000 {
            let a = Transform {
                translation: Vec3::new(random(), 1. + random(), -random()),
                rotation: Quat::from_euler(
                    EulerRot::XYZ,
                    random() * 6.,
                    random() * 6.,
                    random() * 6.,
                ),
                ..default()
            };
            let delta = if n % 20 == 0 {
                Vec3::ZERO
            } else if n % 20 == 1 {
                Vec3::X * 0.01
            } else {
                Vec3::new(random() - 0.5, random() * 0.5 + 0.1, -random() * 0.5 - 0.2)
            };
            let b = Transform::from_translation(a.translation + delta);
            let base = if delta.length() > 0.05 {
                look(delta, a.rotation * v("controllerUp"))
                    * look(v("forward"), v("localUp")).inverse()
            } else {
                a.rotation * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
            };
            let expected = Transform {
                translation: a.translation + base * v("mountPosition"),
                rotation: base * mount,
                ..default()
            };
            let output = p
                .run(&[GlobalTransform::from(a), GlobalTransform::from(b)])
                .unwrap()
                .compute_transform();
            assert!((output.translation - expected.translation).length() < 1e-5);
            assert!(output.rotation.dot(expected.rotation).abs() > 0.99999);
        }
        let inputs = [
            GlobalTransform::IDENTITY,
            GlobalTransform::from_translation(Vec3::NEG_Z),
        ];
        let start = std::time::Instant::now();
        for _ in 0..100000 {
            std::hint::black_box(p.run(std::hint::black_box(&inputs)));
        }
        eprintln!(
            "tiro motion program: {} ops, {:?}/100000 evaluations",
            p.instruction_count(),
            start.elapsed()
        );
    }
}
