//! LoongArch64 UART console driver (16550 compatible)

#![allow(dead_code)]

use core::fmt::Write;
use spin::Mutex;
use bitflags::bitflags;
use crate::arch::loongarch64::consts::VIRT_ADDR_START;

/// UART base address for QEMU virt machine
/// Uses DMW1 mapping (0x9000_0000_xxxx_xxxx)
const UART_ADDR: usize = 0x1fe001e0 | VIRT_ADDR_START;

/// Global UART instance
static COM1: Mutex<Uart> = Mutex::new(Uart::new(UART_ADDR));

bitflags! {
    /// Interrupt Enable Register flags
    pub struct IER: u8 {
        const RX_AVAILABLE = 1 << 0;
        const TX_EMPTY = 1 << 1;
    }

    /// Line Status Register flags
    pub struct LSR: u8 {
        const DATA_AVAILABLE = 1 << 0;
        const THR_EMPTY = 1 << 5;
    }

    /// Modem Control Register flags
    pub struct MCR: u8 {
        const DATA_TERMINAL_READY = 1 << 0;
        const REQUEST_TO_SEND = 1 << 1;
        const AUX_OUTPUT1 = 1 << 2;
        const AUX_OUTPUT2 = 1 << 3;
    }
}

/// UART registers (read mode)
#[repr(C)]
#[allow(dead_code)]
struct ReadRegisters {
    rbr: u8,      // Receiver Buffer Register
    ier: IER,     // Interrupt Enable Register
    iir: u8,      // Interrupt Identification Register
    lcr: u8,      // Line Control Register
    mcr: MCR,     // Modem Control Register
    lsr: LSR,     // Line Status Register
    _msr: u8,     // Modem Status Register (unused)
    _scr: u8,     // Scratch Register (unused)
}

/// UART registers (write mode)
#[repr(C)]
#[allow(dead_code)]
struct WriteRegisters {
    thr: u8,      // Transmitter Holding Register
    ier: IER,     // Interrupt Enable Register
    _fcr: u8,     // FIFO Control Register (unused)
    lcr: u8,      // Line Control Register
    mcr: MCR,     // Modem Control Register
    lsr: LSR,     // Line Status Register (read-only)
    _padding: u16, // Unused registers
}

/// UART device structure
pub struct Uart {
    base_address: usize,
}

impl Uart {
    /// Create a new UART instance
    pub const fn new(base_address: usize) -> Self {
        Uart { base_address }
    }

    /// Get read register access
    fn read_end(&mut self) -> &mut ReadRegisters {
        unsafe { &mut *(self.base_address as *mut ReadRegisters) }
    }

    /// Get write register access
    fn write_end(&mut self) -> &mut WriteRegisters {
        unsafe { &mut *(self.base_address as *mut WriteRegisters) }
    }

    /// Initialize UART
    pub fn init(&mut self) {
        let read_end = self.read_end();

        // Configure modem control register
        let mcr = MCR::DATA_TERMINAL_READY
            | MCR::REQUEST_TO_SEND
            | MCR::AUX_OUTPUT2;
        unsafe {
            core::ptr::write_volatile(&mut read_end.mcr as *mut MCR, mcr);
        }

        // Enable RX available interrupt
        let ier = IER::RX_AVAILABLE;
        unsafe {
            core::ptr::write_volatile(&mut read_end.ier as *mut IER, ier);
        }
    }

    /// Write a byte to UART
    pub fn putchar(&mut self, c: u8) {
        let write_end = self.write_end();

        // Wait until THR is empty
        loop {
            let lsr = unsafe { core::ptr::read_volatile(&write_end.lsr as *const LSR) };
            if lsr.contains(LSR::THR_EMPTY) {
                unsafe {
                    core::ptr::write_volatile(&mut write_end.thr as *mut u8, c);
                }
                break;
            }
        }
    }

    /// Read a byte from UART (non-blocking)
    pub fn getchar(&mut self) -> Option<u8> {
        let read_end = self.read_end();

        let lsr = unsafe { core::ptr::read_volatile(&read_end.lsr as *const LSR) };
        if lsr.contains(LSR::DATA_AVAILABLE) {
            Some(unsafe { core::ptr::read_volatile(&read_end.rbr as *const u8) })
        } else {
            None
        }
    }
}

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for c in s.bytes() {
            self.putchar(c);
        }
        Ok(())
    }
}

/// Public API: Write a byte to console
pub fn console_putchar(c: u8) {
    COM1.lock().putchar(c);
}

/// Public API: Read a byte from console (non-blocking)
/// Returns the character as usize, or 0 if no data available (matches SBI interface)
pub fn console_getchar() -> usize {
    COM1.lock().getchar().map(|c| c as usize).unwrap_or(0)
}

/// Public API: Initialize console
pub fn console_init() {
    COM1.lock().init();
}

/// Console writer for println! macro
pub struct ConsoleWriter;

impl Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        COM1.lock().write_str(s)
    }
}

/// Print function used by logging
pub fn print(args: core::fmt::Arguments) {
    ConsoleWriter.write_fmt(args).unwrap();
}
