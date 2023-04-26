// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Interface between the circuit and prover/verifier

use alloc::vec::Vec;
use core::cell::RefMut;

use anyhow::Result;
use bytemuck::Pod;
use risc0_core::field::{Elem, ExtElem, Field};

// use crate::{hal::cpu::SyncSlice, taps::TapSet};
use crate::taps::TapSet;

// TODO: Remove references to these constants so we don't depend on a
// fixed set of register groups.
pub const REGISTER_GROUP_ACCUM: usize = 0;
pub const REGISTER_GROUP_CODE: usize = 1;
pub const REGISTER_GROUP_DATA: usize = 2;

enum SyncSliceRef<'a, T: Default + Clone + Pod> {
    FromBuf(RefMut<'a, [T]>),
    FromSlice(&'a SyncSlice<'a, T>),
}

/// A buffer which can be used across multiple threads.  Users are
/// responsible for ensuring that no two threads access the same
/// element at the same time.
pub struct SyncSlice<'a, T: Default + Clone + Pod> {
    _buf: SyncSliceRef<'a, T>,
    ptr: *mut T,
    size: usize,
}

// SAFETY: SyncSlice keeps a RefMut to the original CpuBuffer, so
// no other as_slice or as_slice_muts can be active at the same time.
//
// The user of the SyncSlice is responsible for ensuring that no
// two threads access the same elements at the same time.
unsafe impl<'a, T: Default + Clone + Pod> Sync for SyncSlice<'a, T> {}

impl<'a, T: Default + Clone + Pod> SyncSlice<'a, T> {
    pub fn new(mut buf: RefMut<'a, [T]>) -> Self {
        let ptr = buf.as_mut_ptr();
        let size = buf.len();
        SyncSlice {
            ptr,
            size,
            _buf: SyncSliceRef::FromBuf(buf),
        }
    }

    pub fn get_ptr(&self) -> *mut T {
        self.ptr
    }

    pub fn get(&self, offset: usize) -> T {
        assert!(offset < self.size);
        unsafe { self.ptr.add(offset).read() }
    }

    pub fn set(&self, offset: usize, val: T) {
        assert!(offset < self.size);
        unsafe { self.ptr.add(offset).write(val) }
    }

