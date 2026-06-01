//! Uniprocessor interior mutability primitives
use arch::{disable_interrupts, enable_interrupts, interrupts_enabled};
use core::any::type_name;
use core::cell::{Ref, RefCell, RefMut, UnsafeCell};
use core::ops::{Deref, DerefMut};
use lazy_static::lazy_static;

/// Wrap a static data structure inside it so that we are
/// able to access it without any `unsafe`.
///
/// We should only use it in uniprocessor.
///
/// In order to get mutable reference of inner data, call
/// `exclusive_access`.
pub struct UPSafeCell<T> {
    /// inner data
    inner: RefCell<T>,
}

unsafe impl<T> Sync for UPSafeCell<T> {}
unsafe impl<T> Send for UPSafeCell<T> {}

impl<T> UPSafeCell<T> {
    /// User is responsible to guarantee that inner struct is only used in
    /// uniprocessor.
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }
    /// Panic if the data has been borrowed.
    pub fn exclusive_access(&self) -> RefMut<'_, T> {
        self.inner.borrow_mut()
    }

    /// Try to borrow the inner value; returns None if already borrowed.
    pub fn try_exclusive_access(&self) -> Option<RefMut<'_, T>> {
        self.inner.try_borrow_mut().ok()
    }
}

struct UPSafeCellRaw<T> {
    inner: UnsafeCell<T>,
}

unsafe impl<T> Sync for UPSafeCellRaw<T> {}

impl<T> UPSafeCellRaw<T> {
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    pub fn get_mut(&self) -> &mut T {
        unsafe { &mut (*self.inner.get()) }
    }
}

struct IntrMaskingInfo {
    nested_level: usize,
    sie_before_masking: bool,
}

#[derive(Copy, Clone)]
struct BorrowSite {
    file: &'static str,
    line: u32,
}

lazy_static! {
    static ref INTR_MASKING_INFO: UPSafeCellRaw<IntrMaskingInfo> =
        unsafe { UPSafeCellRaw::new(IntrMaskingInfo::new()) };
}

impl IntrMaskingInfo {
    pub fn new() -> Self {
        Self {
            nested_level: 0,
            sie_before_masking: false,
        }
    }

    pub fn enter(&mut self) {
        let sie = interrupts_enabled();
        disable_interrupts();
        if self.nested_level == 0 {
            self.sie_before_masking = sie;
        }
        self.nested_level += 1;
    }

    pub fn exit(&mut self) {
        self.nested_level -= 1;
        if self.nested_level == 0 && self.sie_before_masking {
            enable_interrupts();
        }
    }
}

/// Interior-mutable cell that masks S-mode interrupts on access.
pub struct UPIntrFreeCell<T> {
    inner: RefCell<T>,
    last_borrow_site: UnsafeCell<Option<BorrowSite>>,
}

unsafe impl<T> Sync for UPIntrFreeCell<T> {}
unsafe impl<T> Send for UPIntrFreeCell<T> {}

/// RAII guard that automatically unmasks interrupts when dropped.
pub struct UPIntrRef<'a, T> {
    inner: Option<Ref<'a, T>>,
    borrow_site: *mut Option<BorrowSite>,
}

/// RAII guard that automatically unmasks interrupts when dropped.
pub struct UPIntrRefMut<'a, T> {
    inner: Option<RefMut<'a, T>>,
    borrow_site: *mut Option<BorrowSite>,
}

/// Guard returned by [`UPIntrMutex::lock`].
pub type UPIntrMutexGuard<'a, T> = UPIntrRefMut<'a, T>;
/// Shared guard returned by [`UPIntrRwLock::read`].
pub type UPIntrRwLockReadGuard<'a, T> = UPIntrRef<'a, T>;
/// Exclusive guard returned by [`UPIntrRwLock::write`].
pub type UPIntrRwLockWriteGuard<'a, T> = UPIntrRefMut<'a, T>;

