use std::hint::black_box;

pub const VM_REGS: usize = 32;
pub const VM_INPUTS: usize = 16;
pub const VM_STATE: usize = 16;
pub const VM_OUTPUTS: usize = 16;
pub const VM_MAX_STEPS: usize = 256;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
  Halt = 0,
  LoadInput,
  LoadState,
  StoreState,
  StoreOutput,
  ConstF32,
  Add,
  Sub,
  Mul,
  Min,
  Max,
  Clamp01,
  Sin,
  Cos,
  SelectGt,
}

#[derive(Debug, Clone, Copy)]
pub struct Instr {
  pub op: OpCode,
  pub a: u8,
  pub b: u8,
  pub c: u8,
  pub imm: f32,
}

impl Instr {
  pub const fn halt() -> Self {
      Self { op: OpCode::Halt, a: 0, b: 0, c: 0, imm: 0.0 }
  }

  pub const fn load_input(dst: u8, input_idx: u8) -> Self {
      Self { op: OpCode::LoadInput, a: dst, b: input_idx, c: 0, imm: 0.0 }
  }

  pub const fn load_state(dst: u8, state_idx: u8) -> Self {
      Self { op: OpCode::LoadState, a: dst, b: state_idx, c: 0, imm: 0.0 }
  }

  pub const fn store_state(src: u8, state_idx: u8) -> Self {
      Self { op: OpCode::StoreState, a: src, b: state_idx, c: 0, imm: 0.0 }
  }

  pub const fn store_output(src: u8, output_idx: u8) -> Self {
      Self { op: OpCode::StoreOutput, a: src, b: output_idx, c: 0, imm: 0.0 }
  }

  pub const fn const_f32(dst: u8, value: f32) -> Self {
      Self { op: OpCode::ConstF32, a: dst, b: 0, c: 0, imm: value }
  }

  pub const fn add(dst: u8, lhs: u8, rhs: u8) -> Self {
      Self { op: OpCode::Add, a: dst, b: lhs, c: rhs, imm: 0.0 }
  }

  pub const fn sub(dst: u8, lhs: u8, rhs: u8) -> Self {
      Self { op: OpCode::Sub, a: dst, b: lhs, c: rhs, imm: 0.0 }
  }

  pub const fn mul(dst: u8, lhs: u8, rhs: u8) -> Self {
      Self { op: OpCode::Mul, a: dst, b: lhs, c: rhs, imm: 0.0 }
  }

  pub const fn min(dst: u8, lhs: u8, rhs: u8) -> Self {
      Self { op: OpCode::Min, a: dst, b: lhs, c: rhs, imm: 0.0 }
  }

  pub const fn max(dst: u8, lhs: u8, rhs: u8) -> Self {
      Self { op: OpCode::Max, a: dst, b: lhs, c: rhs, imm: 0.0 }
  }

  pub const fn clamp01(dst: u8, src: u8) -> Self {
      Self { op: OpCode::Clamp01, a: dst, b: src, c: 0, imm: 0.0 }
  }

  pub const fn sin(dst: u8, src: u8) -> Self {
      Self { op: OpCode::Sin, a: dst, b: src, c: 0, imm: 0.0 }
  }

  pub const fn cos(dst: u8, src: u8) -> Self {
      Self { op: OpCode::Cos, a: dst, b: src, c: 0, imm: 0.0 }
  }

  /// if reg[cond] > 0.0 { reg[if_true] } else { reg[if_false] }
  pub const fn select_gt(dst: u8, cond: u8, if_true: u8, if_false_imm_hack: f32) -> Self {
      Self { op: OpCode::SelectGt, a: dst, b: cond, c: if_true, imm: if_false_imm_hack }
  }
}

#[derive(Debug, Clone)]
pub struct VmProgram {
  pub instructions: Box<[Instr]>,
  pub max_steps: u16,
}

