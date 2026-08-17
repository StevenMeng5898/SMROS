use alloc::string::String;
use alloc::vec::Vec;

pub(crate) const LINUX_MQ_PRIO_MAX: usize = 32_768;
pub(crate) const LINUX_MQ_DEFAULT_MAXMSG: usize = 10;
pub(crate) const LINUX_MQ_DEFAULT_MSGSIZE: usize = 8_192;
pub(crate) const LINUX_MQ_MAXMSG_LIMIT: usize = 1_024;
pub(crate) const LINUX_MQ_MSGSIZE_LIMIT: usize = 8_192;
pub(crate) const LINUX_MQ_NAME_MAX: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueMessage {
    pub bytes: Vec<u8>,
    pub priority: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueAttr {
    pub flags: usize,
    pub maxmsg: usize,
    pub msgsize: usize,
    pub curmsgs: usize,
}

impl LinuxMqueueAttr {
    pub(crate) const fn defaults() -> Self {
        Self {
            flags: 0,
            maxmsg: LINUX_MQ_DEFAULT_MAXMSG,
            msgsize: LINUX_MQ_DEFAULT_MSGSIZE,
            curmsgs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueOpen {
    pub created: bool,
    pub attr: LinuxMqueueAttr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueSendOutcome {
    pub receiver: Option<(usize, usize)>,
    pub notification: Option<LinuxMqueueNotification>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueReceiveOutcome {
    pub message: LinuxMqueueMessage,
    pub sender: Option<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueNotification {
    pub handle: u32,
    pub pid: usize,
    pub signum: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMqueueDeadline {
    pub ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxMqueueWaitKind {
    Send,
    Receive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxMqueueWaitOutcome {
    Waiting,
    Woken,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxMqueueError {
    BadDescriptor,
    Capacity,
    Exists,
    Invalid,
    MessageTooLarge,
    NameTooLong,
    NotFound,
    WouldBlock,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinuxMqueueQueuedMessage {
    message: LinuxMqueueMessage,
    sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LinuxMqueueQueue {
    id: u64,
    name: Option<String>,
    handles: Vec<u32>,
    attr: LinuxMqueueAttr,
    messages: Vec<LinuxMqueueQueuedMessage>,
    notification: Option<LinuxMqueueNotification>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinuxMqueueWaiter {
    queue_id: u64,
    kind: LinuxMqueueWaitKind,
    tid: usize,
    scheduler_thread: usize,
    deadline: Option<LinuxMqueueDeadline>,
    sequence: u64,
    outcome: LinuxMqueueWaitOutcome,
}

pub(crate) struct LinuxMqueueState<const Q: usize, const H: usize, const W: usize> {
    queues: Vec<LinuxMqueueQueue>,
    waiters: [Option<LinuxMqueueWaiter>; W],
    next_queue_id: u64,
    next_message_sequence: u64,
    next_waiter_sequence: u64,
    queue_id_exhausted: bool,
    message_sequence_exhausted: bool,
    waiter_sequence_exhausted: bool,
}

impl<const Q: usize, const H: usize, const W: usize> LinuxMqueueState<Q, H, W> {
    pub(crate) const fn new() -> Self {
        Self {
            queues: Vec::new(),
            waiters: [None; W],
            next_queue_id: 1,
            next_message_sequence: 0,
            next_waiter_sequence: 0,
            queue_id_exhausted: false,
            message_sequence_exhausted: false,
            waiter_sequence_exhausted: false,
        }
    }

    pub(crate) fn open(
        &mut self,
        name: &str,
        handle: u32,
        create: bool,
        exclusive: bool,
        attr: Option<LinuxMqueueAttr>,
    ) -> Result<LinuxMqueueOpen, LinuxMqueueError> {
        let name = linux_mqueue_name_component(name)?;
        self.validate_new_handle(handle)?;

        if let Some(index) = self.queue_index_by_name(name) {
            if create && exclusive {
                return Err(LinuxMqueueError::Exists);
            }
            self.attach_handle(index, handle)?;
            let mut opened = self.queues[index].attr;
            opened.curmsgs = self.queues[index].messages.len();
            return Ok(LinuxMqueueOpen {
                created: false,
                attr: opened,
            });
        }

        if !create {
            return Err(LinuxMqueueError::NotFound);
        }
        if self.queues.len() >= Q || self.queue_id_exhausted {
            return Err(LinuxMqueueError::Capacity);
        }

        let mut created_attr = attr.unwrap_or_else(LinuxMqueueAttr::defaults);
        created_attr = validate_linux_mqueue_attr(created_attr)?;
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(1)
            .map_err(|_| LinuxMqueueError::Capacity)?;
        handles.push(handle);

        let mut queue_name = String::new();
        queue_name
            .try_reserve_exact(name.len())
            .map_err(|_| LinuxMqueueError::Capacity)?;
        queue_name.push_str(name);

        self.queues
            .try_reserve_exact(1)
            .map_err(|_| LinuxMqueueError::Capacity)?;
        let queue_id = self.next_queue_id;
        match self.next_queue_id.checked_add(1) {
            Some(next) => self.next_queue_id = next,
            None => self.queue_id_exhausted = true,
        }
        self.queues.push(LinuxMqueueQueue {
            id: queue_id,
            name: Some(queue_name),
            handles,
            attr: created_attr,
            messages: Vec::new(),
            notification: None,
        });

        Ok(LinuxMqueueOpen {
            created: true,
            attr: created_attr,
        })
    }

    pub(crate) fn close_handle(&mut self, handle: u32) -> bool {
        let Some(index) = self.queue_index_by_handle(handle) else {
            return false;
        };
        let queue_id = self.queues[index].id;
        if let Some(handle_index) = self.queues[index]
            .handles
            .iter()
            .position(|current| *current == handle)
        {
            self.queues[index].handles.swap_remove(handle_index);
        }
        if self.queues[index]
            .notification
            .is_some_and(|notification| notification.handle == handle)
        {
            self.queues[index].notification = None;
        }
        if self.queues[index].handles.is_empty() && self.queues[index].name.is_none() {
            self.queues.swap_remove(index);
            self.remove_queue_waiters(queue_id);
        }
        true
    }

    pub(crate) fn unlink(&mut self, name: &str) -> Result<(), LinuxMqueueError> {
        let name = linux_mqueue_name_component(name)?;
        let index = self
            .queue_index_by_name(name)
            .ok_or(LinuxMqueueError::NotFound)?;
        let queue_id = self.queues[index].id;
        self.queues[index].name = None;
        if self.queues[index].handles.is_empty() {
            self.queues.swap_remove(index);
            self.remove_queue_waiters(queue_id);
        }
        Ok(())
    }

    pub(crate) fn getattr(
        &self,
        handle: u32,
        flags: usize,
    ) -> Result<LinuxMqueueAttr, LinuxMqueueError> {
        let queue = self
            .queue_by_handle(handle)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        Ok(LinuxMqueueAttr {
            flags,
            curmsgs: queue.messages.len(),
            ..queue.attr
        })
    }

    pub(crate) fn notify(
        &mut self,
        handle: u32,
        notification: Option<LinuxMqueueNotification>,
    ) -> Result<(), LinuxMqueueError> {
        let index = self
            .queue_index_by_handle(handle)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        match notification {
            Some(notification) => {
                if self.queues[index].notification.is_some() {
                    return Err(LinuxMqueueError::Busy);
                }
                self.queues[index].notification = Some(notification);
            }
            None => {
                self.queues[index].notification = None;
            }
        }
        Ok(())
    }

    pub(crate) fn send(
        &mut self,
        handle: u32,
        bytes: &[u8],
        priority: usize,
    ) -> Result<LinuxMqueueSendOutcome, LinuxMqueueError> {
        let index = self
            .queue_index_by_handle(handle)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        if priority >= LINUX_MQ_PRIO_MAX {
            return Err(LinuxMqueueError::Invalid);
        }
        if bytes.len() > self.queues[index].attr.msgsize {
            return Err(LinuxMqueueError::MessageTooLarge);
        }
        if self.queues[index].messages.len() >= self.queues[index].attr.maxmsg {
            return Err(LinuxMqueueError::WouldBlock);
        }
        if self.message_sequence_exhausted {
            return Err(LinuxMqueueError::Capacity);
        }

        let mut stored = Vec::new();
        stored
            .try_reserve_exact(bytes.len())
            .map_err(|_| LinuxMqueueError::Capacity)?;
        stored.extend_from_slice(bytes);

        let was_empty = self.queues[index].messages.is_empty();
        let queue_id = self.queues[index].id;
        let sequence = self.next_message_sequence;
        match self.next_message_sequence.checked_add(1) {
            Some(next) => self.next_message_sequence = next,
            None => self.message_sequence_exhausted = true,
        }
        self.queues[index].messages.push(LinuxMqueueQueuedMessage {
            message: LinuxMqueueMessage {
                bytes: stored,
                priority,
            },
            sequence,
        });

        let receiver = self.wake_one(queue_id, LinuxMqueueWaitKind::Receive);
        let notification = if was_empty && receiver.is_none() {
            self.queues[index].notification.take()
        } else {
            None
        };
        Ok(LinuxMqueueSendOutcome {
            receiver,
            notification,
        })
    }

    pub(crate) fn receive(
        &mut self,
        handle: u32,
        buffer_len: usize,
    ) -> Result<LinuxMqueueReceiveOutcome, LinuxMqueueError> {
        let index = self
            .queue_index_by_handle(handle)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        if buffer_len < self.queues[index].attr.msgsize {
            return Err(LinuxMqueueError::MessageTooLarge);
        }
        let message_index = self
            .best_message_index(index)
            .ok_or(LinuxMqueueError::WouldBlock)?;
        let queue_id = self.queues[index].id;
        let message = self.queues[index].messages.swap_remove(message_index).message;
        let sender = self.wake_one(queue_id, LinuxMqueueWaitKind::Send);
        Ok(LinuxMqueueReceiveOutcome { message, sender })
    }

    pub(crate) fn ready(
        &self,
        handle: u32,
        kind: LinuxMqueueWaitKind,
    ) -> Result<bool, LinuxMqueueError> {
        let queue = self
            .queue_by_handle(handle)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        Ok(match kind {
            LinuxMqueueWaitKind::Send => queue.messages.len() < queue.attr.maxmsg,
            LinuxMqueueWaitKind::Receive => !queue.messages.is_empty(),
        })
    }

    pub(crate) fn push_waiter(
        &mut self,
        handle: u32,
        kind: LinuxMqueueWaitKind,
        tid: usize,
        scheduler_thread: usize,
        deadline: Option<LinuxMqueueDeadline>,
    ) -> Result<(), LinuxMqueueError> {
        if tid == 0 {
            return Err(LinuxMqueueError::Invalid);
        }
        let queue_id = self
            .queue_by_handle(handle)
            .map(|queue| queue.id)
            .ok_or(LinuxMqueueError::BadDescriptor)?;
        if self.waiter_sequence_exhausted {
            return Err(LinuxMqueueError::Capacity);
        }
        if self.waiters.iter().flatten().any(|waiter| {
            waiter.tid == tid
                && waiter.scheduler_thread == scheduler_thread
                && waiter.outcome == LinuxMqueueWaitOutcome::Waiting
        }) {
            return Err(LinuxMqueueError::Busy);
        }
        let Some(slot) = self.waiters.iter_mut().find(|slot| slot.is_none()) else {
            return Err(LinuxMqueueError::Capacity);
        };
        let sequence = self.next_waiter_sequence;
        match self.next_waiter_sequence.checked_add(1) {
            Some(next) => self.next_waiter_sequence = next,
            None => self.waiter_sequence_exhausted = true,
        }
        *slot = Some(LinuxMqueueWaiter {
            queue_id,
            kind,
            tid,
            scheduler_thread,
            deadline,
            sequence,
            outcome: LinuxMqueueWaitOutcome::Waiting,
        });
        Ok(())
    }

    pub(crate) fn expire(&mut self, now: u64) -> [Option<(usize, usize)>; W] {
        let mut identities = [None; W];
        let mut selected_count = 0usize;
        while selected_count < W {
            let Some(index) = self.oldest_waiter_index(|waiter| {
                waiter.outcome == LinuxMqueueWaitOutcome::Waiting
                    && waiter
                        .deadline
                        .is_some_and(|deadline| deadline.ticks <= now)
            }) else {
                break;
            };
            let waiter = self.waiters[index].as_mut().expect("selected mqueue waiter");
            waiter.outcome = LinuxMqueueWaitOutcome::TimedOut;
            identities[selected_count] = Some((waiter.tid, waiter.scheduler_thread));
            selected_count += 1;
        }
        identities
    }

    pub(crate) fn interrupt(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(waiter) = self.waiters.iter_mut().flatten().find(|waiter| {
            waiter.tid == tid
                && waiter.scheduler_thread == scheduler_thread
                && waiter.outcome == LinuxMqueueWaitOutcome::Waiting
        }) else {
            return false;
        };
        waiter.outcome = LinuxMqueueWaitOutcome::Interrupted;
        true
    }

    pub(crate) fn take_outcome(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<LinuxMqueueWaitOutcome> {
        let slot = self.waiters.iter_mut().find(|slot| {
            slot.is_some_and(|waiter| {
                waiter.tid == tid
                    && waiter.scheduler_thread == scheduler_thread
                    && waiter.outcome != LinuxMqueueWaitOutcome::Waiting
            })
        })?;
        slot.take().map(|waiter| waiter.outcome)
    }

    pub(crate) fn remove_task(&mut self, tid: usize, scheduler_thread: usize) -> usize {
        let mut removed = 0usize;
        for slot in &mut self.waiters {
            if slot.is_some_and(|waiter| {
                waiter.tid == tid && waiter.scheduler_thread == scheduler_thread
            }) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    pub(crate) fn reset(&mut self) {
        self.queues.clear();
        self.waiters.fill(None);
        self.next_queue_id = 1;
        self.next_message_sequence = 0;
        self.next_waiter_sequence = 0;
        self.queue_id_exhausted = false;
        self.message_sequence_exhausted = false;
        self.waiter_sequence_exhausted = false;
    }

    fn validate_new_handle(&self, handle: u32) -> Result<(), LinuxMqueueError> {
        if handle == 0 || self.open_handle_count() >= H || self.queue_index_by_handle(handle).is_some()
        {
            return Err(LinuxMqueueError::Capacity);
        }
        Ok(())
    }

    fn attach_handle(&mut self, index: usize, handle: u32) -> Result<(), LinuxMqueueError> {
        self.queues[index]
            .handles
            .try_reserve_exact(1)
            .map_err(|_| LinuxMqueueError::Capacity)?;
        self.queues[index].handles.push(handle);
        Ok(())
    }

    fn open_handle_count(&self) -> usize {
        self.queues.iter().map(|queue| queue.handles.len()).sum()
    }

    fn queue_by_handle(&self, handle: u32) -> Option<&LinuxMqueueQueue> {
        let index = self.queue_index_by_handle(handle)?;
        self.queues.get(index)
    }

    fn queue_index_by_handle(&self, handle: u32) -> Option<usize> {
        self.queues
            .iter()
            .position(|queue| queue.handles.iter().any(|current| *current == handle))
    }

    fn queue_index_by_name(&self, name: &str) -> Option<usize> {
        self.queues
            .iter()
            .position(|queue| queue.name.as_deref() == Some(name))
    }

    fn best_message_index(&self, queue_index: usize) -> Option<usize> {
        let mut selected = None;
        let mut selected_priority = 0usize;
        let mut selected_sequence = u64::MAX;
        for (index, message) in self.queues[queue_index].messages.iter().enumerate() {
            if selected.is_none()
                || message.message.priority > selected_priority
                || (message.message.priority == selected_priority
                    && message.sequence < selected_sequence)
            {
                selected = Some(index);
                selected_priority = message.message.priority;
                selected_sequence = message.sequence;
            }
        }
        selected
    }

    fn wake_one(
        &mut self,
        queue_id: u64,
        kind: LinuxMqueueWaitKind,
    ) -> Option<(usize, usize)> {
        let index = self.oldest_waiter_index(|waiter| {
            waiter.queue_id == queue_id
                && waiter.kind == kind
                && waiter.outcome == LinuxMqueueWaitOutcome::Waiting
        })?;
        let waiter = self.waiters[index].as_mut().expect("selected mqueue waiter");
        waiter.outcome = LinuxMqueueWaitOutcome::Woken;
        Some((waiter.tid, waiter.scheduler_thread))
    }

    fn oldest_waiter_index(&self, predicate: impl Fn(LinuxMqueueWaiter) -> bool) -> Option<usize> {
        let mut selected = None;
        let mut selected_sequence = u64::MAX;
        for (index, waiter) in self.waiters.iter().enumerate() {
            let Some(waiter) = waiter else {
                continue;
            };
            if predicate(*waiter) && (selected.is_none() || waiter.sequence < selected_sequence) {
                selected = Some(index);
                selected_sequence = waiter.sequence;
            }
        }
        selected
    }

    fn remove_queue_waiters(&mut self, queue_id: u64) -> usize {
        let mut removed = 0usize;
        for slot in &mut self.waiters {
            if slot.is_some_and(|waiter| waiter.queue_id == queue_id) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }
}

pub(crate) fn linux_mqueue_name_component(name: &str) -> Result<&str, LinuxMqueueError> {
    let component = name.strip_prefix('/').unwrap_or(name);
    if component.is_empty() {
        return Err(LinuxMqueueError::Invalid);
    }
    if component.len() > LINUX_MQ_NAME_MAX {
        return Err(LinuxMqueueError::NameTooLong);
    }
    if component.as_bytes().iter().any(|byte| *byte == b'/') {
        return Err(LinuxMqueueError::Invalid);
    }
    Ok(component)
}

pub(crate) fn validate_linux_mqueue_attr(
    attr: LinuxMqueueAttr,
) -> Result<LinuxMqueueAttr, LinuxMqueueError> {
    if attr.maxmsg == 0
        || attr.msgsize == 0
        || attr.maxmsg > LINUX_MQ_MAXMSG_LIMIT
        || attr.msgsize > LINUX_MQ_MSGSIZE_LIMIT
    {
        return Err(LinuxMqueueError::Invalid);
    }
    Ok(LinuxMqueueAttr {
        flags: 0,
        curmsgs: 0,
        ..attr
    })
}
