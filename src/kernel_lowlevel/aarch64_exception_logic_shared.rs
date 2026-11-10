#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64El0MemoryAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64El0AbortKind {
    Translation,
    AccessFlag,
    Permission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Aarch64El0MemoryFault {
    pub access: Aarch64El0MemoryAccess,
    pub kind: Aarch64El0AbortKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64LowerElSync {
    Svc,
    MemoryFault(Aarch64El0MemoryFault),
    /// EC 0x00. Includes the UDF encoding used by EL0.
    UndefinedInstruction,
    /// PC/SP alignment exceptions, and a data abort whose DFSC is alignment.
    AlignmentFault,
    /// Every other lower-EL synchronous class. Still a process fault, not a
    /// reason to halt the kernel and not a reason to resume EL0.
    Unsupported,
}

/// What the lower-EL synchronous vector must do. EL1's own vectors never use
/// this; a saved SPSR that is not EL0 still halts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64LowerElAction {
    HaltKernel,
    DeliverMemoryFault(Aarch64El0MemoryFault),
    /// Kill the current EL0 process (or abandon the EL0 frame when it has no
    /// process) and return to the scheduler. Never resume the faulting PC.
    TerminateProcess,
}

pub(crate) fn aarch64_saved_state_is_el0(spsr: u64) -> bool {
    // SPSR.M[3:2] is the target exception level. EL0 is 0.
    spsr & 0xc == 0
}

pub(crate) fn aarch64_lower_el_sync(esr: u64) -> Aarch64LowerElSync {
    let ec = (esr >> 26) & 0x3f;
    if ec == 0x15 {
        return Aarch64LowerElSync::Svc;
    }
    if ec == 0x00 {
        return Aarch64LowerElSync::UndefinedInstruction;
    }
    // PC alignment fault and SP alignment fault.
    if ec == 0x22 || ec == 0x26 {
        return Aarch64LowerElSync::AlignmentFault;
    }

    let access = match ec {
        0x20 => Aarch64El0MemoryAccess::Execute,
        0x24 if esr & (1 << 6) != 0 => Aarch64El0MemoryAccess::Write,
        0x24 => Aarch64El0MemoryAccess::Read,
        _ => return Aarch64LowerElSync::Unsupported,
    };
    // DFSC 0b100001 is an alignment fault taken as a data/instruction abort.
    if esr & 0x3f == 0x21 {
        return Aarch64LowerElSync::AlignmentFault;
    }
    let kind = match esr & 0x3f {
        0x04..=0x07 => Aarch64El0AbortKind::Translation,
        0x08..=0x0b => Aarch64El0AbortKind::AccessFlag,
        0x0c..=0x0f => Aarch64El0AbortKind::Permission,
        _ => return Aarch64LowerElSync::Unsupported,
    };

    Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault { access, kind })
}

pub(crate) fn aarch64_lower_el_action(from_el0: bool, class: Aarch64LowerElSync) -> Aarch64LowerElAction {
    if !from_el0 {
        return Aarch64LowerElAction::HaltKernel;
    }
    match class {
        Aarch64LowerElSync::MemoryFault(fault) => Aarch64LowerElAction::DeliverMemoryFault(fault),
        Aarch64LowerElSync::Svc
        | Aarch64LowerElSync::UndefinedInstruction
        | Aarch64LowerElSync::AlignmentFault
        | Aarch64LowerElSync::Unsupported => Aarch64LowerElAction::TerminateProcess,
    }
}

/// `SIG_DFL` is 0 and `SIG_IGN` is 1. Any other handler may run only when it
/// points at one of the current process's executable user pages.
pub(crate) fn el0_signal_handler_may_run(handler: u64, executable_user_page: bool) -> bool {
    handler > 1 && executable_user_page
}
