/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! This is separated out into its own package so that we can avoid rebuilding
//! dynarmic more often than necessary, and to improve build-time parallelism.

// Allow the crate to have a non-snake-case name (touchHLE).
// This also allows items in the crate to have non-snake-case names.
#![allow(non_snake_case)]

/// Opaque type from C
#[allow(non_camel_case_types)]
pub type touchHLE_DynarmicWrapper = std::ffi::c_void;
/// Opaque type from Rust (this is the `Mem` type from the main crate, but
/// `c_void` is used here to avoid depending on it directly)
#[allow(non_camel_case_types)]
pub type touchHLE_Mem = std::ffi::c_void;

#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Debug)]
pub struct touchHLE_DynarmicContext {
    pub regs: [u32; 16],
    pub extregs: [u32; 64],
    pub cpsr: u32,
    pub fpscr: u32,
}

impl Default for touchHLE_DynarmicContext {
    fn default() -> Self {
        Self {
            regs: [0; 16],
            extregs: [0; 64],
            cpsr: 0,
            fpscr: 0,
        }
    }
}

impl touchHLE_DynarmicContext {
    pub fn new() -> Self {
        Self::default()
    }
}
type VAddr = u32;

// Import functions from lib.cpp, see build.rs. Note that lib.cpp depends on
// some functions being exported from Rust, but those are in the main crate.
extern "C" {
    pub fn touchHLE_DynarmicWrapper_new(
        dynamic_memory_access_ptr: *mut std::ffi::c_void,
        null_page_count: usize,
        fastmem_enabled: bool,
    ) -> *mut touchHLE_DynarmicWrapper;
    pub fn touchHLE_DynarmicWrapper_delete(cpu: *mut touchHLE_DynarmicWrapper);
    pub fn touchHLE_DynarmicWrapper_regs_const(cpu: *const touchHLE_DynarmicWrapper) -> *const u32;
    pub fn touchHLE_DynarmicWrapper_regs_mut(cpu: *mut touchHLE_DynarmicWrapper) -> *mut u32;
    pub fn touchHLE_DynarmicWrapper_extregs_const(
        cpu: *const touchHLE_DynarmicWrapper,
    ) -> *const u32;
    pub fn touchHLE_DynarmicWrapper_extregs_mut(cpu: *mut touchHLE_DynarmicWrapper) -> *mut u32;
    pub fn touchHLE_DynarmicWrapper_cpsr(cpu: *const touchHLE_DynarmicWrapper) -> u32;
    pub fn touchHLE_DynarmicWrapper_set_cpsr(cpu: *mut touchHLE_DynarmicWrapper, cpsr: u32);
    pub fn touchHLE_DynarmicWrapper_fpscr(cpu: *const touchHLE_DynarmicWrapper) -> u32;
    pub fn touchHLE_DynarmicWrapper_set_fpscr(cpu: *mut touchHLE_DynarmicWrapper, fpscr: u32);
    pub fn touchHLE_DynarmicWrapper_set_tpidruro(cpu: *mut touchHLE_DynarmicWrapper, value: u32);

    pub fn touchHLE_DynarmicWrapper_last_exception_pc(
        cpu: *const touchHLE_DynarmicWrapper,
    ) -> VAddr;
    pub fn touchHLE_DynarmicWrapper_swap_context(
        cpu: *mut touchHLE_DynarmicWrapper,
        context: *mut touchHLE_DynarmicContext,
    );
    pub fn touchHLE_DynarmicWrapper_invalidate_cache_range(
        cpu: *mut touchHLE_DynarmicWrapper,
        start: VAddr,
        size: u32,
    );
    pub fn touchHLE_DynarmicWrapper_run_or_step(
        cpu: *mut touchHLE_DynarmicWrapper,
        mem: *mut touchHLE_Mem,
        ticks: Option<&mut u64>,
    ) -> i32;

}

/// Opaque type from C (the A64 wrapper in lib_a64.cpp)
#[allow(non_camel_case_types)]
pub type touchHLE_DynarmicA64Wrapper = std::ffi::c_void;
/// Opaque type from Rust (this is the `Mem64` type from the main crate, but
/// `c_void` is used here to avoid depending on it directly)
#[allow(non_camel_case_types)]
pub type touchHLE_Mem64 = std::ffi::c_void;

/// Guest register file snapshot for the A64 core. Layout must match
/// `touchHLE_DynarmicA64Context` in lib_a64.cpp. The main crate defines its
/// own `CpuContext` mirroring this type (see src/cpu/a64.rs); keeping this
/// copy here avoids a dependency cycle while staying honest about the ABI.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
pub struct touchHLE_DynarmicA64Context {
    pub regs: [u64; 31],
    pub vectors: [u64; 64],
    pub sp: u64,
    pub fpcr: u32,
    pub fpsr: u32,
    pub pstate: u32,
}

impl Default for touchHLE_DynarmicA64Context {
    fn default() -> Self {
        Self {
            regs: [0; 31],
            vectors: [0; 64],
            sp: 0,
            fpcr: 0,
            fpsr: 0,
            pstate: 0,
        }
    }
}

// Import functions from lib_a64.cpp, see build.rs.
extern "C" {
    pub fn touchHLE_DynarmicA64Wrapper_new() -> *mut touchHLE_DynarmicA64Wrapper;
    pub fn touchHLE_DynarmicA64Wrapper_delete(cpu: *mut touchHLE_DynarmicA64Wrapper);
    pub fn touchHLE_DynarmicA64Wrapper_get_reg(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        index: usize,
    ) -> u64;
    pub fn touchHLE_DynarmicA64Wrapper_set_reg(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        index: usize,
        value: u64,
    );
    pub fn touchHLE_DynarmicA64Wrapper_get_sp(cpu: *mut touchHLE_DynarmicA64Wrapper) -> u64;
    pub fn touchHLE_DynarmicA64Wrapper_set_sp(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u64,
    );
    pub fn touchHLE_DynarmicA64Wrapper_get_pc(cpu: *mut touchHLE_DynarmicA64Wrapper) -> u64;
    pub fn touchHLE_DynarmicA64Wrapper_set_pc(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u64,
    );
    pub fn touchHLE_DynarmicA64Wrapper_set_fpcr(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u32,
    );
    pub fn touchHLE_DynarmicA64Wrapper_set_fpsr(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u32,
    );
    pub fn touchHLE_DynarmicA64Wrapper_set_pstate(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u32,
    );
    pub fn touchHLE_DynarmicA64Wrapper_set_tpidrro_el0(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        value: u64,
    );
    pub fn touchHLE_DynarmicA64Wrapper_last_exception_pc(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
    ) -> u64;
    pub fn touchHLE_DynarmicA64Wrapper_swap_context(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        context: *mut touchHLE_DynarmicA64Context,
    );
    pub fn touchHLE_DynarmicA64Wrapper_invalidate_cache_range(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        start: u64,
        size: u64,
    );
    pub fn touchHLE_DynarmicA64Wrapper_run_or_step(
        cpu: *mut touchHLE_DynarmicA64Wrapper,
        mem: *mut touchHLE_Mem64,
        ticks: Option<&mut u64>,
    ) -> i32;
}
