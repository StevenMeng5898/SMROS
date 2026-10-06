# SMROS Linux User-Space DDK

SMROS now hosts a Linux-shaped Driver Development Kit in the user-level layer.
Drivers are not Linux kernel modules and do not run as isolated EL0 processes
yet. They register through a Linux device-model subset compiled into
`src/user_level/drivers/` and bind during `user_level::init()`.

The DDK exists so Linux driver demos can move onto a stable bus/device/driver
API instead of hard-wiring probe loops in `mod.rs`.

## Status

- Linux device model: module, bus, device, driver, `probe` / `remove`
- Buses: `platform`, `virtio`, `pci`
- Character/misc devices with `file_operations`
- MMIO (`ioremap`, `readl`/`writel`), IRQ, and coherent DMA helpers
- Live Linux driver demos:
  - `virtio_blk` (moved from the old hard-wired VirtIO-MMIO/PCI bind path)
  - `virtio_net` (same)
  - `hello` misc character driver (`/dev/hello`)
  - `dummy` platform driver (`compatible = "smros,dummy"`)
  - `edu` QEMU EDU PCI driver (stock Linux C, `vendor 0x1234` / `device 0x11e8`)
- C ABI and header: `include/smros/ddk.h`
- Linux-compatible PCI C headers: `include/linux/pci.h` and friends
- Stock Linux PCI driver: `src/user_level/drivers/linuxcompat/edu.c`
- C source form of the hello demo: `include/smros/examples/hello_linux.c`

This is still a user-level scaffold. MMIO is identity-mapped (with the RISC-V
virtio alias window), IRQ handlers are registered but not yet dispatched from
hardware, and DMA uses a 16 KiB coherent pool.

## Files

| Path | Role |
| --- | --- |
| `src/user_level/drivers/linux.rs` | Linux driver model and C ABI |
| `src/user_level/drivers/ddk.rs` | Public Rust DDK re-exports |
| `src/user_level/drivers/linux_logic*.rs` | Verified matching/bounds helpers |
| `src/user_level/drivers/demos.rs` | hello + dummy Linux demos |
| `src/user_level/drivers/block.rs` | `virtio_blk` Linux virtio driver |
| `src/user_level/drivers/net.rs` | `virtio_net` Linux virtio driver |
| `include/smros/ddk.h` | C DDK API |
| `include/linux/` | Linux-compatible headers so PCI driver C stays unchanged |
| `src/user_level/drivers/linuxcompat/edu.c` | QEMU EDU PCI driver (Linux C) |
| `src/user_level/drivers/linuxcompat/{compat,pci}.c` | printk/kmalloc/`pci_*` runtime |
| `include/smros/examples/hello_linux.c` | C hello demo |

## Boot / Bind Flow

```text
user_level::init()
  -> drivers::init()
       -> linux::init()
       -> install QEMU virt device tree, including smros,dummy
       -> register modules: hello, dummy, virtio_blk, virtio_net
       -> discover platform + virtio-mmio/pci devices
       -> linuxcompat::init()  // software EDU + module_pci_driver(edu)
       -> linux::attach()  // id_table / compatible match, then probe()
```

VirtIO-MMIO slots are peeked for a non-zero device id, published on the virtio
bus, then matched against `virtio_blk` (`device = 2`) and `virtio_net`
(`device = 1`). x86_64 publishes VirtIO-PCI transports onto the same virtio
bus.

## Rust DDK API

```rust
use crate::user_level::drivers::ddk::{
    register_module, register_platform_driver, register_virtio_driver,
    misc_register, ioremap, readl, writel, request_irq, dma_alloc_coherent,
    LinuxModule, PlatformDriver, VirtioDriver, MiscDevice, FileOperations,
    VIRTIO_ID_BLOCK, MISC_DYNAMIC_MINOR, IRQF_SHARED,
};

const IDS: [VirtioDeviceId; 1] = [VirtioDeviceId { device: VIRTIO_ID_BLOCK }];
const DRIVER: VirtioDriver = VirtioDriver {
    name: "virtio_blk",
    id_table: &IDS,
    probe: virtio_blk_probe,
    remove: None,
};

fn init() -> Result<(), UserDriverError> {
    register_virtio_driver(&DRIVER)
}

const MODULE: LinuxModule = LinuxModule {
    name: "virtio_blk",
    license: "GPL",
    author: "SMROS",
    description: "Linux virtio-blk driver demo",
    init,
    exit: None,
};
```

Platform drivers match `compatible` strings. An empty compatible entry is a
wildcard. VirtIO and PCI tables treat `VIRTIO_ID_ANY` / `PCI_ANY_ID` as
`PCI_ANY_ID`-style wildcards. A `{0,0}` PCI id-table terminator is ignored.

