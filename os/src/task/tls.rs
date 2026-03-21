//! Thread Local Storage (TLS) support for RISC-V
//!
//! RISC-V uses musl's TLS_ABOVE_TP layout:
//!
//! ```text
//! Low address
//! +---------------------------+
//! | (space for pthread struct)|  <- musl: self = tp - sizeof(__pthread) - GAP
//! | (reserved, zeroed)        |
//! +---------------------------+
//! | GAP_ABOVE_TP (16 bytes)  |  (DTV pointer + padding)
//! +---------------------------+  <- tp (x4) points here
//! | .tdata (initialized)     |
//! | .tbss (zero-initialized) |
//! +---------------------------+
//! High address
//! ```
//!
//! Key: musl passes its own tp via CLONE_SETTLS for threads.
//! The kernel only needs to set up initial TLS for the main thread during exec.
//! musl's __init_tls will override it, but tp must be valid enough for early boot.

use crate::config::PAGE_SIZE;
use crate::mm::{MapPermission, MemorySet};

/// musl's GAP_ABOVE_TP for RISC-V (DTV pointer + padding)
const GAP_ABOVE_TP: usize = 16;

/// Reserved space below tp for musl's pthread struct.
/// musl's struct __pthread is ~200-300 bytes depending on version.
/// We reserve a generous 1024 bytes to be safe.
const PTHREAD_STRUCT_RESERVE: usize = 1024;

/// TLS information from ELF PT_TLS segment
#[derive(Debug, Clone, Copy)]
pub struct TlsInfo {
    /// Virtual address of TLS template in ELF
    pub vaddr: usize,
    /// File offset of TLS template in ELF
    pub file_offset: usize,
    /// Size of initialized data (.tdata) in file
    pub filesz: usize,
    /// Total size in memory (.tdata + .tbss)
    pub memsz: usize,
    /// Alignment requirement
    pub align: usize,
}

/// Thread Local Storage area for a process/thread
#[derive(Debug, Clone)]
pub struct TlsArea {
    /// Value to put in tp register (x4)
    pub tp_value: usize,
    /// Base address of TLS region (start of allocation)
    pub tls_base: usize,
    /// Size of entire TLS region
    pub tls_size: usize,
}

impl TlsArea {
    /// Create a new TLS area from TLS info
    ///
    /// Layout (RISC-V musl TLS_ABOVE_TP):
    ///   [pthread_reserve] [GAP_ABOVE_TP=16] [.tdata] [.tbss]
    ///                                        ^-- tp points here
    pub fn new(tls_info: &TlsInfo, memory_set: &mut MemorySet, elf_data: &[u8]) -> Self {
        use crate::mm::translated_refmut;

        let align = if tls_info.align > 0 {
            tls_info.align
        } else {
            8
        };

        // Place TLS at a fixed virtual address
        let tls_base = 0x7000_0000;

        // tp = tls_base + pthread_reserve + GAP_ABOVE_TP
        // Align tp to the TLS alignment requirement
        let tp_value = Self::align_up(tls_base + PTHREAD_STRUCT_RESERVE + GAP_ABOVE_TP, align);

        // TLS data starts at tp (TLS_ABOVE_TP: data is above tp)
        let tls_data_start = tp_value;
        let tls_end = tls_data_start.saturating_add(tls_info.memsz);
        let total_size = tls_end.saturating_sub(tls_base);
        let num_pages = (total_size + PAGE_SIZE - 1) / PAGE_SIZE;
        let total_size = num_pages * PAGE_SIZE;

        // Insert TLS area into memory set
        memory_set.insert_framed_area(
            tls_base.into(),
            (tls_base + total_size).into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );

        let token = memory_set.token();

        // Copy initialized data (.tdata) from ELF
        if tls_info.filesz > 0 {
            let start = tls_info.file_offset;
            let end = start.saturating_add(tls_info.filesz).min(elf_data.len());
            let tls_data = &elf_data[start..end];

            for i in 0..tls_data.len() {
                let va = (tls_data_start + i) as *mut u8;
                *translated_refmut(token, va) = tls_data[i];
            }
        }

        // Zero out .tbss section (memsz - filesz bytes after .tdata)
        for i in tls_info.filesz..tls_info.memsz {
            let va = (tls_data_start + i) as *mut u8;
            *translated_refmut(token, va) = 0;
        }

        // Zero the GAP area (DTV pointer area, 16 bytes before tp)
        for i in 0..GAP_ABOVE_TP {
            let va = (tp_value - GAP_ABOVE_TP + i) as *mut u8;
            *translated_refmut(token, va) = 0;
        }

        // Zero the pthread struct reserve area
        // musl's __init_tls will set this up properly, but zero it for safety
        for i in 0..core::cmp::min(PTHREAD_STRUCT_RESERVE, 64) {
            let va = (tls_base + i) as *mut u8;
            *translated_refmut(token, va) = 0;
        }

        Self {
            tp_value,
            tls_base,
            tls_size: total_size,
        }
    }

    /// Create a new TLS area by copying from parent (for fork)
    pub fn new_from_parent(
        parent: &TlsArea,
        parent_memory_set: &MemorySet,
        child_memory_set: &mut MemorySet,
    ) -> Self {
        use crate::mm::{translated_ref, translated_refmut};
        // Allocate same region in child
        child_memory_set.insert_framed_area(
            parent.tls_base.into(),
            (parent.tls_base + parent.tls_size).into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );

        // Copy TLS data from parent to child byte by byte
        let parent_token = parent_memory_set.token();
        let child_token = child_memory_set.token();

        for i in 0..parent.tls_size {
            let va = (parent.tls_base + i) as *const u8;
            let byte = *translated_ref(parent_token, va);
            let dest_va = (parent.tls_base + i) as *mut u8;
            *translated_refmut(child_token, dest_va) = byte;
        }

        Self {
            tp_value: parent.tp_value,
            tls_base: parent.tls_base,
            tls_size: parent.tls_size,
        }
    }

    /// Align value up to alignment
    fn align_up(value: usize, align: usize) -> usize {
        (value + align - 1) & !(align - 1)
    }
}
