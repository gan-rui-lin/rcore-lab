# rCore-Tutorial-Code

## Code

- [Soure Code of labs](https://github.com/LearningOS/rCore-Tutorial-Code)

## Documents

- Concise Manual: [rCore-Tutorial-Guide](https://LearningOS.github.io/rCore-Tutorial-Guide/)

- Detail Book [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)

## OS API docs of rCore Tutorial Code

- [OS API docs of ch1](https://learningos.github.io/rCore-Tutorial-Code/ch1/os/index.html)
  AND [OS API docs of ch2](https://learningos.github.io/rCore-Tutorial-Code/ch2/os/index.html)
- [OS API docs of ch3](https://learningos.github.io/rCore-Tutorial-Code/ch3/os/index.html)
  AND [OS API docs of ch4](https://learningos.github.io/rCore-Tutorial-Code/ch4/os/index.html)
- [OS API docs of ch5](https://learningos.github.io/rCore-Tutorial-Code/ch5/os/index.html)
  AND [OS API docs of ch6](https://learningos.github.io/rCore-Tutorial-Code/ch6/os/index.html)
- [OS API docs of ch7](https://learningos.github.io/rCore-Tutorial-Code/ch7/os/index.html)
  AND [OS API docs of ch8](https://learningos.github.io/rCore-Tutorial-Code/ch8/os/index.html)
- [OS API docs of ch9](https://learningos.github.io/rCore-Tutorial-Code/ch9/os/index.html)

## Related Resources

- [Learning Resource](https://github.com/LearningOS/rust-based-os-comp2025/blob/main/relatedinfo.md)

## Setup

```bash
$ git clone https://github.com/LearningOS/2025a-rcore-[YOUR_USER_NAME].git
$ cd 2025a-rcore-[YOUR_USER_NAME]
```

## Build & Run

```bash
# setup build&run environment first
$ git clone https://github.com/LearningOS/rCore-Tutorial-Test.git user
$ git checkout ch$ID
$ cd os
# run OS in ch$ID
$ make run
```

If you want to use docker to build and run, you can use the following command:
```bash
# After clone the `rCore-Tutorial-Test` repository to your local machine, you can use the following command to build and run:
$ make build_docker
$ make docker
```

If you experience network issues when accessing foreign resources such as GitHub in Docker, you can follow the following suggestions according to your stage:

- Docker pull:
  1. use proxy: https://docs.docker.com/reference/cli/docker/image/pull/#proxy-configuration

  2. use available domestic source (self-search)

- Docker build: use proxy https://docs.docker.com/engine/cli/proxy/#build-with-a-proxy-configuration

- Docker run: use proxy option, related operations are similar to `Docker build`, can refer to the relevant materials by yourself


Notice: $ID is from [1-9]

## Grading

```bash
# setup build&run environment first
$ rm -rf ci-user
$ git clone https://github.com/LearningOS/rCore-Tutorial-Checker.git ci-user
$ git clone https://github.com/LearningOS/rCore-Tutorial-Test.git ci-user/user
$ git checkout ch$ID
# check&grade OS in ch$ID with more tests
$ cd ci-user && make test CHAPTER=$ID
```

Notice: $ID is from [3,4,5,6,8]

---

## Project Documentation

This repository includes comprehensive documentation for recent development work:

### 📋 [CHANGELOG.md](CHANGELOG.md)
Quick reference for all major changes and features:
- Phase 1-3 system call implementations
- Interrupt safety infrastructure improvements
- TLS/TCB support fixes
- Complete commit history with references

### 📚 Technical Documentation

#### [UPIntrFreeCell Migration Guide](docs/UPIntrFreeCell-Migration.md)
**Essential reading for contributors!**

Comprehensive guide covering the critical infrastructure upgrade from `UPSafeCell` to `UPIntrFreeCell`:
- **Problem Analysis**: Why interrupt safety matters
- **Migration Scope**: 18 files across task, mm, fs, sync modules
- **API Changes**: Return type modifications and new methods
- **Implementation Details**: Interrupt masking mechanism and RAII guards
- **Developer Guide**: Best practices and common pitfalls
- **Performance Considerations**: Optimization tips and trade-offs

**Quick Start**:
```rust
use crate::sync::UPIntrFreeCell;
use lazy_static::lazy_static;

lazy_static! {
    static ref MY_DATA: UPIntrFreeCell<Data> = unsafe {
        UPIntrFreeCell::new(Data::new())
    };
}

fn use_data() {
    let mut data = MY_DATA.exclusive_access();
    // Interrupts are automatically masked
    data.modify();
    // Interrupts are restored on drop
}
```

#### [Shebang Implementation](docs/shebang-implementation.md)
Details on script interpreter support and busybox integration.

### 🔍 Recent Improvements

**Interrupt Safety** (Commit: `5f3b8ee`)
- All global static variables now use `UPIntrFreeCell`
- Automatic interrupt masking during critical sections
- Nested access support with reference counting
- Zero-overhead abstraction with RAII guarantees

**TLS Support** (Commit: `e47e5c3`)
- Minimal TCB initialization for programs without PT_TLS
- Proper tp register setup for musl-libc compatibility
- Support for statically-linked binaries

**System V IPC** (Commit: `6bf2667`)
- Message queues: `msgget`, `msgsnd`, `msgrcv`, `msgctl`
- Shared memory: `shmget`, `shmat`, `shmdt`, `shmctl`
- Signal extensions: `rt_sigtimedwait`

### 🛠️ Development Setup

Current toolchain:
```bash
rustc 1.80.0-nightly (c987ad527 2024-05-01)
cargo 1.80.0-nightly (6087566b3 2024-04-30)
target: riscv64gc-unknown-none-elf
```

Quick build:
```bash
cd /path/to/rcore-lab
bash run.sh  # Builds and runs with default test suite
```

### 📖 For New Contributors

**Start here**:
1. Read [CHANGELOG.md](CHANGELOG.md) to understand project history
2. Review [UPIntrFreeCell Migration Guide](docs/UPIntrFreeCell-Migration.md) for current architecture
3. Check existing commits for code style and documentation standards

**When adding global state**:
- ✅ Use `UPIntrFreeCell` for interrupt safety
- ✅ Keep critical sections small
- ✅ Prefer `try_exclusive_access` for fallible operations
- ✅ Document complex synchronization patterns

**Documentation standards**:
- Update CHANGELOG.md for all significant changes
- Create detailed technical docs for infrastructure changes
- Include code examples and usage patterns
- Document known issues and future optimizations

---

## Maintenance

**Branch**: `zjy-syscall`
**Latest Commit**: `578daf6` - docs: 添加项目更新日志
**Status**: Active development

For questions or contributions, please refer to the documentation above or contact the development team.
