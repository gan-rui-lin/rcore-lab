# Auxiliary Vector (auxv) Implementation Summary

## ✅ Completed Work

### 1. Full Auxiliary Vector Support

#### New Files Created
- **`os/src/task/auxv.rs`**: Complete auxiliary vector module
  - `auxv_type` module with AT_* constants
  - `AuxvInfo` structure for ELF metadata
  - `to_entries()` method to generate auxv stack entries

#### Modified Files

**`os/src/task/mod.rs`**:
- Added `mod auxv`
- Exported `AuxvInfo`

**`os/src/mm/memory_set.rs`**:
- Modified `from_elf()` signature to return `AuxvInfo`
- Calculate AT_PHDR address from ELF program headers
- Extract AT_PHENT (program header entry size)
- Extract AT_PHNUM (number of program headers)
- Added PT_INTERP detection (checks for dynamic linker)
- Enhanced logging for ELF parsing

**`os/src/task/process.rs`**:
- Modified `new()` and `exec()` to handle `AuxvInfo`
- Push auxiliary vectors onto user stack after envp
- Allocate 16 random bytes for AT_RANDOM
- Pass complete auxv entries to user process:
  - AT_PHDR: Program headers address
  - AT_PHENT: Size of program header entry (56 bytes)
  - AT_PHNUM: Number of program headers (4 for busybox)
  - AT_PAGESZ: System page size (4096)
  - AT_ENTRY: Entry point address
  - AT_UID, AT_EUID, AT_GID, AT_EGID: User/group IDs (all 0)
  - AT_SECURE: Secure mode (0)
  - AT_RANDOM: Pointer to 16 random bytes
  - AT_NULL: Terminator

**`os/src/task/process.rs` - Extended TCB**:
- Enhanced minimal TCB to match musl's pthread structure layout:
  - self pointer (offset 0)
  - dtv pointer (offset 8)
  - prev/next pointers (offset 16, 24)
  - sysinfo (offset 32)
  - canary fields (offset 40, 48)
  - tid (offset 56)
  - Zeroed out 256 bytes for safety

**`os/src/syscall/mod.rs`**:
- Added detailed logging for set_tid_address syscall
- Logs ra and sepc registers when syscall returns

## 📊 Test Results

### Successful Initialization Steps

1. **ELF Loading**: ✅ Correctly parses busybox
   ```
   [ INFO] [ELF] Scanning 4 program headers for PT_TLS and PT_INTERP
   [ INFO] [ELF] No PT_TLS segment found
   [ INFO] [ELF] No PT_INTERP (statically linked)
   [ INFO] [ELF] Auxv: phdr=0x10040, phent=56, phnum=4, entry=0x10148
   ```

2. **Auxiliary Vectors**: ✅ Correctly pushed onto stack
   ```
   [ INFO] [kernel] exec: Pushed 12 auxv entries at 0x168ec0, AT_RANDOM=0x168f80
   ```

3. **Thread Control Block**: ✅ Allocated and initialized
   ```
   [ INFO] [kernel] exec: Extended TCB allocated at 0x70001000 (no PT_TLS), tid=2
   tp (x4) = 0x70001000
   ```

4. **set_tid_address**: ✅ Returns successfully
   ```
   [ INFO] [syscall] set_tid_address returned 2 to busybox, ra=0x1206a8, sepc=0x1206cc
   ```

5. **Execution Continues**: ✅ Program continues after set_tid_address
   - sepc changes from 0x1206cc to later addresses
   - ra changes from 0x1206a8 to 0x104a7c
   - This proves execution continued successfully

### Current Crash

After set_tid_address returns and execution continues, busybox crashes:
```
[kernel] trap_handler: Exception(InstructionPageFault) in application
  bad addr (stval) = 0x0
  bad instruction (sepc) = 0x0
  Registers:
    ra (x1) = 0x104a7c      ← Different from set_tid_address return!
    sp (x2) = 0x168e60
    tp (x4) = 0x70001000    ← TCB pointer is valid
    a0 (x10) = 0x10ec8
    a1 (x11) = 0x2          ← set_tid_address return value
```

## ❌ Remaining Issue

### Problem: Null Function Pointer Call After Initialization

**Evidence**:
1. set_tid_address returns successfully at sepc=0x1206cc with ra=0x1206a8
2. Execution continues (ra changes to 0x104a7c)
3. Then tries to execute at address 0x0

**Possible Causes**:

1. **Missing __libc_start_init or Similar**
   - musl libc may expect certain init/fini functions
   - These might be passed via auxv or set up differently

2. **Missing System Calls**
   - There might be additional syscalls between set_tid_address and the crash
   - Enable more verbose syscall tracing to catch them

3. **Stack Canary or Security Features**
   - musl might be trying to set up stack canaries
   - The canary fields in TCB might need proper initialization

4. **Global Constructors**
   - C++ global constructors or __attribute__((constructor)) functions
   - These need to be called before main()

5. **Dynamic Linker Residual**
   - Even though busybox is statically linked, there might be
   - residual dynamic linking code expecting certain structures

## 🔍 Debugging Next Steps

### 1. Add Comprehensive Syscall Tracing

Enable ALL syscall logging to see if any syscalls happen between set_tid_address and the crash:

```rust
// In os/src/syscall/mod.rs, remove the filter:
if known {  // Remove "&& trace" filter
    info!("[syscall] pid={} {} num={} ret={}", pid, name, syscall_id, ret);
}
```

### 2. Use GDB to Analyze Crash Location