impl VmProgram {
  pub fn validate(&self) -> Result<(), String> {
      if self.instructions.is_empty() {
          return Err("program has no instructions".into());
      }
      if self.instructions.len() > VM_MAX_STEPS {
          return Err(format!("program too long: {}", self.instructions.len()));
      }

      for (i, ins) in self.instructions.iter().enumerate() {
          let reg_ok = |r: u8| (r as usize) < VM_REGS;
          let inp_ok = |x: u8| (x as usize) < VM_INPUTS;
          let state_ok = |x: u8| (x as usize) < VM_STATE;
          let out_ok = |x: u8| (x as usize) < VM_OUTPUTS;

          match ins.op {
              OpCode::Halt => {}
              OpCode::LoadInput => {
                  if !reg_ok(ins.a) || !inp_ok(ins.b) {
                      return Err(format!("invalid LoadInput at {}", i));
                  }
              }
              OpCode::LoadState => {
                  if !reg_ok(ins.a) || !state_ok(ins.b) {
                      return Err(format!("invalid LoadState at {}", i));
                  }
              }
              OpCode::StoreState => {
                  if !reg_ok(ins.a) || !state_ok(ins.b) {
                      return Err(format!("invalid StoreState at {}", i));
                  }
              }
              OpCode::StoreOutput => {
                  if !reg_ok(ins.a) || !out_ok(ins.b) {
                      return Err(format!("invalid StoreOutput at {}", i));
                  }
              }
              OpCode::ConstF32 => {
                  if !reg_ok(ins.a) {
                      return Err(format!("invalid ConstF32 at {}", i));
                  }
              }
              OpCode::Add
              | OpCode::Sub
              | OpCode::Mul
              | OpCode::Min
              | OpCode::Max => {
                  if !reg_ok(ins.a) || !reg_ok(ins.b) || !reg_ok(ins.c) {
                      return Err(format!("invalid tri-reg op at {}", i));
                  }
              }
              OpCode::Clamp01 | OpCode::Sin | OpCode::Cos => {
                  if !reg_ok(ins.a) || !reg_ok(ins.b) {
                      return Err(format!("invalid unary op at {}", i));
                  }
              }
              OpCode::SelectGt => {
                  if !reg_ok(ins.a) || !reg_ok(ins.b) || !reg_ok(ins.c) {
                      return Err(format!("invalid SelectGt at {}", i));
                  }
              }
          }
      }

      Ok(())
  }
}

#[derive(Debug, Clone, Copy)]
pub struct VmContext {
  pub regs: [f32; VM_REGS],
}

impl Default for VmContext {
  fn default() -> Self {
      Self { regs: [0.0; VM_REGS] }
  }
}

#[derive(Debug, Clone, Copy)]
pub struct AvatarRawInput {
  pub values: [f32; VM_INPUTS],
}

impl Default for AvatarRawInput {
  fn default() -> Self {
      Self { values: [0.0; VM_INPUTS] }
  }
}

#[derive(Debug, Clone, Copy)]
pub struct AvatarReplicatedState {
  pub values: [f32; VM_STATE],
}

impl Default for AvatarReplicatedState {
  fn default() -> Self {
      Self { values: [0.0; VM_STATE] }
  }
}

#[derive(Debug, Clone, Copy)]
pub struct AvatarAnimOutput {
  pub values: [f32; VM_OUTPUTS],
}

