/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
// Dynarmic A64 (arm64) frontend wrapper.
//
// This is the CPU core for 64-bit guest apps (see PROGRESS.md decision D1).
// It deliberately mirrors lib.cpp (the A32 wrapper) so both cores stay easy
// to compare: same halt-reason mapping (-1 normal, -2 memory abort, -3
// undefined instruction, -4 breakpoint, otherwise the SVC number), same
// "Rust owns memory, C++ owns the JIT" split, same swap_context model.
//
// Differences from A32 that matter here:
// * 31 general-purpose registers (x0-x30), SP and PC are separate state.
// * 32 128-bit vector registers (v0-v31), stored as 32 x 2 x u64.
// * No Thumb flag and no coprocessors; system registers that user code may
//   read (TPIDRRO_EL0 — the arm64 successor of the A32 TPIDRURO TSD pointer —
//   plus CNTFRQ_EL0/CTR_EL0) are configured via UserConfig.
// * The A64 page table in dynarmic wants a sparse 64-bit address space
//   (default 36 address bits = a 128 MiB table). The 64-bit guest memory
//   model is still under construction (PROGRESS.md P1), so this wrapper
//   starts callback-only: every access goes through the Rust-side callbacks.
//   The page table / fastmem integration is a follow-up performance step and
//   must come with the real Mem64.

#include <chrono>
#include <cstdint>
#include <cstdio>

#include "dynarmic/interface/A64/a64.h"
#include "dynarmic/interface/A64/config.h"
#include "dynarmic/interface/exclusive_monitor.h"

namespace touchHLE::cpu::a64 {

using VAddr = std::uint64_t;

// Types and functions defined in Rust
extern "C" {
struct touchHLE_Mem64;
std::uint8_t touchHLE_cpu_a64_read_u8(touchHLE_Mem64 *mem, VAddr addr,
                                      bool *error);
std::uint16_t touchHLE_cpu_a64_read_u16(touchHLE_Mem64 *mem, VAddr addr,
                                        bool *error);
std::uint32_t touchHLE_cpu_a64_read_u32(touchHLE_Mem64 *mem, VAddr addr,
                                        bool *error);
std::uint64_t touchHLE_cpu_a64_read_u64(touchHLE_Mem64 *mem, VAddr addr,
                                        bool *error);
bool touchHLE_cpu_a64_write_u8(touchHLE_Mem64 *mem, VAddr addr,
                               std::uint8_t value);
bool touchHLE_cpu_a64_write_u16(touchHLE_Mem64 *mem, VAddr addr,
                                std::uint16_t value);
bool touchHLE_cpu_a64_write_u32(touchHLE_Mem64 *mem, VAddr addr,
                                std::uint32_t value);
bool touchHLE_cpu_a64_write_u64(touchHLE_Mem64 *mem, VAddr addr,
                                std::uint64_t value);
// Guest register file snapshot for thread switches. Layout must match
// `touchHLE_DynarmicA64Context` in src/cpu/a64.rs.
struct touchHLE_DynarmicA64Context {
  std::array<std::uint64_t, 31> regs;
  std::array<std::uint64_t, 64> vectors;
  std::uint64_t sp;
  std::uint32_t fpcr;
  std::uint32_t fpsr;
  std::uint32_t pstate;
};
}

const auto HaltReasonSvc = Dynarmic::HaltReason::UserDefined1;
const auto HaltReasonUndefinedInstruction = Dynarmic::HaltReason::UserDefined2;
const auto HaltReasonBreakpoint = Dynarmic::HaltReason::UserDefined3;

class Environment final : public Dynarmic::A64::UserCallbacks {
public:
  Dynarmic::A64::Jit *cpu = nullptr;
  touchHLE_Mem64 *mem = nullptr;
  std::uint64_t ticks_remaining;
  std::uint32_t halting_svc;
  // Start address of the instruction that most recently raised an
  // UndefinedInstruction/Breakpoint exception (see the A32 wrapper for why
  // the exact PC is captured here rather than recovered from the register
  // file).
  VAddr last_exception_pc = 0;

private:
  std::uint8_t MemoryRead8(VAddr vaddr) override {
    bool error;
    auto value = touchHLE_cpu_a64_read_u8(mem, vaddr, &error);
    if (error) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
    return value;
  }
  std::uint16_t MemoryRead16(VAddr vaddr) override {
    bool error;
    auto value = touchHLE_cpu_a64_read_u16(mem, vaddr, &error);
    if (error) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
    return value;
  }
  std::uint32_t MemoryRead32(VAddr vaddr) override {
    bool error;
    auto value = touchHLE_cpu_a64_read_u32(mem, vaddr, &error);
    if (error) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
    return value;
  }
  std::uint64_t MemoryRead64(VAddr vaddr) override {
    bool error;
    auto value = touchHLE_cpu_a64_read_u64(mem, vaddr, &error);
    if (error) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
    return value;
  }
  Dynarmic::A64::Vector MemoryRead128(VAddr vaddr) override {
    // 128-bit accesses decompose into two 64-bit callbacks; no host ABI
    // games with __int128 needed.
    Dynarmic::A64::Vector value;
    value[0] = MemoryRead64(vaddr);
    value[1] = MemoryRead64(vaddr + 8);
    return value;
  }

