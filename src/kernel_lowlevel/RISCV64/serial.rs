#![allow(dead_code)]
//! NS16550-compatible UART driver for RISC-V64 FDT-discovered consoles.

use core::ptr::{read_volatile, write_volatile};

use super::{drivers, lowlevel_logic};

const UART_RBR_THR_DLL: usize = 0x00;
const UART_IER_DLM: usize = 0x01;
const UART_FCR_IIR: usize = 0x02;
const UART_LCR: usize = 0x03;
const UART_LSR: usize = 0x05;

const LCR_DLAB: u8 = 1 << 7;
const LCR_8N1: u8 = 0x03;
const FCR_ENABLE_CLEAR: u8 = 0x07;
const LSR_DR: u8 = 1 << 0;
const LSR_THRE: u8 = 1 << 5;

pub struct Serial {
    base: usize,
}

impl Serial {
    pub const fn new() -> Self {
        Self { base: 0 }
    }

    pub fn active() -> Self {
        Self {
            base: drivers::uart_base(),
        }
    }

    pub fn init(&mut self) {
        self.base = drivers::uart_base();
        if self.base == 0 {
            return;
        }

        self.write_reg(UART_IER_DLM, 0);
        self.write_reg(UART_LCR, LCR_DLAB);
        self.write_reg(UART_RBR_THR_DLL, 3);
        self.write_reg(UART_IER_DLM, 0);
        self.write_reg(UART_LCR, LCR_8N1);
        self.write_reg(UART_FCR_IIR, FCR_ENABLE_CLEAR);
    }

    pub fn write_byte(&mut self, byte: u8) {
        if self.base == 0 {
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
        let hex_chars = b"0123456789abcdef";
        let mut buf = [0u8; 16];
        let mut i = 0;
        if value == 0 {
            self.write_byte(b'0');
            return;
        }
        while value > 0 && i < 16 {
            buf[15 - i] = hex_chars[(value & 0xF) as usize];
            value >>= 4;
            i += 1;
        }
        for j in (16 - i)..16 {
            self.write_byte(buf[j]);
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
        self.base != 0 && self.read_reg(UART_LSR) & LSR_DR != 0
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
                0x0C => self.write_str("\x1B[2J\x1B[H"),
                _ => {
                    if pos < buf.len() - 1 && lowlevel_logic::ascii_printable(ch) {
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

    fn read_reg(&self, offset: usize) -> u8 {
        let addr = self.checked_mmio_addr(offset);
        unsafe { read_volatile(addr as *const u8) }
    }

    fn write_reg(&self, offset: usize, value: u8) {
        let addr = self.checked_mmio_addr(offset);
        unsafe { write_volatile(addr as *mut u8, value) }
    }

    fn checked_mmio_addr(&self, offset: usize) -> usize {
        let size = drivers::uart_size();
        match lowlevel_logic::mmio_addr(self.base, offset) {
            Some(addr) if size == 0 || lowlevel_logic::dt_reg_contains(self.base, size, addr) => {
                addr
            }
            _ => self.base,
        }
    }
}
