use alloc::vec;
use alloc::vec::Vec;

use super::errno::{errno, EFAULT};
use crate::config::PAGE_SIZE;
use crate::mm::{
    translated_byte_buffer, translated_byte_buffer_checked, PageTable, PTEFlags, VirtAddr,
};
use crate::task::current_process;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserReadPolicy {
    StrictChecked,
    DemandPaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserWritePolicy {
    StrictChecked,
    DemandCowWithForkFallback,
    RelaxedReadableMapping,
}

pub fn translated_user_read_buffer(
    token: usize,
    ptr: *const u8,
    len: usize,
    policy: UserReadPolicy,
) -> Option<Vec<&'static mut [u8]>> {
    if len == 0 {
        return Some(Vec::new());
    }
    match policy {
        UserReadPolicy::StrictChecked => translated_byte_buffer_checked(token, ptr, len, false),
        UserReadPolicy::DemandPaged => {
            if let Some(buffers) = translated_byte_buffer_checked(token, ptr, len, false) {
                return Some(buffers);
            }
            if try_resolve_user_readable(token, ptr, len) {
                return translated_byte_buffer_checked(token, ptr, len, false);
            }
            None
        }
    }
}

pub fn translated_user_write_buffer(
    token: usize,
    ptr: *const u8,
    len: usize,
    policy: UserWritePolicy,
) -> Option<Vec<&'static mut [u8]>> {
    if len == 0 {
        return Some(Vec::new());
    }
    match policy {
        UserWritePolicy::StrictChecked => translated_byte_buffer_checked(token, ptr, len, true),
        UserWritePolicy::DemandCowWithForkFallback => {
            if let Some(buffers) = translated_byte_buffer_checked(token, ptr, len, true) {
                return Some(buffers);
            }
            if try_resolve_user_cow_writable(token, ptr, len) {
                if let Some(buffers) = translated_byte_buffer_checked(token, ptr, len, true) {
                    return Some(buffers);
                }
            }
            legacy_fork_write_fallback(token, ptr, len)
        }
        UserWritePolicy::RelaxedReadableMapping => {
            if let Some(buffers) = translated_byte_buffer_checked(token, ptr, len, true) {
                return Some(buffers);
            }
            if is_user_read_mapped(token, ptr, len) {
                return Some(translated_byte_buffer(token, ptr, len));
            }
            None
        }
    }
}

pub fn copy_to_user(
    token: usize,
    dst: *mut u8,
    data: &[u8],
    policy: UserWritePolicy,
) -> Result<(), isize> {
    if data.is_empty() {
        return Ok(());
    }
    let Some(slices) = translated_user_write_buffer(token, dst as *const u8, data.len(), policy)
    else {
        return Err(errno(EFAULT));
    };
    let mut offset = 0usize;
    for slice in slices {
        let len = slice.len().min(data.len() - offset);
        slice[..len].copy_from_slice(&data[offset..offset + len]);
        offset += len;
        if offset >= data.len() {
            break;
        }
    }
    if offset == data.len() {
        Ok(())
    } else {
        Err(errno(EFAULT))
    }
}

pub fn read_from_user<T: Copy>(
    token: usize,
    src: *const T,
    policy: UserReadPolicy,
) -> Result<T, isize> {
    if src.is_null() {
        return Err(errno(EFAULT));
    }
    let size = core::mem::size_of::<T>();
    if size == 0 {
        return Ok(unsafe { core::mem::zeroed() });
    }
    let Some(slices) = translated_user_read_buffer(token, src as *const u8, size, policy) else {
        return Err(errno(EFAULT));
    };
    let mut data = vec![0u8; size];
    let mut offset = 0usize;
    for slice in slices {
        let len = slice.len().min(size - offset);
        data[offset..offset + len].copy_from_slice(&slice[..len]);
        offset += len;
        if offset >= size {
            break;
        }
    }
    if offset != size {
        return Err(errno(EFAULT));
    }
    Ok(unsafe { core::ptr::read_unaligned(data.as_ptr() as *const T) })
}

fn try_resolve_user_cow_writable(token: usize, ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let start = ptr as usize;
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let page_table = PageTable::from_token(token);
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let mut va = start;
    while va < end {
        let vpn = VirtAddr::from(va).floor();
        let mut pte = page_table.translate(vpn);
        if pte.is_none() && !inner.memory_set.handle_demand_fault(va) {
            return false;
        }
        pte = page_table.translate(vpn);
        let Some(pte) = pte else {
            return false;
        };
        let flags = pte.flags();
        if !pte.is_valid() || !flags.contains(PTEFlags::U) {
            return false;
        }
        if !flags.contains(PTEFlags::W) && !inner.memory_set.handle_cow_fault(va) {
            return false;
        }
        let Some(pte_after) = page_table.translate(vpn) else {
            return false;
        };
        let flags_after = pte_after.flags();
        if !pte_after.is_valid()
            || !flags_after.contains(PTEFlags::U)
            || !flags_after.contains(PTEFlags::W)
        {
            return false;
        }
        let next_page = ((va / PAGE_SIZE) + 1) * PAGE_SIZE;
        va = next_page.max(va + 1);
    }
    true
}

fn try_resolve_user_readable(token: usize, ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let start = ptr as usize;
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let page_table = PageTable::from_token(token);
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let mut va = start;
    while va < end {
        let vpn = VirtAddr::from(va).floor();
        let mut pte = page_table.translate(vpn);
        if pte.is_none() && !inner.memory_set.handle_demand_fault(va) {
            return false;
        }
        pte = page_table.translate(vpn);
        let Some(pte) = pte else {
            return false;
        };
        let flags = pte.flags();
        if !pte.is_valid() || !flags.contains(PTEFlags::U) || !flags.contains(PTEFlags::R) {
            return false;
        }
        let next_page = ((va / PAGE_SIZE) + 1) * PAGE_SIZE;
        va = next_page.max(va + 1);
    }
    true
}

fn is_user_read_mapped(token: usize, ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let start = ptr as usize;
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let page_table = PageTable::from_token(token);
    let mut va = start;
    while va < end {
        let vpn = VirtAddr::from(va).floor();
        let Some(pte) = page_table.translate(vpn) else {
            return false;
        };
        let flags = pte.flags();
        if !pte.is_valid() || !flags.contains(PTEFlags::U) || !flags.contains(PTEFlags::R) {
            return false;
        }
        let next_page = ((va / PAGE_SIZE) + 1) * PAGE_SIZE;
        va = next_page.max(va + 1);
    }
    true
}

fn legacy_fork_write_fallback(
    token: usize,
    ptr: *const u8,
    len: usize,
) -> Option<Vec<&'static mut [u8]>> {
    let process = current_process();
    let proc_name = process.inner_exclusive_access().name.clone();
    if !proc_name.starts_with("fork") {
        return None;
    }
    if !is_user_read_mapped(token, ptr, len) {
        return None;
    }
    if translated_byte_buffer_checked(token, ptr, len, false).is_none() {
        return None;
    }
    Some(translated_byte_buffer(token, ptr, len))
}
