//! Auxiliary Vector (auxv) support for ELF program initialization
//!
//! The auxiliary vector provides information from the kernel to the dynamic linker
//! and C library initialization code. It's placed on the initial stack after envp.

use alloc::vec::Vec;

/// Auxiliary vector entry type constants (AT_* from Linux)
pub mod auxv_type {
    pub const AT_NULL: usize = 0;       // End of vector
    #[allow(dead_code)]
    pub const AT_IGNORE: usize = 1;     // Entry should be ignored
    #[allow(dead_code)]
    pub const AT_EXECFD: usize = 2;     // File descriptor of program
    pub const AT_PHDR: usize = 3;       // Program headers for program
    pub const AT_PHENT: usize = 4;      // Size of program header entry
    pub const AT_PHNUM: usize = 5;      // Number of program headers
    pub const AT_PAGESZ: usize = 6;     // System page size
    #[allow(dead_code)]
    pub const AT_BASE: usize = 7;       // Base address of interpreter
    #[allow(dead_code)]
    pub const AT_FLAGS: usize = 8;      // Flags
    pub const AT_ENTRY: usize = 9;      // Entry point of program
    #[allow(dead_code)]
    pub const AT_NOTELF: usize = 10;    // Program is not ELF
    pub const AT_UID: usize = 11;       // Real uid
    pub const AT_EUID: usize = 12;      // Effective uid
    pub const AT_GID: usize = 13;       // Real gid
    pub const AT_EGID: usize = 14;      // Effective gid
    #[allow(dead_code)]
    pub const AT_PLATFORM: usize = 15;  // String identifying platform
    #[allow(dead_code)]
    pub const AT_HWCAP: usize = 16;     // Machine dependent hints about processor capabilities
    #[allow(dead_code)]
    pub const AT_CLKTCK: usize = 17;    // Frequency of times()
    pub const AT_SECURE: usize = 23;    // Secure mode boolean
    pub const AT_RANDOM: usize = 25;    // Address of 16 random bytes
}

/// Information needed for auxiliary vectors
#[derive(Debug, Clone, Copy)]
pub struct AuxvInfo {
    /// Address where program headers are loaded
    pub phdr_addr: usize,
    /// Size of one program header entry
    pub phent_size: usize,
    /// Number of program headers
    pub phnum: usize,
    /// Entry point address
    pub entry: usize,
}

impl AuxvInfo {
    /// Create auxiliary vector entries for the stack
    /// Returns a vector of (type, value) pairs
    pub fn to_entries(&self, page_size: usize) -> Vec<(usize, usize)> {
        use auxv_type::*;
        alloc::vec![
            (AT_PHDR, self.phdr_addr),
            (AT_PHENT, self.phent_size),
            (AT_PHNUM, self.phnum),
            (AT_PAGESZ, page_size),
            (AT_ENTRY, self.entry),
            (AT_UID, 0),        // Root user
            (AT_EUID, 0),       // Root user
            (AT_GID, 0),        // Root group
            (AT_EGID, 0),       // Root group
            (AT_SECURE, 0),     // Not secure mode
            (AT_RANDOM, 0),     // TODO: Implement random bytes
            (AT_NULL, 0),       // Terminator
        ]
    }
}
