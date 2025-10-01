// POSIX I/O FFI Implementation for TriCTI
// Provides C-compatible functions for file system operations

use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::{AsRawFd, FromRawFd};

// =============================================================================
// FILE OPERATIONS
// =============================================================================

#[no_mangle]
pub extern "C" fn posix_open(path: *const u8, flags: i32, mode: i32) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        let mut options = OpenOptions::new();
        
        // Parse flags
        let o_rdonly = 0;
        let o_wronly = 1;
        let o_rdwr = 2;
        let o_creat = 64;
        let o_trunc = 512;
        let o_append = 1024;
        
        let access_mode = flags & 3;
        if access_mode == o_rdonly {
            options.read(true);
        } else if access_mode == o_wronly {
            options.write(true);
        } else if access_mode == o_rdwr {
            options.read(true).write(true);
        }
        
        if (flags & o_creat) != 0 {
            options.create(true);
        }
        if (flags & o_trunc) != 0 {
            options.truncate(true);
        }
        if (flags & o_append) != 0 {
            options.append(true);
        }
        
        match options.open(path_str) {
            Ok(file) => {
                let fd = file.as_raw_fd();
                std::mem::forget(file); // Prevent file from being closed
                fd
            }
            Err(_) => -1,
        }
    }
}

#[no_mangle]
pub extern "C" fn posix_close(fd: i32) -> i32 {
    unsafe {
        let file = File::from_raw_fd(fd);
        drop(file);
        0
    }
}

#[no_mangle]
pub extern "C" fn posix_read(fd: i32, buf: *mut u8, count: u64) -> i64 {
    unsafe {
        let mut file = File::from_raw_fd(fd);
        let slice = std::slice::from_raw_parts_mut(buf, count as usize);
        
        let result = match file.read(slice) {
            Ok(n) => n as i64,
            Err(_) => -1,
        };
        
        std::mem::forget(file); // Prevent file from being closed
        result
    }
}

#[no_mangle]
pub extern "C" fn posix_write(fd: i32, buf: *const u8, count: u64) -> i64 {
    unsafe {
        let mut file = File::from_raw_fd(fd);
        let slice = std::slice::from_raw_parts(buf, count as usize);
        
        let result = match file.write(slice) {
            Ok(n) => n as i64,
            Err(_) => -1,
        };
        
        std::mem::forget(file); // Prevent file from being closed
        result
    }
}

#[no_mangle]
pub extern "C" fn posix_lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    unsafe {
        let mut file = File::from_raw_fd(fd);
        
        let seek_from = match whence {
            0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => {
                std::mem::forget(file);
                return -1;
            }
        };
        
        let result = match file.seek(seek_from) {
            Ok(pos) => pos as i64,
            Err(_) => -1,
        };
        
        std::mem::forget(file); // Prevent file from being closed
        result
    }
}

#[no_mangle]
pub extern "C" fn posix_fsync(fd: i32) -> i32 {
    unsafe {
        let file = File::from_raw_fd(fd);
        
        let result = match file.sync_all() {
            Ok(_) => 0,
            Err(_) => -1,
        };
        
        std::mem::forget(file); // Prevent file from being closed
        result
    }
}

// =============================================================================
// FILE STATUS
// =============================================================================

#[no_mangle]
pub extern "C" fn posix_stat(path: *const u8, buf: *mut u8) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        let metadata = match std::fs::metadata(path_str) {
            Ok(m) => m,
            Err(_) => return -1,
        };

        // Write metadata to buffer (144 bytes stat structure)
        let stat_buf = std::slice::from_raw_parts_mut(buf, 144);
        
        // Zero out buffer
        for byte in stat_buf.iter_mut() {
            *byte = 0;
        }
        
        // Write st_mode at offset 24
        let mode_ptr = buf.add(24) as *mut u32;
        let mut mode = metadata.permissions().mode();
        
        // Set file type bits
        if metadata.is_file() {
            mode |= 0o100000; // S_IFREG
        } else if metadata.is_dir() {
            mode |= 0o040000; // S_IFDIR
        }
        
        *mode_ptr = mode;
        
        // Write st_size at offset 48
        let size_ptr = buf.add(48) as *mut u64;
        *size_ptr = metadata.len();
        
        // Write timestamps (if available)
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            
            // st_atime at offset 72
            let atime_ptr = buf.add(72) as *mut i64;
            *atime_ptr = metadata.atime();
            
            // st_mtime at offset 88
            let mtime_ptr = buf.add(88) as *mut i64;
            *mtime_ptr = metadata.mtime();
            
            // st_ctime at offset 104
            let ctime_ptr = buf.add(104) as *mut i64;
            *ctime_ptr = metadata.ctime();
        }
        
        0
    }
}

