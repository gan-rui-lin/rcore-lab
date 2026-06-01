use alloc::vec;
use alloc::vec::Vec;

use super::errno::{errno, EFAULT};
use crate::config::{MEMORY_END, PAGE_SIZE};
use crate::mm::{
    translated_byte_buffer, translated_byte_buffer_checked, PTEFlags, PageTable, PhysAddr, VirtAddr,
};
use crate::task::current_process;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserReadPolicy {
    StrictChecked,
    DemandPaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserWritePolicy {
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

pub fn for_each_user_write_slice<F>(
    token: usize,
    ptr: *const u8,
    len: usize,
    policy: UserWritePolicy,
    mut f: F,
) -> Result<usize, isize>
where
    F: FnMut(&mut [u8]) -> Result<usize, isize>,
{
    if len == 0 {
        return Ok(0);
    }
    match policy {
        UserWritePolicy::DemandCowWithForkFallback => {
            if user_write_range_mapped(token, ptr, len, true) {
                return walk_user_write_slices(token, ptr, len, true, &mut f);
            }
            if try_resolve_user_cow_writable(token, ptr, len) {
                if user_write_range_mapped(token, ptr, len, true) {
                    return walk_user_write_slices(token, ptr, len, true, &mut f);
                }
            }
            let Some(slices) = legacy_fork_write_fallback(token, ptr, len) else {
                return Err(errno(EFAULT));
            };
            run_write_callback(slices, &mut f)
        }
        UserWritePolicy::RelaxedReadableMapping => {
            if user_write_range_mapped(token, ptr, len, true) {
                return walk_user_write_slices(token, ptr, len, true, &mut f);
            }
            if user_write_range_mapped(token, ptr, len, false) {
                return walk_user_write_slices(token, ptr, len, false, &mut f);
            }
            Err(errno(EFAULT))
        }
    }
}

pub fn copy_to_user_inline(
    token: usize,
    dst: *mut u8,
    data: &[u8],
    policy: UserWritePolicy,
) -> Result<(), isize> {
    if data.is_empty() {
        return Ok(());
    }
    if dst.is_null() {
        return Err(errno(EFAULT));
    }
    let mut offset = 0usize;
    for_each_user_write_slice(token, dst as *const u8, data.len(), policy, |slice| {
        let len = slice.len().min(data.len() - offset);
        slice[..len].copy_from_slice(&data[offset..offset + len]);
        offset += len;
        Ok(len)
    })?;
    if offset == data.len() {
        Ok(())
    } else {
        Err(errno(EFAULT))
    }
}

pub fn write_value_to_user<T: Copy>(
    token: usize,
    dst: *mut T,
    value: T,
    policy: UserWritePolicy,
) -> Result<(), isize> {
    if dst.is_null() {
        return Err(errno(EFAULT));
    }
    let data = unsafe {
        core::slice::from_raw_parts((&value as *const T) as *const u8, core::mem::size_of::<T>())
    };
    copy_to_user_inline(token, dst as *mut u8, data, policy)
}

pub fn copy_from_user(
    token: usize,
    src: *const u8,
    dst: &mut [u8],
    policy: UserReadPolicy,
) -> Result<(), isize> {
    if dst.is_empty() {
        return Ok(());
    }
    let Some(slices) = translated_user_read_buffer(token, src, dst.len(), policy) else {
        return Err(errno(EFAULT));
    };
    let mut offset = 0usize;
    for slice in slices {
        let len = slice.len().min(dst.len() - offset);
        dst[offset..offset + len].copy_from_slice(&slice[..len]);
        offset += len;
        if offset >= dst.len() {
            break;
        }
    }
    if offset == dst.len() {
        Ok(())
    } else {
        Err(errno(EFAULT))
    }
}

pub fn ensure_user_readable(
    token: usize,
    ptr: *const u8,
    len: usize,
    policy: UserReadPolicy,
) -> bool {
    if len == 0 {
        return true;
    }
    translated_user_read_buffer(token, ptr, len, policy).is_some()
}

pub fn ensure_user_writable(
    token: usize,
    ptr: *const u8,
    len: usize,
    policy: UserWritePolicy,
) -> bool {
    if len == 0 {
        return true;
    }
    translated_user_write_buffer(token, ptr, len, policy).is_some()
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

fn walk_user_write_slices<F>(
    token: usize,
    ptr: *const u8,
    len: usize,
    writable: bool,
    f: &mut F,
) -> Result<usize, isize>
where
    F: FnMut(&mut [u8]) -> Result<usize, isize>,
{
    if len == 0 {
        return Ok(0);
    }
    let mut start = ptr as usize;
    let end = start.checked_add(len).ok_or_else(|| errno(EFAULT))?;
    let page_table = PageTable::from_token(token);
    let max_user_ppn = PhysAddr::from(MEMORY_END).floor().0;
    let mut total = 0usize;
    while start < end {
        let start_va = VirtAddr::from(start);
        let vpn = start_va.floor();
        let pte = page_table.translate(vpn).ok_or_else(|| errno(EFAULT))?;
        let flags = pte.flags();
        if !pte.is_valid() || !flags.contains(PTEFlags::U) {
            return Err(errno(EFAULT));
        }
        if writable {
            if !pte.writable() {
                return Err(errno(EFAULT));
            }
        } else if !pte.readable() {
            return Err(errno(EFAULT));
        }
        let ppn = pte.ppn();
        if flags.contains(PTEFlags::U) && ppn.0 >= max_user_ppn {
            return Err(errno(EFAULT));
        }
        let next_page = start
            .checked_add(PAGE_SIZE - start_va.page_offset())
            .ok_or_else(|| errno(EFAULT))?;
        let chunk_end = next_page.min(end);
        let start_offset = start_va.page_offset();
        let end_offset = if chunk_end == next_page {
            PAGE_SIZE
        } else {
            VirtAddr::from(chunk_end).page_offset()
        };
        if start_offset >= end_offset || end_offset > PAGE_SIZE {
            return Err(errno(EFAULT));
        }
        let slice = &mut ppn.get_bytes_array()[start_offset..end_offset];
        let produced = f(slice)?;
        if produced > slice.len() {
            return Err(errno(EFAULT));
        }
        total += produced;
        if produced < slice.len() {
            return Ok(total);
        }
        start = chunk_end;
    }
    Ok(total)
}

fn user_write_range_mapped(token: usize, ptr: *const u8, len: usize, writable: bool) -> bool {
    if len == 0 {
        return true;
    }
    let mut start = ptr as usize;
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let page_table = PageTable::from_token(token);
    let max_user_ppn = PhysAddr::from(MEMORY_END).floor().0;
    while start < end {
        let start_va = VirtAddr::from(start);
        let vpn = start_va.floor();
        let Some(pte) = page_table.translate(vpn) else {
            return false;
        };
        let flags = pte.flags();
        if !pte.is_valid() || !flags.contains(PTEFlags::U) {
            return false;
        }
        if writable {
            if !pte.writable() {
                return false;
            }
        } else if !pte.readable() {
            return false;
        }
        let ppn = pte.ppn();
        if flags.contains(PTEFlags::U) && ppn.0 >= max_user_ppn {
            return false;
        }
        let Some(next_page) = start.checked_add(PAGE_SIZE - start_va.page_offset()) else {
            return false;
        };
        start = next_page.min(end);
    }
    true
}

fn run_write_callback<F>(slices: Vec<&'static mut [u8]>, f: &mut F) -> Result<usize, isize>
where
    F: FnMut(&mut [u8]) -> Result<usize, isize>,
{
    let mut total = 0usize;
    for slice in slices {
        let produced = f(slice)?;
        if produced > slice.len() {
            return Err(errno(EFAULT));
        }
        total += produced;
        if produced < slice.len() {
            break;
        }
    }
    Ok(total)
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
    process.with_memory_set_mut(|memory_set| {
        let mut va = start;
        while va < end {
            let vpn = VirtAddr::from(va).floor();
            let mut pte = page_table.translate(vpn);
            // `translate()` may return a present leaf entry that is invalid (V=0).
            // Treat that the same as "missing" for demand-paged VMAs.
            if pte.map_or(true, |entry| !entry.is_valid()) && !memory_set.handle_demand_fault(va) {
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
            if !flags.contains(PTEFlags::W) && !memory_set.handle_cow_fault(va) {
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
    })
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
    process.with_memory_set_mut(|memory_set| {
        let mut va = start;
        while va < end {
            let vpn = VirtAddr::from(va).floor();
            let mut pte = page_table.translate(vpn);
            // `translate()` may return an invalid leaf entry (V=0) when the page
            // table page exists but the specific user page has not been materialized.
            if pte.map_or(true, |entry| !entry.is_valid()) && !memory_set.handle_demand_fault(va) {
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
    })
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
    let proc_name = process.name();
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
