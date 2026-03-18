//! Shared helpers for architecture page-table implementations.
//!
//! This module is the only place where arch page-table code directly touches
//! `ArchInterface` callbacks for frame allocation and kernel token query.

#![allow(missing_docs)]

use crate::api::ArchInterface;

#[inline]
pub fn frame_alloc_persist() -> usize {
    ArchInterface::frame_alloc()
}

#[inline]
pub fn frame_dealloc_persist(ppn: usize) {
    ArchInterface::frame_dealloc(ppn);
}

#[inline]
pub fn kernel_page_table_token_if_ready() -> usize {
    ArchInterface::kernel_page_table_token()
}

#[inline]
pub fn kernel_root_ppn_if_ready(ppn_mask: usize) -> Option<usize> {
    let token = ArchInterface::kernel_page_table_token();
    if token == 0 {
        None
    } else {
        Some(token & ppn_mask)
    }
}