  std::optional<std::uint32_t> MemoryReadCode(VAddr vaddr) override {
    bool error;
    auto value = touchHLE_cpu_a64_read_u32(mem, vaddr, &error);
    if (error) {
      return std::nullopt;
    } else {
      return value;
    }
  }

  void MemoryWrite8(VAddr vaddr, std::uint8_t value) override {
    if (touchHLE_cpu_a64_write_u8(mem, vaddr, value)) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
  }
  void MemoryWrite16(VAddr vaddr, std::uint16_t value) override {
    if (touchHLE_cpu_a64_write_u16(mem, vaddr, value)) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
  }
  void MemoryWrite32(VAddr vaddr, std::uint32_t value) override {
    if (touchHLE_cpu_a64_write_u32(mem, vaddr, value)) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
  }
  void MemoryWrite64(VAddr vaddr, std::uint64_t value) override {
    if (touchHLE_cpu_a64_write_u64(mem, vaddr, value)) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    }
  }
  void MemoryWrite128(VAddr vaddr, Dynarmic::A64::Vector value) override {
    MemoryWrite64(vaddr, value[0]);
    MemoryWrite64(vaddr + 8, value[1]);
  }

  bool MemoryWriteExclusive8(VAddr addr, std::uint8_t value,
                             std::uint8_t expected) override {
    if (MemoryRead8(addr) != expected) {
      return false;
    }
    MemoryWrite8(addr, value);
    return true;
  }
  bool MemoryWriteExclusive16(VAddr addr, std::uint16_t value,
                              std::uint16_t expected) override {
    if (MemoryRead16(addr) != expected) {
      return false;
    }
    MemoryWrite16(addr, value);
    return true;
  }
  bool MemoryWriteExclusive32(VAddr addr, std::uint32_t value,
                              std::uint32_t expected) override {
    if (MemoryRead32(addr) != expected) {
      return false;
    }
    MemoryWrite32(addr, value);
    return true;
  }
  bool MemoryWriteExclusive64(VAddr addr, std::uint64_t value,
                              std::uint64_t expected) override {
    if (MemoryRead64(addr) != expected) {
      return false;
    }
    MemoryWrite64(addr, value);
    return true;
  }
  bool MemoryWriteExclusive128(VAddr addr, Dynarmic::A64::Vector value,
                               Dynarmic::A64::Vector expected) override {
    if (MemoryRead64(addr) != expected[0] ||
        MemoryRead64(addr + 8) != expected[1]) {
      return false;
    }
    MemoryWrite128(addr, value);
    return true;
  }