### Character devices

```rust
const FOPS: FileOperations = FileOperations {
    open: Some(hello_open),
    read: Some(hello_read),
    write: Some(hello_write),
    ioctl: Some(hello_ioctl),
    release: Some(hello_release),
};

misc_register(MiscDevice {
    name: "hello",
    minor: MISC_DYNAMIC_MINOR,
    fops: FOPS,
})?;
```

`MISC_DYNAMIC_MINOR` (255) allocates the next unused minor. Reads and writes
use a Linux-style file position (`loff_t`).

### MMIO, IRQ, DMA

```rust
let io = ioremap(phys, size)?;
let magic = readl(io);
writel(1, io + 4);
request_irq(32, handler, IRQF_SHARED, "dummy", io)?;
let (cpu, dma) = dma_alloc_coherent(64)?;
```

`ioremap` returns a supervisor-accessible alias of the physical address.
`dma_alloc_coherent` bump-allocates from a 16 KiB aligned pool; freeing only
reclaims the most recent allocation.

## C DDK API

```c
#include <smros/ddk.h>

unsigned int smros_ddk_api_version(void);
void *smros_ddk_ioremap(uint64_t phys, unsigned long size);
uint32_t smros_ddk_readl(const volatile void *addr);
void smros_ddk_writel(uint32_t value, volatile void *addr);
int smros_ddk_request_irq(unsigned int irq, smros_ddk_irq_handler_t handler,
                          unsigned long flags, const char *name, void *dev);
void *smros_ddk_dma_alloc_coherent(unsigned long size, uint64_t *dma_handle);
long smros_ddk_chrdev_read(const char *name, char *buf, unsigned long count,
                           long long *ppos);
```

Error returns follow negated Linux errno values (`-ENOENT`, `-EINVAL`,
`-ENOMEM`, `-EBUSY`, `-EIO`, `-ETIMEDOUT`).

## Shell

```text
drivers          # device tree, bindings, and Linux DDK registry
ddk              # DDK summary
ddk hello        # read the hello misc demo
ddk edu          # QEMU EDU PCI ident
testsc           # includes Linux DDK hello/dummy/edu smoke
```

## Writing a New Driver

1. Add a `LinuxModule` whose `init` registers a bus driver or misc device.
2. Put hardware IDs in an `id_table` / `compatible` list.
3. Implement `probe` with DDK helpers (`ioremap`, `request_irq`, queues).
4. Call `register_module` from `drivers::init` (or from another module init).
5. Keep block/net consumer APIs (`block_read_at`, `net_send_frame`) stable.

The hello and dummy demos in `src/user_level/drivers/demos.rs` are the
smallest templates. `virtio_blk` / `virtio_net` show a real Linux virtio
driver moved onto the framework. For PCI, keep the driver file as stock
Linux C under `src/user_level/drivers/linuxcompat/` and include only
`<linux/pci.h>` / `<linux/module.h>`.

## Linux PCI C drivers

A Linux PCI driver can be compiled against SMROS with about 90% of the C
unchanged. The driver file includes only Linux headers:

```c
#include <linux/module.h>
#include <linux/pci.h>
#include <linux/io.h>
#include <linux/slab.h>

static const struct pci_device_id edu_ids[] = {
    { PCI_DEVICE(0x1234, 0x11e8) },
    { 0, }
};

static struct pci_driver edu_driver = {
    .name = "edu",
    .id_table = edu_ids,
    .probe = edu_probe,
    .remove = edu_remove,
};

module_pci_driver(edu_driver);
```

`edu_probe` is ordinary Linux C: `pci_enable_device`, `pci_request_regions`,
`pci_set_master`, `kzalloc`, `pci_iomap`, `ioread32` of ident `0x010000ed`,
`pci_set_drvdata`. There is no `#include <smros/ddk.h>` in the driver.

The ~10% that is SMROS-specific is outside the driver:

- `include/linux/*.h` source-level Linux API
- `linuxcompat/compat.c` (`printk`, `kmalloc`) and `pci.c` (`pci_*` wrappers)
- `build.rs` compiling those C files into the kernel for AArch64 and x86_64

A software EDU BAR is published on every architecture so probe does not need
QEMU `-device edu`. RISC-V currently has no `riscv64-linux-gnu-gcc` in the
build image, so it binds the same EDU device through a Rust PCI fallback.

```text
ddk              # DDK summary, including edu
ddk hello        # read the hello misc demo
ddk edu          # QEMU EDU PCI ident
```
