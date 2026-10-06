#![allow(dead_code)]
#![allow(static_mut_refs)]

//! Linux `pci_*` C ABI used by `linuxcompat/pci.c` and stock PCI driver C.

use super::{
    cstr, ddk_errno, ioremap, iounmap, push_binding, registry, ModuleInfo, IORESOURCE_IO,
    PCI_ANY_ID,
};
use crate::user_level::drivers::linux_logic;
use crate::user_level::drivers::{UserDeviceKind, UserDriverError};
use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

pub const PCI_NUM_RESOURCES: usize = 6;
const MAX_PCI_C_DRIVERS: usize = 4;
const MAX_PCI_C_DEVS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DeviceC {
    pub driver_data: *mut c_void,
    pub init_name: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PciResourceC {
    pub start: u64,
    pub end: u64,
    pub name: *const c_char,
    pub flags: c_ulong,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PciDevC {
    pub dev: DeviceC,
    pub devfn: c_uint,
    pub vendor: u16,
    pub device: u16,
    pub subsystem_vendor: u16,
    pub subsystem_device: u16,
    pub class: c_uint,
    pub revision: u8,
    pub irq: c_uint,
    pub resource: [PciResourceC; PCI_NUM_RESOURCES],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PciDeviceIdC {
    pub vendor: u32,
    pub device: u32,
    pub subvendor: u32,
    pub subdevice: u32,
    pub class: u32,
    pub class_mask: u32,
    pub driver_data: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PciDriverC {
    pub name: *const c_char,
    pub id_table: *const PciDeviceIdC,
    pub probe: Option<extern "C" fn(*mut PciDevC, *const PciDeviceIdC) -> c_int>,
    pub remove: Option<extern "C" fn(*mut PciDevC)>,
}

#[derive(Clone, Copy)]
struct PciSlot {
    used: bool,
    enabled: bool,
    regions: bool,
    master: bool,
    registry_index: usize,
    bus: u8,
    device_num: u8,
    function: u8,
    iomap: [usize; PCI_NUM_RESOURCES],
    iomap_size: [usize; PCI_NUM_RESOURCES],
    cdev: PciDevC,
}

impl PciSlot {
    const fn empty() -> Self {
        Self {
            used: false,
            enabled: false,
            regions: false,
            master: false,
            registry_index: 0,
            bus: 0,
            device_num: 0,
            function: 0,
            iomap: [0; PCI_NUM_RESOURCES],
            iomap_size: [0; PCI_NUM_RESOURCES],
            cdev: PciDevC {
                dev: DeviceC {
                    driver_data: core::ptr::null_mut(),
                    init_name: core::ptr::null(),
                },
                devfn: 0,
                vendor: 0,
                device: 0,
                subsystem_vendor: 0,
                subsystem_device: 0,
                class: 0,
                revision: 0,
                irq: 0,
                resource: [PciResourceC {
                    start: 0,
                    end: 0,
                    name: core::ptr::null(),
                    flags: 0,
                }; PCI_NUM_RESOURCES],
            },
        }
    }
}

static mut SLOTS: [PciSlot; MAX_PCI_C_DEVS] = [PciSlot::empty(); MAX_PCI_C_DEVS];
static mut C_DRIVERS: [Option<*const PciDriverC>; MAX_PCI_C_DRIVERS] = [None; MAX_PCI_C_DRIVERS];

pub fn reset() {
    unsafe {
        SLOTS = [PciSlot::empty(); MAX_PCI_C_DEVS];
        C_DRIVERS = [None; MAX_PCI_C_DRIVERS];
    }
}

pub fn driver_count() -> usize {
    unsafe { C_DRIVERS.iter().filter(|entry| entry.is_some()).count() }
}

pub fn attach() {
    let driver_ptrs: [*const PciDriverC; MAX_PCI_C_DRIVERS] = unsafe {
        let mut ptrs = [core::ptr::null(); MAX_PCI_C_DRIVERS];
        for (index, entry) in C_DRIVERS.iter().enumerate() {
            if let Some(drv) = entry {
                ptrs[index] = *drv;
            }
        }
        ptrs
    };
    for ptr in driver_ptrs {
        if !ptr.is_null() {
            attach_driver(ptr);
        }
    }
}

fn attach_driver(drv: *const PciDriverC) {
    let driver = unsafe { &*drv };
    let Some(name) = cstr(driver.name) else {
        return;
    };
    let Some(probe) = driver.probe else {
        return;
    };
    if driver.id_table.is_null() {
        return;
    }
    let device_count = registry().pci_devices.len();
    for device_index in 0..device_count {
        if registry().pci_devices[device_index].driver.is_some() {
            continue;
        }
        let device = registry().pci_devices[device_index];
        let Some(matched) = match_id_table(driver.id_table, &device) else {
            continue;
        };
        let Some(slot) = alloc_slot() else {
            continue;
        };
        fill_slot(slot, device_index, &device, driver.name);
        let pdev = unsafe { &raw mut SLOTS[slot].cdev };
        let status = probe(pdev, matched);
        if linux_logic::probe_status_ok(status) {
            registry().pci_devices[device_index].driver = Some(name);
            push_binding(
                device.node_path,
                name,
                device.name,
                UserDeviceKind::Pci,
                0,
                0,
                0,
                [0; 6],
            );
        } else {
            unsafe {
                SLOTS[slot] = PciSlot::empty();
            }
        }
    }
}

fn match_id_table(
    table: *const PciDeviceIdC,
    device: &super::PciDevice,
) -> Option<*const PciDeviceIdC> {
    let mut cursor = table;
    for _ in 0..32 {
        let id = unsafe { &*cursor };
        if linux_logic::id_table_end(id.vendor, id.device) {
            return None;
        }
        if id_matches(device, id) {
            return Some(cursor);
        }
        cursor = unsafe { cursor.add(1) };
    }
    None
}

fn id_matches(device: &super::PciDevice, id: &PciDeviceIdC) -> bool {
    if !linux_logic::pci_id_match(
        device.vendor,
        device.device,
        id.vendor,
        id.device,
        PCI_ANY_ID,
    ) {
        return false;
    }
    if id.subvendor != PCI_ANY_ID && id.subvendor != device.subsystem_vendor {
        return false;
    }
    if id.subdevice != PCI_ANY_ID && id.subdevice != device.subsystem_device {
        return false;
    }
    if id.class_mask != 0 && (device.class & id.class_mask) != (id.class & id.class_mask) {
        return false;
    }
    true
}

fn alloc_slot() -> Option<usize> {
    unsafe { SLOTS.iter().position(|slot| !slot.used) }
}

fn fill_slot(slot: usize, registry_index: usize, device: &super::PciDevice, name: *const c_char) {
    let entry = unsafe { &mut SLOTS[slot] };
    *entry = PciSlot::empty();
    entry.used = true;
    entry.registry_index = registry_index;
    entry.bus = device.bus;
    entry.device_num = pci_slot(device.devfn);
    entry.function = pci_func(device.devfn);
    entry.cdev.dev.init_name = name;
    entry.cdev.devfn = device.devfn as c_uint;
    entry.cdev.vendor = device.vendor as u16;
    entry.cdev.device = device.device as u16;
    entry.cdev.subsystem_vendor = device.subsystem_vendor as u16;
    entry.cdev.subsystem_device = device.subsystem_device as u16;
    entry.cdev.class = device.class;
    entry.cdev.revision = device.revision;
    entry.cdev.irq = device.irq.unwrap_or(0);
    for bar in 0..PCI_NUM_RESOURCES {
        let src = device.bars[bar];
        entry.cdev.resource[bar] = PciResourceC {
            start: src.start,
            end: if src.size == 0 {
                0
            } else {
                src.start.saturating_add(src.size.saturating_sub(1))
            },
            name,
            flags: src.flags as c_ulong,
        };
    }
}

const fn pci_slot(devfn: u8) -> u8 {
    (devfn >> 3) & 0x1f
}

const fn pci_func(devfn: u8) -> u8 {
    devfn & 0x07
}

fn slot_index(pdev: *const PciDevC) -> Option<usize> {
    if pdev.is_null() {
        return None;
    }
    let addr = pdev as usize;
    unsafe {
        SLOTS
            .iter()
            .position(|slot| slot.used && core::ptr::addr_of!(slot.cdev) as usize == addr)
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_register_driver(drv: *mut PciDriverC) -> c_int {
    super::init();
    if drv.is_null() {
        return -22;
    }
    let driver = unsafe { &*drv };
    let Some(name) = cstr(driver.name) else {
        return -22;
    };
    if driver.id_table.is_null() || driver.probe.is_none() {
        return -22;
    }
    let free = unsafe { C_DRIVERS.iter().position(|entry| entry.is_none()) };
    let Some(index) = free else {
        return -16;
    };
    unsafe {
        C_DRIVERS[index] = Some(drv);
    }
    let reg = registry();
    if !reg.modules.iter().any(|module| module.name == name) {
        reg.modules.push(ModuleInfo {
            name,
            license: "GPL",
            description: "Linux PCI driver",
        });
    }
    0
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_unregister_driver(drv: *mut PciDriverC) {
    if drv.is_null() {
        return;
    }
    unsafe {
        for entry in C_DRIVERS.iter_mut() {
            if *entry == Some(drv) {
                *entry = None;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_enable_device(pdev: *mut PciDevC) -> c_int {
    let Some(index) = slot_index(pdev) else {
        return -19;
    };
    let slot = unsafe { &mut SLOTS[index] };
    crate::user_level::drivers::pci::enable_function(slot.bus, slot.device_num, slot.function);
    slot.enabled = true;
    0
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_disable_device(pdev: *mut PciDevC) {
    if let Some(index) = slot_index(pdev) {
        unsafe {
            SLOTS[index].enabled = false;
            SLOTS[index].master = false;
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_request_regions(
    pdev: *mut PciDevC,
    res_name: *const c_char,
) -> c_int {
    let Some(index) = slot_index(pdev) else {
        return -19;
    };
    let slot = unsafe { &mut SLOTS[index] };
    if slot.regions {
        return ddk_errno(UserDriverError::Busy);
    }
    if !res_name.is_null() {
        for bar in slot.cdev.resource.iter_mut() {
            if bar.start != 0 || bar.end != 0 {
                bar.name = res_name;
            }
        }
    }
    slot.regions = true;
    0
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_release_regions(pdev: *mut PciDevC) {
    if let Some(index) = slot_index(pdev) {
        unsafe {
            SLOTS[index].regions = false;
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_set_master(pdev: *mut PciDevC, enable: c_int) {
    if let Some(index) = slot_index(pdev) {
        unsafe {
            SLOTS[index].master = enable != 0;
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_iomap(
    pdev: *mut PciDevC,
    bar: c_int,
    maxlen: c_ulong,
) -> *mut c_void {
    let Some(index) = slot_index(pdev) else {
        return core::ptr::null_mut();
    };
    if bar < 0 || bar as usize >= PCI_NUM_RESOURCES {
        return core::ptr::null_mut();
    }
    let bar = bar as usize;
    let slot = unsafe { &mut SLOTS[index] };
    let start = slot.cdev.resource[bar].start;
    let mut len = if start == 0 && slot.cdev.resource[bar].end == 0 {
        0
    } else {
        slot.cdev.resource[bar]
            .end
            .saturating_sub(start)
            .saturating_add(1)
    };
    if len == 0 {
        return core::ptr::null_mut();
    }
    if maxlen != 0 && (maxlen as u64) < len {
        len = maxlen as u64;
    }
    match ioremap(start, len as usize) {
        Ok(virt) => {
            slot.iomap[bar] = virt;
            slot.iomap_size[bar] = len as usize;
            virt as *mut c_void
        }
        Err(_) => core::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_iounmap(pdev: *mut PciDevC, addr: *mut c_void) {
    let Some(index) = slot_index(pdev) else {
        return;
    };
    let slot = unsafe { &mut SLOTS[index] };
    let virt = addr as usize;
    for bar in 0..PCI_NUM_RESOURCES {
        if slot.iomap[bar] == virt && virt != 0 {
            iounmap(virt, slot.iomap_size[bar]);
            slot.iomap[bar] = 0;
            slot.iomap_size[bar] = 0;
        }
    }
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_read_config(
    pdev: *const PciDevC,
    where_: c_int,
    size: c_int,
    val: *mut u32,
) -> c_int {
    if val.is_null() || where_ < 0 {
        return -22;
    }
    let Some(index) = slot_index(pdev) else {
        return -19;
    };
    let slot = unsafe { &SLOTS[index] };
    let offset = where_ as u16;
    let hw = crate::user_level::drivers::pci::config_read(
        slot.bus,
        slot.device_num,
        slot.function,
        offset,
        size as u8,
    );
    let value = hw.unwrap_or_else(|| synthesize_config(slot, offset, size as u8));
    unsafe {
        *val = value;
    }
    0
}

#[no_mangle]
pub extern "C" fn smros_ddk_pci_write_config(
    pdev: *mut PciDevC,
    where_: c_int,
    size: c_int,
    val: u32,
) -> c_int {
    if where_ < 0 {
        return -22;
    }
    let Some(index) = slot_index(pdev) else {
        return -19;
    };
    let slot = unsafe { &SLOTS[index] };
    if crate::user_level::drivers::pci::config_write(
        slot.bus,
        slot.device_num,
        slot.function,
        where_ as u16,
        size as u8,
        val,
    ) {
        0
    } else {
        0
    }
}

fn synthesize_config(slot: &PciSlot, offset: u16, size: u8) -> u32 {
    let dword = match offset & !0x3 {
        0x00 => (slot.cdev.vendor as u32) | ((slot.cdev.device as u32) << 16),
        0x08 => (slot.cdev.revision as u32) | (slot.cdev.class << 8),
        0x2c => (slot.cdev.subsystem_vendor as u32) | ((slot.cdev.subsystem_device as u32) << 16),
        0x3c => slot.cdev.irq,
        bar @ 0x10..=0x24 if (bar - 0x10) % 4 == 0 => {
            let index = ((bar - 0x10) / 4) as usize;
            let res = slot.cdev.resource[index];
            let flags = if res.flags as u32 & IORESOURCE_IO != 0 {
                1
            } else {
                0
            };
            (res.start as u32 & !0xf) | flags
        }
        _ => 0,
    };
    let shift = (offset & 0x3) * 8;
    match size {
        1 => (dword >> shift) & 0xff,
        2 => (dword >> shift) & 0xffff,
        _ => dword,
    }
}
