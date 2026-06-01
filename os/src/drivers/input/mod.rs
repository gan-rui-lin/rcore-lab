use crate::drivers::bus::virtio::VirtioHal;
use crate::sync::{Condvar, UPIntrFreeCell};
use crate::task::schedule;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use arch::DeviceKind;
use core::any::Any;
use virtio_drivers::{VirtIOHeader, VirtIOInput};

struct VirtIOInputInner {
    virtio_input: VirtIOInput<'static, VirtioHal>,
    events: VecDeque<u64>,
}

struct VirtIOInputWrapper {
    inner: UPIntrFreeCell<VirtIOInputInner>,
    condvar: Condvar,
}

/// Input device interface.
pub trait InputDevice: Send + Sync + Any {
    /// Read a single input event.
    fn read_event(&self) -> u64;
    /// Handle device interrupt.
    fn handle_irq(&self);
    /// Whether the device event queue is empty.
    fn is_empty(&self) -> bool;
}

lazy_static::lazy_static!(
    /// Global keyboard input device.
    pub static ref KEYBOARD_DEVICE: Arc<dyn InputDevice> =
        Arc::new(VirtIOInputWrapper::new(input_base(DeviceKind::InputKeyboard)));
    /// Global mouse input device.
    pub static ref MOUSE_DEVICE: Arc<dyn InputDevice> =
        Arc::new(VirtIOInputWrapper::new(input_base(DeviceKind::InputMouse)));
);

fn input_base(kind: DeviceKind) -> usize {
    arch::platform_config()
        .device(kind)
        .and_then(|device| device.mmio_base())
        .expect("VirtIO input MMIO base missing from platform config")
}

impl VirtIOInputWrapper {
    pub fn new(addr: usize) -> Self {
        let inner = VirtIOInputInner {
            virtio_input: unsafe {
                VirtIOInput::<VirtioHal>::new(&mut *(addr as *mut VirtIOHeader)).unwrap()
            },
            events: VecDeque::new(),
        };
        Self {
            inner: unsafe { UPIntrFreeCell::new(inner) },
            condvar: Condvar::new(),
        }
    }
}

impl InputDevice for VirtIOInputWrapper {
    fn is_empty(&self) -> bool {
        self.inner.exclusive_access().events.is_empty()
    }

    fn read_event(&self) -> u64 {
        loop {
            let mut inner = self.inner.exclusive_access();
            if let Some(event) = inner.events.pop_front() {
                return event;
            } else {
                let task_cx_ptr = self.condvar.wait_no_sched();
                drop(inner);
                schedule(task_cx_ptr);
            }
        }
    }

    fn handle_irq(&self) {
        let mut count = 0;
        let mut result = 0;
        self.inner.exclusive_session(|inner| {
            inner.virtio_input.ack_interrupt();
            while let Some(event) = inner.virtio_input.pop_pending_event() {
                count += 1;
                result = (event.event_type as u64) << 48
                    | (event.code as u64) << 32
                    | (event.value) as u64;
                inner.events.push_back(result);
            }
        });
        if count > 0 {
            self.condvar.signal();
        };
    }
}