impl<T> UPIntrFreeCell<T> {
    /// Create a new interrupt-masking cell.
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
            last_borrow_site: UnsafeCell::new(None),
        }
    }

    /// Borrow the inner value immutably while masking interrupts.
    #[track_caller]
    pub fn shared_access(&self) -> UPIntrRef<'_, T> {
        let caller = core::panic::Location::caller();
        INTR_MASKING_INFO.get_mut().enter();
        match self.inner.try_borrow() {
            Ok(inner) => {
                unsafe {
                    *self.last_borrow_site.get() = Some(BorrowSite {
                        file: caller.file(),
                        line: caller.line(),
                    });
                }
                UPIntrRef {
                    inner: Some(inner),
                    borrow_site: self.last_borrow_site.get(),
                }
            }
            Err(_) => {
                let info = INTR_MASKING_INFO.get_mut();
                let nested_level = info.nested_level;
                let sie_before_masking = info.sie_before_masking;
                let holder = unsafe { *self.last_borrow_site.get() };
                let (holder_file, holder_line) = match holder {
                    Some(site) => (site.file, site.line),
                    None => ("unknown", 0),
                };
                info.exit();
                error!(
                    "[upcell] shared_access conflict type={} cell={:p} caller={}:{} holder={}:{} nested_level={} sie_before_masking={}",
                    type_name::<T>(),
                    self as *const Self,
                    caller.file(),
                    caller.line(),
                    holder_file,
                    holder_line,
                    nested_level,
                    sie_before_masking
                );
                panic!(
                    "UPIntrFreeCell borrow conflict: type={} caller={}:{} holder={}:{}",
                    type_name::<T>(),
                    caller.file(),
                    caller.line(),
                    holder_file,
                    holder_line
                );
            }
        }
    }

    /// Try to borrow the inner value immutably while masking interrupts.
    #[track_caller]
    pub fn try_shared_access(&self) -> Option<UPIntrRef<'_, T>> {
        let caller = core::panic::Location::caller();
        INTR_MASKING_INFO.get_mut().enter();
        match self.inner.try_borrow() {
            Ok(inner) => {
                unsafe {
                    *self.last_borrow_site.get() = Some(BorrowSite {
                        file: caller.file(),
                        line: caller.line(),
                    });
                }
                Some(UPIntrRef {
                    inner: Some(inner),
                    borrow_site: self.last_borrow_site.get(),
                })
            }
            Err(_) => {
                INTR_MASKING_INFO.get_mut().exit();
                None
            }
        }
    }

    /// Borrow the inner value while masking interrupts.
    #[track_caller]
    pub fn exclusive_access(&self) -> UPIntrRefMut<'_, T> {
        let caller = core::panic::Location::caller();
        INTR_MASKING_INFO.get_mut().enter();
        match self.inner.try_borrow_mut() {
            Ok(inner) => {
                unsafe {
                    *self.last_borrow_site.get() = Some(BorrowSite {
                        file: caller.file(),
                        line: caller.line(),
                    });
                }
                UPIntrRefMut {
                    inner: Some(inner),
                    borrow_site: self.last_borrow_site.get(),
                }
            }
            Err(_) => {
                let info = INTR_MASKING_INFO.get_mut();
                let nested_level = info.nested_level;
                let sie_before_masking = info.sie_before_masking;
                let holder = unsafe { *self.last_borrow_site.get() };
                let (holder_file, holder_line) = match holder {
                    Some(site) => (site.file, site.line),
                    None => ("unknown", 0),
                };
                // Keep interrupt nesting state balanced before panic diagnostics.
                info.exit();
                error!(
                    "[upcell] exclusive_access conflict type={} cell={:p} caller={}:{} holder={}:{} nested_level={} sie_before_masking={}",
                    type_name::<T>(),
                    self as *const Self,
                    caller.file(),
                    caller.line(),
                    holder_file,
                    holder_line,
                    nested_level,
                    sie_before_masking
                );
                panic!(
                    "UPIntrFreeCell borrow conflict: type={} caller={}:{} holder={}:{}",
                    type_name::<T>(),
                    caller.file(),
                    caller.line(),
                    holder_file,
                    holder_line
                );
            }
        }
    }

    /// Try to borrow the inner value while masking interrupts; returns None if already borrowed.
    #[track_caller]
    pub fn try_exclusive_access(&self) -> Option<UPIntrRefMut<'_, T>> {
        let caller = core::panic::Location::caller();
        INTR_MASKING_INFO.get_mut().enter();
        match self.inner.try_borrow_mut() {
            Ok(refmut) => {
                unsafe {
                    *self.last_borrow_site.get() = Some(BorrowSite {
                        file: caller.file(),
                        line: caller.line(),
                    });
                }
                Some(UPIntrRefMut {
                    inner: Some(refmut),
                    borrow_site: self.last_borrow_site.get(),
                })
            }
            Err(_) => {
                INTR_MASKING_INFO.get_mut().exit();
                None
            }
        }
    }

    /// Run a closure with exclusive access while masking interrupts.
    pub fn exclusive_session<F, V>(&self, f: F) -> V
    where
        F: FnOnce(&mut T) -> V,
    {
        let mut inner = self.exclusive_access();
        f(inner.deref_mut())
    }
}

