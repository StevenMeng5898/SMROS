// Per-process handle slots.
//
// A user-visible handle is either an encoded slot (index + generation) or the
// object id of a kernel object adopted into the same table (channels, Linux
// description handles). Closing a slot bumps its generation, so a reused slot
// rejects the previous encoded value. The kernel table used by EL1 is separate
// and is not represented here.

pub const PROCESS_HANDLE_SLOTS: usize = 1024;
pub const PROCESS_HANDLE_INDEX_BITS: u32 = 10;
pub const PROCESS_HANDLE_PREFIX: u32 = 0x4000_0000;
pub const PROCESS_HANDLE_PREFIX_MASK: u32 = 0xC000_0000;
const PROCESS_HANDLE_INDEX_MASK: u32 = (1 << PROCESS_HANDLE_INDEX_BITS) - 1;
const PROCESS_HANDLE_GENERATION_MAX: u32 = (1 << (30 - PROCESS_HANDLE_INDEX_BITS)) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleNamespaceError {
    Missing,
    WrongType,
    AccessDenied,
    Full,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandleSlot<T> {
    pub generation: u32,
    pub value: Option<T>,
}

impl<T> HandleSlot<T> {
    const fn empty() -> Self {
        Self {
            generation: 1,
            value: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandleNamespace<T> {
    slots: Vec<HandleSlot<T>>,
    limit: usize,
}

impl<T> HandleNamespace<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            limit,
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|slot| slot.value.is_none())
    }

    pub fn free_slots(&self) -> usize {
        let holes = self.slots.iter().filter(|slot| slot.value.is_none()).count();
        let unallocated = self.limit.saturating_sub(self.slots.len());
        holes + unallocated
    }

    pub fn slots(&self) -> &[HandleSlot<T>] {
        &self.slots
    }

    pub fn try_clone(&self) -> Result<Self, ()>
    where
        T: Clone,
    {
        let mut slots = Vec::new();
        slots.try_reserve_exact(self.slots.len()).map_err(|_| ())?;
        slots.extend_from_slice(&self.slots);
        Ok(Self {
            slots,
            limit: self.limit,
        })
    }
}

impl<T: Copy> HandleNamespace<T> {
    pub fn lookup(&self, handle: u32, object_id: impl Fn(&T) -> u32) -> Option<&T> {
        if let Some((index, generation)) = decode_process_handle(handle) {
            let slot = self.slots.get(index)?;
            if slot.generation != generation {
                return None;
            }
            return slot.value.as_ref();
        }
        self.slots.iter().find_map(|slot| {
            slot.value
                .as_ref()
                .filter(|value| object_id(value) == handle)
        })
    }

    pub fn insert_encoded(&mut self, make: impl FnOnce(u32) -> T) -> Result<u32, HandleNamespaceError> {
        let index = self.reserve_slot()?;
        let generation = self.slots[index].generation;
        let handle = encode_process_handle(index, generation).ok_or(HandleNamespaceError::Invalid)?;
        self.slots[index].value = Some(make(handle));
        Ok(handle)
    }

    pub fn insert_external(&mut self, value: T) -> Result<(), HandleNamespaceError> {
        let index = self.reserve_slot()?;
        self.slots[index].value = Some(value);
        Ok(())
    }

    pub fn close(&mut self, handle: u32, object_id: impl Fn(&T) -> u32) -> Result<T, HandleNamespaceError> {
        let index = if let Some((index, generation)) = decode_process_handle(handle) {
            let slot = self.slots.get(index).ok_or(HandleNamespaceError::Missing)?;
            if slot.generation != generation || slot.value.is_none() {
                return Err(HandleNamespaceError::Missing);
            }
            index
        } else {
            self.slots
                .iter()
                .position(|slot| {
                    slot.value
                        .as_ref()
                        .is_some_and(|value| object_id(value) == handle)
                })
                .ok_or(HandleNamespaceError::Missing)?
        };
        let value = self.slots[index].value.take().ok_or(HandleNamespaceError::Missing)?;
        self.slots[index].generation = next_generation(self.slots[index].generation);
        Ok(value)
    }

    /// Move handles into `dest`. `dest` receives new encoded values whose objects
    /// are produced by `reborn`. On `Full` or any later failure, both tables match
    /// their inputs.
    pub fn transfer_to(
        &mut self,
        dest: &mut Self,
        handles: &[u32],
        object_id: impl Fn(&T) -> u32,
        reborn: impl Fn(&T, u32) -> T,
    ) -> Result<Vec<u32>, HandleNamespaceError> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let mut moved = Vec::new();
        moved
            .try_reserve_exact(handles.len())
            .map_err(|_| HandleNamespaceError::Full)?;
        for handle in handles {
            let value = *self
                .lookup(*handle, &object_id)
                .ok_or(HandleNamespaceError::Missing)?;
            moved.push(value);
        }
        if dest.free_slots() < handles.len() {
            return Err(HandleNamespaceError::Full);
        }
        let mut created = Vec::new();
        created
            .try_reserve_exact(handles.len())
            .map_err(|_| HandleNamespaceError::Full)?;
        for value in &moved {
            match dest.insert_encoded(|user| reborn(value, user)) {
                Ok(user) => created.push(user),
                Err(error) => {
                    for user in created.iter().rev() {
                        let _ = dest.close(*user, &object_id);
                    }
                    return Err(error);
                }
            }
        }
        for handle in handles {
            let _ = self.close(*handle, &object_id);
        }
        Ok(created)
    }

    /// Same-table transfer. New encoded values are installed before the sources
    /// are closed. A full table leaves every source in place.
    pub fn transfer_within(
        &mut self,
        handles: &[u32],
        object_id: impl Fn(&T) -> u32,
        reborn: impl Fn(&T, u32) -> T,
    ) -> Result<Vec<u32>, HandleNamespaceError> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let mut moved = Vec::new();
        moved
            .try_reserve_exact(handles.len())
            .map_err(|_| HandleNamespaceError::Full)?;
        for handle in handles {
            let value = *self
                .lookup(*handle, &object_id)
                .ok_or(HandleNamespaceError::Missing)?;
            moved.push(value);
        }
        if self.free_slots() < handles.len() {
            return Err(HandleNamespaceError::Full);
        }
        let mut created = Vec::new();
        created
            .try_reserve_exact(handles.len())
            .map_err(|_| HandleNamespaceError::Full)?;
        for value in &moved {
            match self.insert_encoded(|user| reborn(value, user)) {
                Ok(user) => created.push(user),
                Err(error) => {
                    for user in created.iter().rev() {
                        let _ = self.close(*user, &object_id);
                    }
                    return Err(error);
                }
            }
        }
        for handle in handles {
            let _ = self.close(*handle, &object_id);
        }
        Ok(created)
    }

    fn reserve_slot(&mut self) -> Result<usize, HandleNamespaceError> {
        if let Some(index) = self.slots.iter().position(|slot| slot.value.is_none()) {
            return Ok(index);
        }
        if self.slots.len() >= self.limit {
            return Err(HandleNamespaceError::Full);
        }
        self.slots
            .try_reserve(1)
            .map_err(|_| HandleNamespaceError::Full)?;
        self.slots.push(HandleSlot::empty());
        Ok(self.slots.len() - 1)
    }
}