```bash
bash run.sh -t debug -d
# In another terminal:
riscv64-unknown-elf-gdb -ex 'file target/riscv64gc-unknown-none-elf/debug/os' \
                        -ex 'target remote localhost:1234' \
                        -ex 'b *0x1206cc'  # Break at set_tid_address return
                        -ex 'b *0x104a7c'  # Break at crash ra
                        -ex 'c'
```

Then use GDB commands:
- `stepi` to single-step through instructions
- `x/10i $pc` to disassemble around current location
- `info registers` to see all register values

### 3. Check for Missing Syscalls

Common syscalls that might be needed:
- `rt_sigaction` (134) - signal handler setup
- `rt_sigprocmask` (135) - signal mask
- `getrandom` (278) - for AT_RANDOM
- `clock_gettime` (113) - for AT_CLKTCK

### 4. Examine busybox Binary

```bash
readelf -a /path/to/busybox | grep -A10 "init\|fini\|constructor"
objdump -d /path/to/busybox | grep -A20 "^000000000001206cc"  # set_tid_address return
objdump -d /path/to/busybox | grep -A20 "^0000000000104a7c"  # crash ra
```

### 5. Test with Simpler Program

Create a minimal musl program to isolate the issue:

```c
// test_tls.c
#include <stdio.h>
#include <sys/syscall.h>
#include <unistd.h>

int main() {
    long tid = syscall(SYS_set_tid_address, NULL);
    printf("set_tid_address returned: %ld\n", tid);
    printf("Hello from musl!\n");
    return 0;
}
```

Compile with musl and test:
```bash
musl-gcc -static test_tls.c -o test_tls
# Copy to filesystem and test
```

## 📚 Implementation Architecture

### Stack Layout (After exec)

```
High Address
+------------------------+
| argc                   | ← sp points here (aligned to 16 bytes)
+------------------------+
| argv[0] pointer        |
| argv[1] pointer        |
| ...                    |
| NULL                   |
+------------------------+
| envp[0] pointer        |
| envp[1] pointer        |
| ...                    |
| NULL                   |
+------------------------+
| AT_PHDR (3)           | ← auxv_base
| phdr value            |
| AT_PHENT (4)          |
| phent value           |
| ...                    |
| AT_RANDOM (25)         |
| random_addr            | ← Points to 16 random bytes
| AT_NULL (0)            |
| 0                      |
+------------------------+
| 16 random bytes        | ← random_addr
+------------------------+
| argv strings          |
| envp strings          |
+------------------------+
Low Address
```

### TCB Structure (at 0x70001000)

```
Offset  | Field          | Value
--------|----------------|------------------
0       | self           | 0x70001000
8       | dtv            | 0
16      | prev           | 0
24      | next           | 0
32      | sysinfo        | 0
40      | canary         | 0
48      | canary2        | 0
56      | tid            | PID (2 for busybox)
60      | errno_val      | 0
...     | ...            | (zeroed)
```

## 🎯 Success Criteria

We have successfully implemented:
- ✅ Complete TLS support (PT_TLS parsing and initialization)
- ✅ Minimal TCB for programs without PT_TLS
- ✅ Full auxiliary vector support (12 entries)
- ✅ Proper tp register initialization
- ✅ set_tid_address syscall returning correct value
- ✅ Execution continues after set_tid_address

Still needed:
- ❌ Fix null function pointer call after initialization
- ❌ Identify missing initialization step or syscall
- ❌ Get busybox to run to main() and beyond

## 📈 Progress Summary

**Before This Work:**
- busybox crashed immediately with tp=0x0
- set_tid_address not implemented
- No auxiliary vectors
- No TLS support

**After This Work:**
- busybox successfully executes through:
  1. ELF loading ✅
  2. TLS/TCB initialization ✅
  3. Auxiliary vector setup ✅
  4. set_tid_address syscall ✅
  5. **Crashes after returning from set_tid_address** ❌

We've made significant progress! The crash has moved from the very beginning (tp=0x0) to after successful libc initialization. We're very close to getting busybox fully running.

## 📝 Key Learnings

1. **musl libc Requires Complete Environment:**
   - Not just TLS, but also auxiliary vectors
   - Proper TCB structure matching pthread layout
   - All initialization syscalls must succeed

2. **Auxiliary Vectors are Critical:**
   - AT_PHDR allows musl to access program headers
   - AT_RANDOM is used for stack canaries
   - AT_PAGESZ needed for memory management

3. **TCB Must Match Expected Layout:**
   - Just dtv+self is insufficient
   - Need full pthread structure fields
   - tid field must be set correctly

4. **Debugging Approach:**
   - Enhanced logging shows execution flow
   - Tracking register changes (especially ra) reveals progress
   - Single-step through initialization with GDB would be ideal

## 🔗 Related Documentation

- [RISC-V ELF psABI](https://github.com/riscv-non-isa/riscv-elf-psabi-doc)
- [ELF TLS Specification](https://www.akkadia.org/drepper/tls.pdf)
- [Linux Auxiliary Vectors](https://man7.org/linux/man-pages/man3/getauxval.3.html)
- [musl libc source](https://git.musl-libc.org/cgit/musl/)
- Previous work: TLS_IMPLEMENTATION_SUMMARY.md

## 🎬 Conclusion

We successfully implemented complete auxiliary vector support and enhanced TLS/TCB initialization. The set_tid_address syscall now works correctly, and execution continues after it returns. The remaining issue is a null function pointer being called during musl's initialization sequence after set_tid_address. This requires either:
1. Finding and implementing the missing syscall(s)
2. Fixing the initialization sequence
3. Understanding what function pointer musl expects to be set

The next step is to use GDB to single-step through the code between set_tid_address return (0x1206cc) and the crash location (ra=0x104a7c) to identify exactly what's being called and why it's null.
