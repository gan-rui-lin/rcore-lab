use super::disk::{Fat32Disk, Fat32IoError};
use super::inode::Fat32Inode;
use crate::drivers::BLOCK_DEVICE;
use crate::sync::UPSafeCell;
use alloc::string::String;
use alloc::sync::Arc;
use easy_fs::BlockDevice;
use fatfs::{Error, FatType, FileSystem, FsOptions};

use super::super::core::VfsInode;

pub(super) type Fat32Fs = FileSystem<Fat32Disk>;

pub(in crate::fs::vfs) fn fat32_root() -> Result<Arc<dyn VfsInode>, Error<Fat32IoError>> {
    let (base_lba, total_bytes) = fat32_locate(&BLOCK_DEVICE)?;
    let disk = Fat32Disk::new(BLOCK_DEVICE.clone(), total_bytes, base_lba);
    let fs = FileSystem::new(disk, FsOptions::new())?;
    if fs.fat_type() != FatType::Fat32 {
        return Err(Error::CorruptedFileSystem);
    }
    let fs = Arc::new(unsafe { UPSafeCell::new(fs) });
    Ok(Arc::new(Fat32Inode::new_dir(String::from("/"), fs)))
}

fn fat32_locate(device: &Arc<dyn BlockDevice>) -> Result<(usize, Option<u64>), Error<Fat32IoError>> {
    let sector0 = read_sector(device, 0);
    if let Some(total_bytes) = fat32_total_bytes(&sector0) {
        return Ok((0, Some(total_bytes)));
    }
    let base_lba = fat32_partition_lba(&sector0).ok_or(Error::CorruptedFileSystem)?;
    let boot_sector = read_sector(device, base_lba);
    let total_bytes = fat32_total_bytes(&boot_sector).ok_or(Error::CorruptedFileSystem)?;
    Ok((base_lba as usize, Some(total_bytes)))
}

fn read_sector(device: &Arc<dyn BlockDevice>, lba: u32) -> [u8; 512] {
    let mut sector = [0u8; 512];
    device.read_block(lba as usize, &mut sector);
    sector
}

fn fat32_total_bytes(sector: &[u8; 512]) -> Option<u64> {
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return None;
    }
    if &sector[82..90] != b"FAT32   " {
        return None;
    }
    let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
    if bytes_per_sector != 512 {
        return None;
    }
    let total_sectors_16 = u16::from_le_bytes([sector[19], sector[20]]) as u32;
    let total_sectors_32 = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
    let total_sectors = if total_sectors_16 != 0 {
        total_sectors_16
    } else {
        total_sectors_32
    };
    if total_sectors == 0 {
        return None;
    }
    Some(u64::from(bytes_per_sector) * u64::from(total_sectors))
}

fn fat32_partition_lba(sector: &[u8; 512]) -> Option<u32> {
    const PARTITION_TABLE_OFFSET: usize = 0x1BE;
    const PARTITION_ENTRY_SIZE: usize = 16;
    for i in 0..4 {
        let start = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
        let entry = &sector[start..start + PARTITION_ENTRY_SIZE];
        let part_type = entry[4];
        if part_type == 0x0B || part_type == 0x0C {
            let lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
            if lba != 0 {
                return Some(lba);
            }
        }
    }
    None
}