/// Interrupt-masking mutex for short uniprocessor kernel critical sections.
pub struct UPIntrMutex<T> {
    inner: UPIntrFreeCell<T>,
}

unsafe impl<T> Sync for UPIntrMutex<T> {}
unsafe impl<T> Send for UPIntrMutex<T> {}

impl<T> UPIntrMutex<T> {
    /// Create a new interrupt-masking mutex.
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: UPIntrFreeCell::new(value),
        }
    }

    /// Lock the mutex and mask interrupts until the returned guard is dropped.
    pub fn lock(&self) -> UPIntrMutexGuard<'_, T> {
        self.inner.exclusive_access()
    }

    /// Try to lock the mutex, returning `None` if it is already borrowed.
    pub fn try_lock(&self) -> Option<UPIntrMutexGuard<'_, T>> {
        self.inner.try_exclusive_access()
    }
}

/// Interrupt-masking read/write lock for uniprocessor kernel state.
pub struct UPIntrRwLock<T> {
    inner: UPIntrFreeCell<T>,
}

unsafe impl<T> Sync for UPIntrRwLock<T> {}
unsafe impl<T> Send for UPIntrRwLock<T> {}

impl<T> UPIntrRwLock<T> {
    /// Create a new interrupt-masking read/write lock.
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: UPIntrFreeCell::new(value),
        }
    }

    /// Borrow the protected value immutably while interrupts are masked.
    pub fn read(&self) -> UPIntrRwLockReadGuard<'_, T> {
        self.inner.shared_access()
    }

    /// Borrow the protected value mutably while interrupts are masked.
    pub fn write(&self) -> UPIntrRwLockWriteGuard<'_, T> {
        self.inner.exclusive_access()
    }

    /// Try to borrow the protected value immutably.
    pub fn try_read(&self) -> Option<UPIntrRwLockReadGuard<'_, T>> {
        self.inner.try_shared_access()
    }

    /// Try to borrow the protected value mutably.
    pub fn try_write(&self) -> Option<UPIntrRwLockWriteGuard<'_, T>> {
        self.inner.try_exclusive_access()
    }
}

impl<'a, T> Drop for UPIntrRef<'a, T> {
    fn drop(&mut self) {
        self.inner = None;
        unsafe {
            *self.borrow_site = None;
        }
        INTR_MASKING_INFO.get_mut().exit();
    }
}

impl<'a, T> Deref for UPIntrRef<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap().deref()
    }
}

impl<'a, T> Drop for UPIntrRefMut<'a, T> {
    fn drop(&mut self) {
        self.inner = None;
        unsafe {
            *self.borrow_site = None;
        }
        INTR_MASKING_INFO.get_mut().exit();
    }
}

impl<'a, T> Deref for UPIntrRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap().deref()
    }
}

impl<'a, T> DerefMut for UPIntrRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap().deref_mut()
    }
}
