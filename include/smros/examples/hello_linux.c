/*
 * Linux misc-character driver demo for the SMROS user-space DDK.
 *
 * The live hello driver is the Rust port in
 * `src/user_level/drivers/demos.rs`. This C file is the Linux-shaped source
 * form of the same demo: a GPL module that registers a misc device named
 * "hello" and implements read/write/ioctl.
 *
 * Build against the SMROS DDK instead of the Linux kernel:
 *   #include <smros/ddk.h>
 *
 * On Linux this would be:
 *   #include <linux/module.h>
 *   #include <linux/miscdevice.h>
 *   #include <linux/fs.h>
 */

#include <smros/ddk.h>
#include <string.h>

#define HELLO_DEFAULT "Hello from SMROS Linux DDK\n"

static char hello_buf[128];
static unsigned long hello_len;

static int hello_init(void)
{
    hello_len = strlen(HELLO_DEFAULT);
    memcpy(hello_buf, HELLO_DEFAULT, hello_len);
    /*
     * Linux equivalent:
     *   misc_register(&hello_misc);
     * SMROS hosts the live misc device from the Rust hello module; this C
     * demo reads it through the DDK character-device ABI.
     */
    return SMROS_DDK_SUCCESS;
}

static long hello_read_demo(char *buf, unsigned long count, long long *ppos)
{
    return smros_ddk_chrdev_read("hello", buf, count, ppos);
}

static long hello_write_demo(const char *buf, unsigned long count, long long *ppos)
{
    return smros_ddk_chrdev_write("hello", buf, count, ppos);
}

/*
 * Linux:
 *   module_init(hello_init);
 *   MODULE_LICENSE("GPL");
 *   MODULE_DESCRIPTION("Linux misc character driver demo");
 */
int smros_linux_hello_demo_init(void)
{
    return hello_init();
}

long smros_linux_hello_demo_read(char *buf, unsigned long count)
{
    long long pos = 0;
    return hello_read_demo(buf, count, &pos);
}

long smros_linux_hello_demo_write(const char *buf, unsigned long count)
{
    long long pos = 0;
    return hello_write_demo(buf, count, &pos);
}
