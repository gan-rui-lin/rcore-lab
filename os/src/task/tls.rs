//! Thread Local Storage (TLS) support for RISC-V
//!
//! RISC-V uses TLS Variant I layout:
//! ```
//! High address
//! +------------------+
//! |  TCB (Thread     |
//! |  Control Block)  | <- tp (x4) points here
//! +------------------+
//! |  .tdata (init)   |
//! |  .tbss (zero)    |
//! +------------------+
//! Low address
//! ```

use crate::config::PAGE_SIZE;
use crate::mm::{MapPermission, MemorySet};

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
    /// Base address of TLS region
    pub tls_base: usize,
    /// Size of entire TLS region (including TCB)
    pub tls_size: usize,
}

impl TlsArea {
    /// Create a new TLS area from TLS info
    ///
    /// For RISC-V Variant I:
    /// - Allocate memory for TLS data + TCB
    /// - Copy initialized data from ELF
    /// - Zero out .tbss section
    /// - Set tp to point to TCB
    pub fn new(
        tls_info: &TlsInfo,
        memory_set: &mut MemorySet,
        elf_data: &[u8],
    ) -> Self {
        use crate::mm::translated_refmut;
        // TCB size: minimum of 2 pointers (dtv pointer + self pointer)
        let tcb_size = 2 * core::mem::size_of::<usize>();

        // Calculate total size: TLS data + TCB, aligned
        let total_size = Self::align_up(tls_info.memsz + tcb_size, tls_info.align);

        // Calculate number of pages needed
        let num_pages = (total_size + PAGE_SIZE - 1) / PAGE_SIZE;
        let total_size = num_pages * PAGE_SIZE;

        // Choose a suitable virtual address for TLS
        // Place it after user stack (around 0x70000000)
        let tls_base = 0x7000_0000;

        // Insert TLS area into memory set
        memory_set.insert_framed_area(
            tls_base.into(),
            (tls_base + total_size).into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );

        let token = memory_set.token();

        // Copy initialized data from ELF to TLS region
        if tls_info.filesz > 0 {
            let start = tls_info.file_offset;
            let end = start.saturating_add(tls_info.filesz).min(elf_data.len());
            let tls_data = &elf_data[start..end];

            // Copy data byte by byte
            for i in 0..tls_data.len() {
                let va = (tls_base + i) as *mut u8;
                *translated_refmut(token, va) = tls_data[i];
            }
        }

        // Zero out .tbss section (memsz - filesz bytes)
        for i in tls_info.filesz..tls_info.memsz {
            let va = (tls_base + i) as *mut u8;
            *translated_refmut(token, va) = 0;
        }

        // tp points to the start of TCB (after TLS data)
        let tp_value = tls_base + tls_info.memsz;

        // Initialize TCB
        // TCB[0] = dtv pointer (set to 0 for now)
        *translated_refmut(token, tp_value as *mut usize) = 0;
        // TCB[1] = self pointer (points to TCB itself)
        *translated_refmut(token, (tp_value + 8) as *mut usize) = tp_value;

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