pub fn encode_process_handle(index: usize, generation: u32) -> Option<u32> {
    if index >= PROCESS_HANDLE_SLOTS || generation == 0 || generation > PROCESS_HANDLE_GENERATION_MAX
    {
        return None;
    }
    Some(PROCESS_HANDLE_PREFIX | (generation << PROCESS_HANDLE_INDEX_BITS) | (index as u32))
}

pub fn decode_process_handle(value: u32) -> Option<(usize, u32)> {
    if value & PROCESS_HANDLE_PREFIX_MASK != PROCESS_HANDLE_PREFIX {
        return None;
    }
    let index = (value & PROCESS_HANDLE_INDEX_MASK) as usize;
    let generation = (value & !PROCESS_HANDLE_PREFIX_MASK) >> PROCESS_HANDLE_INDEX_BITS;
    if index >= PROCESS_HANDLE_SLOTS || generation == 0 {
        return None;
    }
    Some((index, generation))
}

pub fn authorize_handle_use(
    found: bool,
    actual_type: u8,
    expected_type: u8,
    rights: u32,
    required_rights: u32,
) -> Result<(), HandleNamespaceError> {
    if !found {
        return Err(HandleNamespaceError::Missing);
    }
    if actual_type != expected_type {
        return Err(HandleNamespaceError::WrongType);
    }
    if rights & required_rights != required_rights {
        return Err(HandleNamespaceError::AccessDenied);
    }
    Ok(())
}

fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 || next > PROCESS_HANDLE_GENERATION_MAX {
        1
    } else {
        next
    }
}

pub fn fd_slot_available(open_fds: usize, limit: usize, replacing: bool) -> bool {
    replacing || open_fds < limit
}