  void InterpreterFallback(VAddr pc, size_t) override {
    // An instruction dynarmic could not translate (UDF, undecodable
    // encodings, runs of zeros). Halt as an undefined instruction so the
    // Rust side can log and recover, exactly like the ExceptionRaised
    // path — aborting here would kill the host process on the first
    // undecodable guest instruction (seen with repack-corrupted code).
    last_exception_pc = pc;
    cpu->HaltExecution(HaltReasonUndefinedInstruction);
  }
  void CallSVC(std::uint32_t svc) override {
    halting_svc = svc;
    cpu->HaltExecution(HaltReasonSvc);
  }
  void ExceptionRaised(VAddr pc, Dynarmic::A64::Exception exception) override {
    if (exception == Dynarmic::A64::Exception::NoExecuteFault) {
      cpu->HaltExecution(Dynarmic::HaltReason::MemoryAbort);
    } else if (exception == Dynarmic::A64::Exception::Breakpoint) {
      last_exception_pc = pc;
      cpu->HaltExecution(HaltReasonBreakpoint);
    } else if (exception == Dynarmic::A64::Exception::Yield ||
               exception == Dynarmic::A64::Exception::WaitForEvent ||
               exception == Dynarmic::A64::Exception::WaitForInterrupt ||
               exception == Dynarmic::A64::Exception::SendEvent ||
               exception == Dynarmic::A64::Exception::SendEventLocal) {
      // Hint instructions: no-ops on real hardware, dynarmic already
      // advanced the PC, so just continue (same policy as the A32 wrapper).
    } else {
      // UnallocatedEncoding / ReservedValue / UnpredictableInstruction:
      // treat like an undefined instruction so the Rust side can decide
      // (log, bypass or halt) instead of killing the host process.
      std::fprintf(stderr, "A64 ExceptionRaised: exception %u at %llx\n",
                   unsigned(exception), (unsigned long long)pc);
      last_exception_pc = pc;
      cpu->HaltExecution(HaltReasonUndefinedInstruction);
    }
  }
  void AddTicks(std::uint64_t ticks) override {
    if (ticks > ticks_remaining) {
      ticks_remaining = 0;
      return;
    }
    ticks_remaining -= ticks;
  }
  std::uint64_t GetTicksRemaining() override { return ticks_remaining; }
  std::uint64_t GetCNTPCT() override {
    // Guest-visible physical counter (CNTPCT_EL0). The HLE layer implements
    // the real time APIs (mach_absolute_time etc.), so any monotonically
    // increasing nanosecond value is fine here.
    return std::chrono::duration_cast<std::chrono::nanoseconds>(
               std::chrono::steady_clock::now().time_since_epoch())
        .count();
  }
};

class DynarmicA64Wrapper {
  Environment env;
  // TPIDRRO_EL0 holds the guest thread's thread-specific-data pointer (the
  // arm64 equivalent of the A32 CP15 TPIDRURO handled in lib.cpp). Guest
  // code reads it through a pointer, so it must live at a fixed address and
  // be updated in place on every guest thread switch.
  std::uint64_t tpidrro_el0 = 0;
  std::unique_ptr<Dynarmic::A64::Jit> cpu;
  std::unique_ptr<Dynarmic::ExclusiveMonitor> mon;

public:
  DynarmicA64Wrapper() {
    Dynarmic::A64::UserConfig user_config;
    user_config.callbacks = &env;
    // CLREX/atomics: keep the exclusive monitor (same reasoning as A32).
    mon = std::make_unique<Dynarmic::ExclusiveMonitor>(1);
    user_config.global_monitor = mon.get();
    // Same FP unsafe-optimization policy as the A32 wrapper: drop per-*
    // instruction FPCR/NaN bookkeeping, keep the global monitor.
    user_config.unsafe_optimizations = true;
    user_config.optimizations =
        user_config.optimizations |
        Dynarmic::OptimizationFlag::Unsafe_UnfuseFMA |
        Dynarmic::OptimizationFlag::Unsafe_ReducedErrorFP |
        Dynarmic::OptimizationFlag::Unsafe_InaccurateNaN |
        Dynarmic::OptimizationFlag::Unsafe_IgnoreStandardFPCRValue;
    // iOS userland reads the read-only thread pointer via TPIDRRO_EL0
    // (pthread_getspecific and Mach thread state both end up there).
    user_config.tpidrro_el0 = &tpidrro_el0;
    // CNTFRQ_EL0: iOS reports a 24 MHz timebase. mach_absolute_time is HLE'd
    // anyway, but user code sometimes reads the counter register directly.
    user_config.cntfrq_el0 = 24'000'000;
#ifndef NDEBUG
    user_config.check_halt_on_memory_access = true;
#endif
    cpu = std::make_unique<Dynarmic::A64::Jit>(user_config);
    env.cpu = cpu.get();
  }