impl Default for AvatarAnimOutput {
  fn default() -> Self {
      Self { values: [0.0; VM_OUTPUTS] }
  }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VmExecStats {
  pub steps_executed: u16,
  pub halted: bool,
}

#[inline(always)]
pub fn run_input_program(
  program: &VmProgram,
  ctx: &mut VmContext,
  input: &AvatarRawInput,
  state: &mut AvatarReplicatedState,
) -> VmExecStats {
  run_vm(program, ctx, &input.values, &mut state.values, None)
}

#[inline(always)]
pub fn run_anim_program(
  program: &VmProgram,
  ctx: &mut VmContext,
  state: &AvatarReplicatedState,
  output: &mut AvatarAnimOutput,
) -> VmExecStats {
  let mut scratch_state = state.values;
  run_vm(program, ctx, &state.values, &mut scratch_state, Some(&mut output.values))
}

#[inline(always)]
fn run_vm(
  program: &VmProgram,
  ctx: &mut VmContext,
  inputs: &[f32; VM_INPUTS],
  state: &mut [f32; VM_STATE],
  mut outputs: Option<&mut [f32; VM_OUTPUTS]>,
) -> VmExecStats {
  let regs = &mut ctx.regs;
  let max_steps = usize::min(program.max_steps as usize, program.instructions.len());

  let mut stats = VmExecStats::default();

  for (pc, ins) in program.instructions.iter().take(max_steps).enumerate() {
      black_box(pc);

      match ins.op {
          OpCode::Halt => {
              stats.halted = true;
              stats.steps_executed = (pc + 1) as u16;
              return stats;
          }
          OpCode::LoadInput => {
              regs[ins.a as usize] = inputs[ins.b as usize];
          }
          OpCode::LoadState => {
              regs[ins.a as usize] = state[ins.b as usize];
          }
          OpCode::StoreState => {
              state[ins.b as usize] = regs[ins.a as usize];
          }
          OpCode::StoreOutput => {
              if let Some(out) = outputs.as_deref_mut() {
                  out[ins.b as usize] = regs[ins.a as usize];
              }
          }
          OpCode::ConstF32 => {
              regs[ins.a as usize] = ins.imm;
          }
          OpCode::Add => {
              regs[ins.a as usize] = regs[ins.b as usize] + regs[ins.c as usize];
          }
          OpCode::Sub => {
              regs[ins.a as usize] = regs[ins.b as usize] - regs[ins.c as usize];
          }
          OpCode::Mul => {
              regs[ins.a as usize] = regs[ins.b as usize] * regs[ins.c as usize];
          }
          OpCode::Min => {
              regs[ins.a as usize] = regs[ins.b as usize].min(regs[ins.c as usize]);
          }
          OpCode::Max => {
              regs[ins.a as usize] = regs[ins.b as usize].max(regs[ins.c as usize]);
          }
          OpCode::Clamp01 => {
              regs[ins.a as usize] = regs[ins.b as usize].clamp(0.0, 1.0);
          }
          OpCode::Sin => {
              regs[ins.a as usize] = regs[ins.b as usize].sin();
          }
          OpCode::Cos => {
              regs[ins.a as usize] = regs[ins.b as usize].cos();
          }
          OpCode::SelectGt => {
              let if_false_reg = ins.imm as u8;
              regs[ins.a as usize] = if regs[ins.b as usize] > 0.0 {
                  regs[ins.c as usize]
              } else {
                  regs[if_false_reg as usize]
              };
          }
      }
  }

  stats.steps_executed = max_steps as u16;
  stats
}

// ------------------------------------------------------------
// Ejemplos de programas
// ------------------------------------------------------------

pub fn example_input_program() -> VmProgram {
  // inputs:
  // 0 = move_x
  // 1 = move_y
  // 2 = jump_pressed (0/1)
  //
  // state:
  // 0 = locomotion_x
  // 1 = locomotion_y
  // 2 = jump_flag
  VmProgram {
      instructions: vec![
          Instr::load_input(0, 0),
          Instr::load_input(1, 1),
          Instr::load_input(2, 2),
          Instr::store_state(0, 0),
          Instr::store_state(1, 1),
          Instr::store_state(2, 2),
          Instr::halt(),
      ]
      .into_boxed_slice(),
      max_steps: 16,
  }
}

pub fn example_anim_program() -> VmProgram {
  // state:
  // 0 = locomotion_x
  // 1 = locomotion_y
  // 2 = jump_flag
  //
  // output:
  // 0 = walk_weight
  // 1 = strafe_weight
  // 2 = jump_weight
  VmProgram {
      instructions: vec![
          Instr::load_input(0, 0), // state[0]
          Instr::load_input(1, 1), // state[1]
          Instr::load_input(2, 2), // state[2]
          Instr::store_output(0, 0),
          Instr::store_output(1, 1),
          Instr::store_output(2, 2),
          Instr::halt(),
      ]
      .into_boxed_slice(),
      max_steps: 16,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Instant;

  #[test]
  fn validates_example_programs() {
      assert!(example_input_program().validate().is_ok());
      assert!(example_anim_program().validate().is_ok());
  }

  #[test]
  fn input_program_writes_replicated_state() {
      let program = example_input_program();
      let mut ctx = VmContext::default();
      let input = AvatarRawInput {
          values: [0.25, 0.75, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
      };
      let mut state = AvatarReplicatedState::default();

      let stats = run_input_program(&program, &mut ctx, &input, &mut state);

      assert!(stats.halted);
      assert_eq!(state.values[0], 0.25);
      assert_eq!(state.values[1], 0.75);
      assert_eq!(state.values[2], 1.0);
  }

  #[test]
  fn anim_program_reads_state_and_writes_outputs() {
      let program = example_anim_program();
      let mut ctx = VmContext::default();
      let state = AvatarReplicatedState {
          values: [0.4, 0.8, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
      };
      let mut output = AvatarAnimOutput::default();

      let stats = run_anim_program(&program, &mut ctx, &state, &mut output);

      assert!(stats.halted);
      assert_eq!(output.values[0], 0.4);
      assert_eq!(output.values[1], 0.8);
      assert_eq!(output.values[2], 1.0);
  }

  #[test]
  #[ignore]
  fn bench_vm_10k_input_passes() {
      let program = example_input_program();
      let mut ctxs = vec![VmContext::default(); 10_000];
      let input = AvatarRawInput {
          values: [0.3, 0.7, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
      };
      let mut states = vec![AvatarReplicatedState::default(); 10_000];

      let start = Instant::now();
      for i in 0..10_000 {
          run_input_program(&program, &mut ctxs[i], &input, &mut states[i]);
      }
      let elapsed = start.elapsed();

      let per_avatar_ns = elapsed.as_nanos() as f64 / 10_000.0;
      eprintln!(
          "10k input passes: {:?} total, {:.2} ns/avatar",
          elapsed, per_avatar_ns
      );
  }

  #[test]
  #[ignore]
  fn bench_vm_10k_anim_passes() {
      let program = example_anim_program();
      let mut ctxs = vec![VmContext::default(); 10_000];
      let states = vec![
          AvatarReplicatedState {
              values: [0.2, 0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
          };
          10_000
      ];
      let mut outputs = vec![AvatarAnimOutput::default(); 10_000];

      let start = Instant::now();
      for i in 0..10_000 {
          run_anim_program(&program, &mut ctxs[i], &states[i], &mut outputs[i]);
      }
      let elapsed = start.elapsed();

      let per_avatar_ns = elapsed.as_nanos() as f64 / 10_000.0;
      eprintln!(
          "10k anim passes: {:?} total, {:.2} ns/avatar",
          elapsed, per_avatar_ns
      );
  }
}