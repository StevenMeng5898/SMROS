#![allow(dead_code)]

use super::drivers;

const UART_RBR_THR_DLL: u16 = 0x00;
const UART_IER_DLM: u16 = 0x01;
const UART_FCR_IIR: u16 = 0x02;
const UART_LCR: u16 = 0x03;
const UART_LSR: u16 = 0x05;
const LCR_DLAB: u8 = 1 << 7;
const LCR_8N1: u8 = 0x03;
const FCR_ENABLE_CLEAR: u8 = 0x07;
const LSR_DR: u8 = 1 << 0;
const LSR_THRE: u8 = 1 << 5;

pub struct Serial {
    port: u16,
}

impl Serial {
    pub const fn new() -> Self {
        Self { port: 0 }
    }

    pub fn active() -> Self {
        Self {
            port: drivers::uart_base() as u16,
        }
    }

    pub fn init(&mut self) {
        self.port = drivers::uart_base() as u16;
        if self.port == 0 {
            return;
        }
        self.write_reg(UART_IER_DLM, 0);
        self.write_reg(UART_LCR, LCR_DLAB);
        self.write_reg(UART_RBR_THR_DLL, 1);
        self.write_reg(UART_IER_DLM, 0);
        self.write_reg(UART_LCR, LCR_8N1);
        self.write_reg(UART_FCR_IIR, FCR_ENABLE_CLEAR);
    }

    pub fn write_byte(&mut self, byte: u8) {
        if self.port == 0 {
            return;
        }
        while self.read_reg(UART_LSR) & LSR_THRE == 0 {
            core::hint::spin_loop();
        }
        self.write_reg(UART_RBR_THR_DLL, byte);
    }

    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }

    pub fn write_hex(&mut self, mut value: u64) {
        let mut buf = [0u8; 16];
        let mut len = 0;
        if value == 0 {
            self.write_byte(b'0');
            return;
        }
        while value > 0 && len < buf.len() {
            let digit = (value & 0xf) as u8;
            buf[len] = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + digit - 10
            };
            value >>= 4;
            len += 1;
        }
        while len > 0 {
            len -= 1;
            self.write_byte(buf[len]);
        }
    }

    pub fn write_buf(&mut self, buf: &[u8]) {
        for &byte in buf {
            self.write_byte(byte);
        }
    }

    pub fn read_byte(&mut self) -> u8 {
        while !self.has_byte() {
            crate::kernel_lowlevel::cpu::wait_for_event();
        }
        self.read_reg(UART_RBR_THR_DLL)
    }

    pub fn has_byte(&mut self) -> bool {
        self.port != 0 && self.read_reg(UART_LSR) & LSR_DR != 0
    }

    pub fn read_line(&mut self, buf: &mut [u8]) -> usize {
        let mut pos = 0;
        loop {
            let ch = self.read_byte();
            match ch {
                b'\r' | b'\n' => {
                    self.write_str("\r\n");
                    break;
                }
                b'\x7f' | b'\x08' => {
                    if pos > 0 {
                        pos -= 1;
                        self.write_str("\x08 \x08");
                    }
                }
                0x03 => {
                    self.write_str("^C\r\n");
                    pos = 0;
                    break;
                }
                0x15 => {
                    while pos > 0 {
                        pos -= 1;
                        self.write_str("\x08 \x08");
                    }
                }
                0x0c => self.write_str("\x1B[2J\x1B[H"),
                _ => {
                    if pos < buf.len() - 1
                        && crate::kernel_lowlevel::lowlevel_logic::ascii_printable(ch)
                    {
                        buf[pos] = ch;
                        pos += 1;
                        self.write_byte(ch);
                    }
                }
            }
        }
        if pos < buf.len() {
            buf[pos] = 0;
        } else {
            buf[buf.len() - 1] = 0;
            pos = buf.len() - 1;
        }
        pos
    }

    fn read_reg(&self, offset: u16) -> u8 {
        unsafe { inb(self.port + offset) }
    }

    fn write_reg(&self, offset: u16, value: u8) {
        unsafe { outb(self.port + offset, value) }
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags),
    );
    value
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags),
    );
}