  std::uint64_t get_register(size_t index) const {
    return cpu->GetRegister(index);
  }
  void set_register(size_t index, std::uint64_t value) {
    cpu->SetRegister(index, value);
  }

  std::uint64_t sp() const { return cpu->GetSP(); }
  void set_sp(std::uint64_t value) { cpu->SetSP(value); }
  std::uint64_t pc() const { return cpu->GetPC(); }
  void set_pc(std::uint64_t value) { cpu->SetPC(value); }

  std::uint32_t fpcr() const { return cpu->GetFpcr(); }
  void set_fpcr(std::uint32_t value) { cpu->SetFpcr(value); }
  std::uint32_t fpsr() const { return cpu->GetFpsr(); }
  void set_fpsr(std::uint32_t value) { cpu->SetFpsr(value); }
  std::uint32_t pstate() const { return cpu->GetPstate(); }
  void set_pstate(std::uint32_t value) { cpu->SetPstate(value); }
  void set_tpidrro_el0(std::uint64_t value) { tpidrro_el0 = value; }

  std::uint64_t last_exception_pc() const { return env.last_exception_pc; }

  void invalidate_cache_range(VAddr start, std::uint64_t size) {
    cpu->InvalidateCacheRange(start, size);
  }

  void get_context(touchHLE_DynarmicA64Context *context) {
    context->regs = cpu->GetRegisters();
    auto vectors = cpu->GetVectors();
    for (size_t i = 0; i < vectors.size(); ++i) {
      context->vectors[i * 2] = vectors[i][0];
      context->vectors[i * 2 + 1] = vectors[i][1];
    }
    context->sp = cpu->GetSP();
    context->fpcr = cpu->GetFpcr();
    context->fpsr = cpu->GetFpsr();
    context->pstate = cpu->GetPstate();
  }

  void set_context(const touchHLE_DynarmicA64Context *context) {
    cpu->SetRegisters(context->regs);
    std::array<Dynarmic::A64::Vector, 32> vectors;
    for (size_t i = 0; i < vectors.size(); ++i) {
      vectors[i] = {context->vectors[i * 2], context->vectors[i * 2 + 1]};
    }
    cpu->SetVectors(vectors);
    cpu->SetSP(context->sp);
    cpu->SetFpcr(context->fpcr);
    cpu->SetFpsr(context->fpsr);
    cpu->SetPstate(context->pstate);
  }

  void swap_context(touchHLE_DynarmicA64Context *context) {
    touchHLE_DynarmicA64Context tmp;
    get_context(&tmp);
    set_context(context);
    *context = tmp;
  }

  std::int32_t run_or_step(touchHLE_Mem64 *mem, std::uint64_t *ticks) {
    env.mem = mem;
    Dynarmic::HaltReason hr;
    if (ticks) {
      env.ticks_remaining = *ticks;
      hr = cpu->Run();
    } else {
      hr = cpu->Step();
    }
    std::int32_t res;
    if ((!hr && ticks) || (hr == Dynarmic::HaltReason::Step && !ticks)) {
      res = -1;
    } else if (Dynarmic::Has(hr, Dynarmic::HaltReason::MemoryAbort)) {
      res = -2;
    } else if (Dynarmic::Has(hr, HaltReasonUndefinedInstruction)) {
      res = -3;
    } else if (Dynarmic::Has(hr, HaltReasonBreakpoint)) {
      res = -4;
    } else if (Dynarmic::Has(hr, HaltReasonSvc)) {
      res = std::int32_t(env.halting_svc);
    } else {
      printf("A64: unhandled halt reason %u\n", unsigned(hr));
      abort();
    }
    env.mem = nullptr;
    if (ticks) {
      *ticks = env.ticks_remaining;
    }
    return res;
  }
};

