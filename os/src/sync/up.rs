//! Uniprocessor interior mutability primitives
use arch::{disable_interrupts, enable_interrupts, interrupts_enabled};
use core::cell::{RefCell, RefMut, UnsafeCell};
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
}

unsafe impl<T> Sync for UPIntrFreeCell<T> {}
unsafe impl<T> Send for UPIntrFreeCell<T> {}

/// RAII guard that automatically unmasks interrupts when dropped.
pub struct UPIntrRefMut<'a, T>(Option<RefMut<'a, T>>);

impl<T> UPIntrFreeCell<T> {
    /// Create a new interrupt-masking cell.
    pub unsafe fn new(value: T) -> Self {
        Self {
            inner: RefCell::new(value),
        }
    }

    /// Borrow the inner value while masking interrupts.
    pub fn exclusive_access(&self) -> UPIntrRefMut<'_, T> {
        INTR_MASKING_INFO.get_mut().enter();
        UPIntrRefMut(Some(self.inner.borrow_mut()))
    }

    /// Try to borrow the inner value while masking interrupts; returns None if already borrowed.
    pub fn try_exclusive_access(&self) -> Option<UPIntrRefMut<'_, T>> {
        INTR_MASKING_INFO.get_mut().enter();
        match self.inner.try_borrow_mut() {
            Ok(refmut) => Some(UPIntrRefMut(Some(refmut))),
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

impl<'a, T> Drop for UPIntrRefMut<'a, T> {
    fn drop(&mut self) {
        self.0 = None;
        INTR_MASKING_INFO.get_mut().exit();
    }
}

impl<'a, T> Deref for UPIntrRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap().deref()
    }
}

impl<'a, T> DerefMut for UPIntrRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().unwrap().deref_mut()
    }
}