#[no_mangle]
pub extern "C" fn posix_fstat(fd: i32, buf: *mut u8) -> i32 {
    unsafe {
        let file = File::from_raw_fd(fd);
        
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(_) => {
                std::mem::forget(file);
                return -1;
            }
        };
        
        std::mem::forget(file); // Prevent file from being closed
        
        // Similar to posix_stat - write metadata to buffer
        let stat_buf = std::slice::from_raw_parts_mut(buf, 144);
        
        for byte in stat_buf.iter_mut() {
            *byte = 0;
        }
        
        let mode_ptr = buf.add(24) as *mut u32;
        let mut mode = metadata.permissions().mode();
        
        if metadata.is_file() {
            mode |= 0o100000;
        } else if metadata.is_dir() {
            mode |= 0o040000;
        }
        
        *mode_ptr = mode;
        
        let size_ptr = buf.add(48) as *mut u64;
        *size_ptr = metadata.len();
        
        0
    }
}

#[no_mangle]
pub extern "C" fn posix_access(path: *const u8, mode: i32) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        // F_OK = 0 (exists)
        if mode == 0 {
            return if std::path::Path::new(path_str).exists() { 0 } else { -1 };
        }
        
        // For other modes, just check if file exists
        // (proper permission checking would require more platform-specific code)
        if std::path::Path::new(path_str).exists() { 0 } else { -1 }
    }
}

// =============================================================================
// DIRECTORY OPERATIONS
// =============================================================================

#[no_mangle]
pub extern "C" fn posix_opendir(path: *const u8) -> *mut u8 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        match std::fs::read_dir(path_str) {
            Ok(read_dir) => Box::into_raw(Box::new(read_dir)) as *mut u8,
            Err(_) => std::ptr::null_mut(),
        }
    }
}

#[no_mangle]
pub extern "C" fn posix_readdir(dir: *mut u8) -> *mut u8 {
    unsafe {
        let read_dir = &mut *(dir as *mut std::fs::ReadDir);
        
        match read_dir.next() {
            Some(Ok(entry)) => {
                // Allocate dirent structure (275 bytes simplified)
                let dirent = vec![0u8; 275];
                let mut boxed = Box::new(dirent);
                
                // Write entry name at offset 19
                let name = entry.file_name();
                let name_bytes = name.to_string_lossy().as_bytes();
                let name_start = 19;
                let max_name_len = 256;
                
                for (i, &byte) in name_bytes.iter().take(max_name_len - 1).enumerate() {
                    boxed[name_start + i] = byte;
                }
                boxed[name_start + name_bytes.len().min(max_name_len - 1)] = 0; // null terminator
                
                // Write d_type at offset 18
                if let Ok(meta) = entry.metadata() {
                    boxed[18] = if meta.is_dir() { 4 } else if meta.is_file() { 8 } else { 0 };
                }
                
                Box::into_raw(boxed) as *mut u8
            }
            _ => std::ptr::null_mut(),
        }
    }
}

#[no_mangle]
pub extern "C" fn posix_closedir(dir: *mut u8) -> i32 {
    unsafe {
        if dir.is_null() {
            return -1;
        }
        let _read_dir = Box::from_raw(dir as *mut std::fs::ReadDir);
        0
    }
}

#[no_mangle]
pub extern "C" fn posix_mkdir(path: *const u8, mode: u32) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match std::fs::create_dir(path_str) {
            Ok(_) => {
                // Set permissions on Unix
                #[cfg(unix)]
                {
                    use std::fs::Permissions;
                    let perms = Permissions::from_mode(mode);
                    let _ = std::fs::set_permissions(path_str, perms);
                }
                0
            }
            Err(_) => -1,
        }
    }
}

#[no_mangle]
pub extern "C" fn posix_rmdir(path: *const u8) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match std::fs::remove_dir(path_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

// =============================================================================
// FILE MANAGEMENT
// =============================================================================

#[no_mangle]
pub extern "C" fn posix_unlink(path: *const u8) -> i32 {
    unsafe {
        let path_cstr = CStr::from_ptr(path as *const i8);
        let path_str = match path_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match std::fs::remove_file(path_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

#[no_mangle]
pub extern "C" fn posix_rename(old: *const u8, new: *const u8) -> i32 {
    unsafe {
        let old_cstr = CStr::from_ptr(old as *const i8);
        let old_str = match old_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        let new_cstr = CStr::from_ptr(new as *const i8);
        let new_str = match new_cstr.to_str() {
            Ok(s) => s,
            Err(_) => return -1,
        };

        match std::fs::rename(old_str, new_str) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}
