//! PL011 UART Serial Driver for QEMU ARM64
//! 
//! This module provides basic serial output functionality
//! for the ARM PrimeCell UART (PL011) used by QEMU.

use core::ptr::{read_volatile, write_volatile};

/// PL011 UART Base Address for QEMU virt machine
const UART_BASE: usize = 0x9000000;

/// UART Register offsets
const UART_DR: usize   = 0x00; // Data Register
const UART_FR: usize   = 0x18; // Flag Register
const UART_IBRD: usize = 0x24; // Integer Baud Rate Divisor
const UART_FBRD: usize = 0x28; // Fractional Baud Rate Divisor
const UART_LCRH: usize = 0x2C; // Line Control Register
const UART_CR: usize   = 0x30; // Control Register
const UART_IFLS: usize = 0x34; // Interrupt FIFO Level Select
const UART_IMSC: usize = 0x38; // Interrupt Mask Set/Clear
const UART_RIS: usize  = 0x3C; // Raw Interrupt Status
const UART_MIS: usize  = 0x40; // Masked Interrupt Status
const UART_ICR: usize  = 0x44; // Interrupt Clear Register

/// Flag Register bits
const FR_TXFF: u32 = 1 << 5; // Transmit FIFO Full
const FR_RXFE: u32 = 1 << 4; // Receive FIFO Empty
const FR_BUSY: u32 = 1 << 3; // UART Busy

/// Line Control Register bits
const LCRH_WLEN_8: u32 = 3 << 5; // 8-bit word length
const LCRH_FEN: u32 = 1 << 4;    // Enable FIFOs

/// Control Register bits
const CR_UARTEN: u32 = 1 << 0;   // UART Enable
const CR_TXE: u32 = 1 << 8;      // Transmit Enable
const CR_RXE: u32 = 1 << 9;      // Receive Enable

/// Serial port structure
pub struct Serial {
    base: usize,
}

impl Serial {
    /// Create a new Serial instance
    /// 
    /// # Safety
    /// This function accesses hardware registers directly
    pub const unsafe fn new() -> Self {
        Serial { base: UART_BASE }
    }
    
    /// Initialize the UART
    pub fn init(&mut self) {
        // Disable UART during configuration
        self.write_reg(UART_CR, 0);
        
        // Set baud rate to 115200 (assuming 24MHz clock)
        // baud_rate = clock / (16 * (IBRD + FBRD/64))
        // IBRD = 24000000 / (16 * 115200) = 13
        // FBRD = ((24000000 % (16 * 115200)) * 64 + 8) / (16 * 115200) = 2
        self.write_reg(UART_IBRD, 13);
        self.write_reg(UART_FBRD, 2);
        
        // Set line control: 8-bit word, FIFO enabled
        self.write_reg(UART_LCRH, LCRH_WLEN_8 | LCRH_FEN);
        
        // Clear any pending interrupts
        self.write_reg(UART_ICR, 0x7FF);
        
        // Enable UART, TX, and RX
        self.write_reg(UART_CR, CR_UARTEN | CR_TXE | CR_RXE);
    }
    
    /// Write a byte to the serial port
    pub fn write_byte(&mut self, byte: u8) {
        // Wait until TX FIFO is not full
        while (self.read_reg(UART_FR) & FR_TXFF) != 0 {
            core::hint::spin_loop();
        }
        
        // Write the byte
        self.write_reg(UART_DR, byte as u32);
    }
    
    /// Write a string to the serial port
    pub fn write_str(&mut self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
    }
    
    /// Write a hex number to the serial port
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
        
        // Skip leading zeros
        let start = 16 - i;
        for j in start..16 {
            self.write_byte(buf[j]);
        }
    }
    
    /// Write a buffer to the serial port
    pub fn write_buf(&mut self, buf: &[u8]) {
        for &byte in buf {
            self.write_byte(byte);
        }
    }
    
    /// Read a byte from the serial port (non-blocking)
    pub fn read_byte(&mut self) -> Option<u8> {
        // Check if RX FIFO is empty
        if (self.read_reg(UART_FR) & FR_RXFE) != 0 {
            return None;
        }
        
        Some((self.read_reg(UART_DR) & 0xFF) as u8)
    }
    
    /// Read a register
    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }
    
    /// Write a register
    fn write_reg(&self, offset: usize, value: u32) {
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }
}
