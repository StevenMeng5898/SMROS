use core::sync::atomic::{AtomicUsize, Ordering};

const EMPTY_FRAME: usize = 0;
const INSTALLING_FRAME: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSyscallFrameSnapshot {
    pub frame: usize,
    pub return_pc: usize,
    pub pstate: usize,
}

pub(crate) struct LinuxSyscallFrameOwners<const N: usize> {
    frames: [AtomicUsize; N],
    return_pcs: [AtomicUsize; N],
    pstates: [AtomicUsize; N],
}

impl<const N: usize> LinuxSyscallFrameOwners<N> {
    pub(crate) const fn new() -> Self {
        Self {
            frames: [const { AtomicUsize::new(EMPTY_FRAME) }; N],
            return_pcs: [const { AtomicUsize::new(0) }; N],
            pstates: [const { AtomicUsize::new(0) }; N],
        }
    }

    pub(crate) fn install(
        &self,
        owner: usize,
        frame: usize,
        return_pc: usize,
        pstate: usize,
    ) -> bool {
        if owner == 0 || frame <= INSTALLING_FRAME {
            return false;
        }
        let Some(frame_slot) = self.frames.get(owner) else {
            return false;
        };
        if frame_slot
            .compare_exchange(
                EMPTY_FRAME,
                INSTALLING_FRAME,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }

        self.return_pcs[owner].store(return_pc, Ordering::Relaxed);
        self.pstates[owner].store(pstate, Ordering::Relaxed);
        frame_slot.store(frame, Ordering::Release);
        true
    }

    pub(crate) fn current(&self, owner: usize) -> Option<LinuxSyscallFrameSnapshot> {
        if owner == 0 {
            return None;
        }
        let frame_slot = self.frames.get(owner)?;
        let frame = frame_slot.load(Ordering::Acquire);
        if frame <= INSTALLING_FRAME {
            return None;
        }
        let snapshot = LinuxSyscallFrameSnapshot {
            frame,
            return_pc: self.return_pcs[owner].load(Ordering::Relaxed),
            pstate: self.pstates[owner].load(Ordering::Relaxed),
        };
        (frame_slot.load(Ordering::Acquire) == frame).then_some(snapshot)
    }

    pub(crate) fn clear(&self, owner: usize, frame: usize) -> bool {
        if owner == 0 || frame <= INSTALLING_FRAME {
            return false;
        }
        let Some(frame_slot) = self.frames.get(owner) else {
            return false;
        };
        if frame_slot
            .compare_exchange(frame, INSTALLING_FRAME, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.return_pcs[owner].store(0, Ordering::Relaxed);
        self.pstates[owner].store(0, Ordering::Relaxed);
        frame_slot.store(EMPTY_FRAME, Ordering::Release);
        true
    }

    pub(crate) fn clear_all(&self) {
        for owner in 1..N {
            let frame_slot = &self.frames[owner];
            loop {
                let frame = frame_slot.load(Ordering::Acquire);
                if frame == EMPTY_FRAME {
                    break;
                }
                if frame == INSTALLING_FRAME {
                    core::hint::spin_loop();
                    continue;
                }
                if frame_slot
                    .compare_exchange(frame, INSTALLING_FRAME, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    continue;
                }
                self.return_pcs[owner].store(0, Ordering::Relaxed);
                self.pstates[owner].store(0, Ordering::Relaxed);
                frame_slot.store(EMPTY_FRAME, Ordering::Release);
                break;
            }
        }
    }
}
