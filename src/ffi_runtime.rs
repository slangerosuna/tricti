use std::ffi::{CStr, CString};
use std::os::raw::c_char;

#[cfg(unix)]
use libloading::os::unix::Symbol as RawSymbol;
#[cfg(windows)]
use libloading::os::windows::Symbol as RawSymbol;

#[no_mangle]
pub extern "C" fn ffi_dlopen(path: *const u8) -> *mut std::ffi::c_void {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    
    let path_cstr = unsafe { CStr::from_ptr(path as *const c_char) };
    let path_str = match path_cstr.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        match unsafe { Library::new(path_str) } {
            Ok(lib) => Box::into_raw(Box::new(lib)) as *mut std::ffi::c_void,
            Err(_) => std::ptr::null_mut(),
        }
    }
    
    #[cfg(windows)]
    {
        use libloading::os::windows::Library;
        match unsafe { Library::new(path_str) } {
            Ok(lib) => Box::into_raw(Box::new(lib)) as *mut std::ffi::c_void,
            Err(_) => std::ptr::null_mut(),
        }
    }
    
    #[cfg(not(any(unix, windows)))]
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn ffi_dlsym(handle: *mut std::ffi::c_void, symbol: *const u8) -> *mut std::ffi::c_void {
    if handle.is_null() || symbol.is_null() {
        return std::ptr::null_mut();
    }
    
    let symbol_cstr = unsafe { CStr::from_ptr(symbol as *const c_char) };
    let symbol_bytes = symbol_cstr.to_bytes();
    
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        let lib = unsafe { &*(handle as *const Library) };
        match unsafe { lib.get::<*mut std::ffi::c_void>(symbol_bytes) } {
            Ok(sym) => *sym,
            Err(_) => std::ptr::null_mut(),
        }
    }
    
    #[cfg(windows)]
    {
        use libloading::os::windows::Library;
        let lib = unsafe { &*(handle as *const Library) };
        match unsafe { lib.get::<*mut std::ffi::c_void>(symbol_bytes) } {
            Ok(sym) => *sym,
            Err(_) => std::ptr::null_mut(),
        }
    }
    
    #[cfg(not(any(unix, windows)))]
    std::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn ffi_dlclose(handle: *mut std::ffi::c_void) -> i32 {
    if handle.is_null() {
        return -1;
    }
    
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        unsafe {
            let _ = Box::from_raw(handle as *mut Library);
        }
        0
    }
    
    #[cfg(windows)]
    {
        use libloading::os::windows::Library;
        unsafe {
            let _ = Box::from_raw(handle as *mut Library);
        }
        0
    }
    
    #[cfg(not(any(unix, windows)))]
    -1
}

#[no_mangle]
pub extern "C" fn ffi_dlerror() -> *const u8 {
    std::ptr::null()
}

#[no_mangle]
pub extern "C" fn ffi_malloc(size: u64) -> *mut u8 {
    if size == 0 {
        return std::ptr::null_mut();
    }
    unsafe {
        libc::malloc(size as libc::size_t) as *mut u8
    }
}

#[no_mangle]
pub extern "C" fn ffi_free(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe {
            libc::free(ptr as *mut libc::c_void);
        }
    }
}

#[no_mangle]
pub extern "C" fn ffi_memcpy(dest: *mut u8, src: *const u8, n: u64) -> *mut u8 {
    if dest.is_null() || src.is_null() || n == 0 {
        return dest;
    }
    unsafe {
        libc::memcpy(dest as *mut libc::c_void, src as *const libc::c_void, n as libc::size_t);
    }
    dest
}

#[no_mangle]
pub extern "C" fn ffi_memset(ptr: *mut u8, value: i32, n: u64) -> *mut u8 {
    if ptr.is_null() || n == 0 {
        return ptr;
    }
    unsafe {
        libc::memset(ptr as *mut libc::c_void, value, n as libc::size_t);
    }
    ptr
}