extern "C" {

DynarmicA64Wrapper *touchHLE_DynarmicA64Wrapper_new() {
  return new DynarmicA64Wrapper();
}
void touchHLE_DynarmicA64Wrapper_delete(DynarmicA64Wrapper *cpu) { delete cpu; }

std::uint64_t touchHLE_DynarmicA64Wrapper_get_reg(DynarmicA64Wrapper *cpu,
                                                  size_t index) {
  return cpu->get_register(index);
}
void touchHLE_DynarmicA64Wrapper_set_reg(DynarmicA64Wrapper *cpu, size_t index,
                                         std::uint64_t value) {
  cpu->set_register(index, value);
}
std::uint64_t touchHLE_DynarmicA64Wrapper_get_sp(DynarmicA64Wrapper *cpu) {
  return cpu->sp();
}
void touchHLE_DynarmicA64Wrapper_set_sp(DynarmicA64Wrapper *cpu,
                                        std::uint64_t value) {
  cpu->set_sp(value);
}
std::uint64_t touchHLE_DynarmicA64Wrapper_get_pc(DynarmicA64Wrapper *cpu) {
  return cpu->pc();
}
void touchHLE_DynarmicA64Wrapper_set_pc(DynarmicA64Wrapper *cpu,
                                        std::uint64_t value) {
  cpu->set_pc(value);
}

std::uint32_t touchHLE_DynarmicA64Wrapper_fpcr(DynarmicA64Wrapper *cpu) {
  return cpu->fpcr();
}
void touchHLE_DynarmicA64Wrapper_set_fpcr(DynarmicA64Wrapper *cpu,
                                          std::uint32_t value) {
  cpu->set_fpcr(value);
}
std::uint32_t touchHLE_DynarmicA64Wrapper_fpsr(DynarmicA64Wrapper *cpu) {
  return cpu->fpsr();
}
void touchHLE_DynarmicA64Wrapper_set_fpsr(DynarmicA64Wrapper *cpu,
                                          std::uint32_t value) {
  cpu->set_fpsr(value);
}
std::uint32_t touchHLE_DynarmicA64Wrapper_pstate(DynarmicA64Wrapper *cpu) {
  return cpu->pstate();
}
void touchHLE_DynarmicA64Wrapper_set_pstate(DynarmicA64Wrapper *cpu,
                                            std::uint32_t value) {
  cpu->set_pstate(value);
}
void touchHLE_DynarmicA64Wrapper_set_tpidrro_el0(DynarmicA64Wrapper *cpu,
                                                 std::uint64_t value) {
  cpu->set_tpidrro_el0(value);
}

std::uint64_t
touchHLE_DynarmicA64Wrapper_last_exception_pc(DynarmicA64Wrapper *cpu) {
  return cpu->last_exception_pc();
}

void touchHLE_DynarmicA64Wrapper_swap_context(
    DynarmicA64Wrapper *cpu, touchHLE_DynarmicA64Context *context) {
  cpu->swap_context(context);
}

void touchHLE_DynarmicA64Wrapper_invalidate_cache_range(
    DynarmicA64Wrapper *cpu, std::uint64_t start, std::uint64_t size) {
  cpu->invalidate_cache_range(start, size);
}

std::int32_t
touchHLE_DynarmicA64Wrapper_run_or_step(DynarmicA64Wrapper *cpu,
                                        touchHLE_Mem64 *mem,
                                        std::uint64_t *ticks) {
  return cpu->run_or_step(mem, ticks);
}
}

} // namespace touchHLE::cpu::a64
