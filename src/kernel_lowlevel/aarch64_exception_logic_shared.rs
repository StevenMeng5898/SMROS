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
    Unsupported,
}

pub(crate) fn aarch64_lower_el_sync(esr: u64) -> Aarch64LowerElSync {
    let ec = (esr >> 26) & 0x3f;
    if ec == 0x15 {
        return Aarch64LowerElSync::Svc;
    }

    let access = match ec {
        0x20 => Aarch64El0MemoryAccess::Execute,
        0x24 if esr & (1 << 6) != 0 => Aarch64El0MemoryAccess::Write,
        0x24 => Aarch64El0MemoryAccess::Read,
        _ => return Aarch64LowerElSync::Unsupported,
    };
    let kind = match esr & 0x3f {
        0x04..=0x07 => Aarch64El0AbortKind::Translation,
        0x08..=0x0b => Aarch64El0AbortKind::AccessFlag,
        0x0c..=0x0f => Aarch64El0AbortKind::Permission,
        _ => return Aarch64LowerElSync::Unsupported,
    };

    Aarch64LowerElSync::MemoryFault(Aarch64El0MemoryFault { access, kind })
}
