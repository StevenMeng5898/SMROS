//! Rust coroutine + Linux epoll demo for SMROS.
//!
//! Two async tasks share one epoll instance:
//!   writer coroutine writes a pipe
//!   reader coroutine awaits EPOLLIN, then reads
//!
//! Build into /shared (rebuild the kernel after this):
//!   make -C src/user_level/apps smros
//!
//! On SMROS:
//!   run epoll_coro.elf

#![no_std]
#![allow(static_mut_refs)]

use core::future::Future;
use core::pin::Pin;
use core::ptr;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

const EPOLLIN: u32 = 0x0001;
const EPOLL_CTL_ADD: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct EpollEvent {
    events: u32,
    _pad: u32,
    data: u64,
}

extern "C" {
    fn write(fd: i32, buf: *const u8, count: usize) -> isize;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn epoll_create1(flags: i32) -> i32;
    fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: *mut EpollEvent) -> i32;
    fn epoll_wait(epfd: i32, events: *mut EpollEvent, maxevents: i32, timeout: i32) -> i32;
    fn exit(status: i32) -> !;
}

static mut READY_MASK: u64 = 0;

fn mark_ready(fd: i32) {
    if fd >= 0 && fd < 64 {
        unsafe {
            READY_MASK |= 1u64 << fd;
        }
    }
}

fn take_ready(fd: i32) -> bool {
    if fd < 0 || fd >= 64 {
        return false;
    }
    unsafe {
        let bit = 1u64 << fd;
        let ready = READY_MASK & bit != 0;
        READY_MASK &= !bit;
        ready
    }
}

fn put(msg: &str) {
    unsafe {
        let _ = write(1, msg.as_ptr(), msg.len());
    }
}

fn put_int(prefix: &str, value: i32, suffix: &str) {
    put(prefix);
    let mut buf = [0u8; 16];
    let mut n = value;
    let negative = n < 0;
    if negative {
        n = -n;
    }
    if n == 0 {
        put("0");
        put(suffix);
        return;
    }
    let mut i = buf.len();
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if negative && i > 0 {
        i -= 1;
        buf[i] = b'-';
    }
    unsafe {
        let _ = write(1, buf.as_ptr().add(i), buf.len() - i);
    }
    put(suffix);
}

fn dummy_raw_waker() -> RawWaker {
    fn clone(_: *const ()) -> RawWaker {
        dummy_raw_waker()
    }
    fn wake(_: *const ()) {}
    fn wake_by_ref(_: *const ()) {}
    fn drop(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
    RawWaker::new(ptr::null(), &VTABLE)
}

fn dummy_waker() -> Waker {
    unsafe { Waker::from_raw(dummy_raw_waker()) }
}

struct WaitReadable {
    fd: i32,
}

impl Future for WaitReadable {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if take_ready(self.fd) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

async fn pipe_writer(write_fd: i32) {
    put("[coro] writer: sending hello-from-rust-coro\n");
    let msg = b"hello-from-rust-coro\n";
    let wrote = unsafe { write(write_fd, msg.as_ptr(), msg.len()) };
    if wrote != msg.len() as isize {
        put("[coro] writer: write failed\n");
        unsafe { exit(2) }
    }
}

async fn pipe_reader(read_fd: i32) {
    put("[coro] reader: awaiting EPOLLIN\n");
    WaitReadable { fd: read_fd }.await;
    let mut buf = [0u8; 64];
    let n = unsafe { read(read_fd, buf.as_mut_ptr(), buf.len()) };
    if n <= 0 {
        put("[coro] reader: read failed\n");
        unsafe { exit(3) }
    }
    put("[coro] reader: got ");
    unsafe {
        let _ = write(1, buf.as_ptr(), n as usize);
    }
}

fn epoll_add(epfd: i32, fd: i32) -> bool {
    let mut event = EpollEvent {
        events: EPOLLIN,
        _pad: 0,
        data: fd as u64,
    };
    unsafe { epoll_ctl(epfd, EPOLL_CTL_ADD, fd, &mut event) == 0 }
}

fn run_pair(epfd: i32, read_fd: i32, write_fd: i32) -> bool {
    let mut reader = core::pin::pin!(pipe_reader(read_fd));
    let mut writer = core::pin::pin!(pipe_writer(write_fd));
    let waker = dummy_waker();
    let mut cx = Context::from_waker(&waker);
    let mut reader_done = false;
    let mut writer_done = false;
    let mut spins = 0u32;

    while !(reader_done && writer_done) {
        if !writer_done && writer.as_mut().poll(&mut cx).is_ready() {
            writer_done = true;
            put("[coro] writer: complete\n");
        }
        if !reader_done && reader.as_mut().poll(&mut cx).is_ready() {
            reader_done = true;
            put("[coro] reader: complete\n");
        }
        if reader_done && writer_done {
            break;
        }

        let mut events = [EpollEvent {
            events: 0,
            _pad: 0,
            data: 0,
        }; 4];
        let n = unsafe { epoll_wait(epfd, events.as_mut_ptr(), 4, 0) };
        put_int("[coro] epoll_wait -> ", n, " event(s)\n");
        if n < 0 {
            put("[coro] epoll_wait failed\n");
            return false;
        }
        for event in events.iter().take(n as usize) {
            put_int("[coro]   ready fd=", event.data as i32, " EPOLLIN\n");
            mark_ready(event.data as i32);
        }

        spins += 1;
        if spins > 8 {
            put("[coro] executor gave up waiting for EPOLLIN\n");
            return false;
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    put("SMROS Rust coroutine epoll demo\n");

    let epfd = unsafe { epoll_create1(0) };
    if epfd < 0 {
        put("[coro] epoll_create1 failed\n");
        return 1;
    }
    put_int("[coro] epoll_create1 -> fd=", epfd, "\n");

    let mut fds = [0i32; 2];
    if unsafe { pipe2(fds.as_mut_ptr(), 0) } != 0 {
        put("[coro] pipe2 failed\n");
        return 1;
    }
    put_int("[coro] pipe read fd=", fds[0], "");
    put_int(" write fd=", fds[1], "\n");

    if !epoll_add(epfd, fds[0]) {
        put("[coro] epoll_ctl ADD failed\n");
        return 1;
    }
    put("[coro] epoll_ctl ADD EPOLLIN on pipe read fd\n");

    let ok = run_pair(epfd, fds[0], fds[1]);
    unsafe {
        let _ = close(fds[0]);
        let _ = close(fds[1]);
        let _ = close(epfd);
    }
    if ok {
        put("PASS\n");
        0
    } else {
        put("FAIL\n");
        1
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    put("panic\n");
    unsafe { exit(111) }
}