    pub fn slice(&self, offset: usize, size: usize) -> SyncSlice<'_, T> {
        assert!(
            offset + size <= self.size,
            "Attempting to slice [{offset}, {offset} + {size} = {}) from a slice of length {}",
            offset + size,
            self.size
        );
        SyncSlice {
            _buf: SyncSliceRef::FromSlice(self),
            ptr: unsafe { self.ptr.add(offset) },
            size: size,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

#[derive(Clone, Copy)]
pub struct MixState<EE: ExtElem> {
    pub tot: EE,
    pub mul: EE,
}

pub trait CircuitStepHandler<E: Elem> {
    fn call(
        &mut self,
        cycle: usize,
        name: &str,
        extra: &str,
        args: &[E],
        outs: &mut [E],
    ) -> Result<()>;

    fn sort(&mut self, name: &str);
    fn calc_prefix_products(&mut self);
}

pub struct CircuitStepContext {
    pub size: usize,
    pub cycle: usize,
}

pub trait CircuitStep<E: Elem> {
    fn step_exec<S: CircuitStepHandler<E>>(
        &self,
        ctx: &CircuitStepContext,
        custom: &mut S,
        args: &[SyncSlice<E>],
    ) -> Result<E>;

    fn step_verify_bytes<S: CircuitStepHandler<E>>(
        &self,
        ctx: &CircuitStepContext,
        custom: &mut S,
        args: &[SyncSlice<E>],
    ) -> Result<E>;

    fn step_verify_mem<S: CircuitStepHandler<E>>(
        &self,
        ctx: &CircuitStepContext,
        custom: &mut S,
        args: &[SyncSlice<E>],
    ) -> Result<E>;

    fn step_compute_accum<S: CircuitStepHandler<E>>(
        &self,
        ctx: &CircuitStepContext,
        custom: &mut S,
        args: &[SyncSlice<E>],
    ) -> Result<E>;

    fn step_verify_accum<S: CircuitStepHandler<E>>(
        &self,
        ctx: &CircuitStepContext,
        custom: &mut S,
        args: &[SyncSlice<E>],
    ) -> Result<E>;
}

pub trait PolyFp<F: Field> {
    fn poly_fp(
        &self,
        cycle: usize,
        steps: usize,
        mix: &F::ExtElem,
        args: &[&[F::Elem]],
    ) -> F::ExtElem;
}

pub trait PolyExt<F: Field> {
    fn poly_ext(
        &self,
        mix: &F::ExtElem,
        u: &[F::ExtElem],
        args: &[&[F::Elem]],
    ) -> MixState<F::ExtElem>;
}

pub trait TapsProvider {
    fn get_taps(&self) -> &'static TapSet<'static>;

    fn code_size(&self) -> usize {
        self.get_taps().group_size(REGISTER_GROUP_CODE)
    }
}

pub trait CircuitInfo {
    const OUTPUT_SIZE: usize;
    const MIX_SIZE: usize;
}

pub trait CircuitDef<F: Field>:
    CircuitInfo + CircuitStep<F::Elem> + PolyFp<F> + PolyExt<F> + TapsProvider + Sync
{
}

pub type Arg = usize;
pub type Var = usize;

pub struct PolyExtStepDef {
    pub block: &'static [PolyExtStep],
    pub ret: Var,
}

pub enum PolyExtStep {
    Const(u32),
    Get(usize),
    GetGlobal(Arg, usize),
    Add(Var, Var),
    Sub(Var, Var),
    Mul(Var, Var),
    True,
    AndEqz(Var, Var),
    AndCond(Var, Var, Var),
}

impl PolyExtStep {
    pub fn step<F: Field>(
        &self,
        fp_vars: &mut Vec<F::ExtElem>,
        mix_vars: &mut Vec<MixState<F::ExtElem>>,
        mix: &F::ExtElem,
        u: &[F::ExtElem],
        args: &[&[F::Elem]],
    ) {
        match self {
            PolyExtStep::Const(value) => {
                let elem = F::Elem::from_u64(*value as u64);
                fp_vars.push(F::ExtElem::from_subfield(&elem));
            }
            PolyExtStep::Get(tap) => {
                fp_vars.push(u[*tap]);
            }
            PolyExtStep::GetGlobal(base, offset) => {
                fp_vars.push(F::ExtElem::from_subfield(&args[*base][*offset]));
            }
            PolyExtStep::Add(x1, x2) => {
                fp_vars.push(fp_vars[*x1] + fp_vars[*x2]);
            }
            PolyExtStep::Sub(x1, x2) => {
                fp_vars.push(fp_vars[*x1] - fp_vars[*x2]);
            }
            PolyExtStep::Mul(x1, x2) => {
                fp_vars.push(fp_vars[*x1] * fp_vars[*x2]);
            }
            PolyExtStep::True => {
                mix_vars.push(MixState {
                    tot: F::ExtElem::ZERO,
                    mul: F::ExtElem::ONE,
                });
            }
            PolyExtStep::AndEqz(x, val) => {
                let x = mix_vars[*x];
                let val = fp_vars[*val];
                mix_vars.push(MixState {
                    tot: x.tot + x.mul * val,
                    mul: x.mul * *mix,
                });
            }
            PolyExtStep::AndCond(x, cond, inner) => {
                let x = mix_vars[*x];
                let cond = fp_vars[*cond];
                let inner = mix_vars[*inner];
                mix_vars.push(MixState {
                    tot: x.tot + cond * inner.tot * x.mul,
                    mul: x.mul * inner.mul,
                });
            }
        }
    }
}

impl PolyExtStepDef {
    pub fn step<F: Field>(
        &self,
        mix: &F::ExtElem,
        u: &[F::ExtElem],
        args: &[&[F::Elem]],
    ) -> MixState<F::ExtElem> {
        let mut fp_vars = Vec::with_capacity(self.block.len() - (self.ret + 1));
        let mut mix_vars = Vec::with_capacity(self.ret + 1);
        for op in self.block.iter() {
            op.step::<F>(&mut fp_vars, &mut mix_vars, mix, u, args);
        }
        assert_eq!(
            fp_vars.len(),
            self.block.len() - (self.ret + 1),
            "Miscalculated capacity for fp_vars"
        );
        assert_eq!(
            mix_vars.len(),
            self.ret + 1,
            "Miscalculated capacity for mix_vars"
        );
        mix_vars[self.ret]
    }
}
