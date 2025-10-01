//! Parallel vector operations using Rayon
//!
//! This module provides parallel implementations of common vector operations
//! using the Rayon library for true multi-threaded execution.

use rayon::prelude::*;

#[no_mangle]
pub extern "C" fn par_sum_i64(input: *const i64, len: u64, num_threads: u64) -> i64 {
    if len == 0 {
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().sum()
    })
}

#[no_mangle]
pub extern "C" fn par_product_i64(input: *const i64, len: u64, num_threads: u64) -> i64 {
    if len == 0 {
        return 1;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().product()
    })
}

#[no_mangle]
pub extern "C" fn par_min_i64(input: *const i64, len: u64, num_threads: u64, has_value: *mut u8) -> i64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter().min()
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn par_max_i64(input: *const i64, len: u64, num_threads: u64, has_value: *mut u8) -> i64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter().max()
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn par_sum_u64(input: *const u64, len: u64, num_threads: u64) -> u64 {
    if len == 0 {
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().sum()
    })
}

#[no_mangle]
pub extern "C" fn par_product_u64(input: *const u64, len: u64, num_threads: u64) -> u64 {
    if len == 0 {
        return 1;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().product()
    })
}

#[no_mangle]
pub extern "C" fn par_min_u64(input: *const u64, len: u64, num_threads: u64, has_value: *mut u8) -> u64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter().min()
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn par_max_u64(input: *const u64, len: u64, num_threads: u64, has_value: *mut u8) -> u64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter().max()
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn par_sum_f64(input: *const f64, len: u64, num_threads: u64) -> f64 {
    if len == 0 {
        return 0.0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().sum()
    })
}

#[no_mangle]
pub extern "C" fn par_product_f64(input: *const f64, len: u64, num_threads: u64) -> f64 {
    if len == 0 {
        return 1.0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter().product()
    })
}

#[no_mangle]
pub extern "C" fn par_min_f64(input: *const f64, len: u64, num_threads: u64, has_value: *mut u8) -> f64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0.0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0.0
        }
    }
}

#[no_mangle]
pub extern "C" fn par_max_f64(input: *const f64, len: u64, num_threads: u64, has_value: *mut u8) -> f64 {
    if len == 0 {
        unsafe { *has_value = 0 };
        return 0.0;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let result = pool.install(|| {
        input_slice.par_iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    });

    match result {
        Some(&val) => {
            unsafe { *has_value = 1 };
            val
        }
        None => {
            unsafe { *has_value = 0 };
            0.0
        }
    }
}

// ============================================================================
// PARALLEL MAP OPERATIONS
// ============================================================================

#[no_mangle]
pub extern "C" fn par_map_add_i64(input: *const i64, output: *mut i64, len: u64, addend: i64, num_threads: u64) {
    if len == 0 {
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    let output_slice = unsafe { std::slice::from_raw_parts_mut(output, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter()
            .zip(output_slice.par_iter_mut())
            .for_each(|(&x, out)| *out = x + addend);
    });
}

#[no_mangle]
pub extern "C" fn par_map_mul_i64(input: *const i64, output: *mut i64, len: u64, multiplier: i64, num_threads: u64) {
    if len == 0 {
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    let output_slice = unsafe { std::slice::from_raw_parts_mut(output, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter()
            .zip(output_slice.par_iter_mut())
            .for_each(|(&x, out)| *out = x * multiplier);
    });
}

#[no_mangle]
pub extern "C" fn par_map_square_i64(input: *const i64, output: *mut i64, len: u64, num_threads: u64) {
    if len == 0 {
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    let output_slice = unsafe { std::slice::from_raw_parts_mut(output, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter()
            .zip(output_slice.par_iter_mut())
            .for_each(|(&x, out)| *out = x * x);
    });
}

#[no_mangle]
pub extern "C" fn par_map_negate_i64(input: *const i64, output: *mut i64, len: u64, num_threads: u64) {
    if len == 0 {
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    let output_slice = unsafe { std::slice::from_raw_parts_mut(output, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    pool.install(|| {
        input_slice.par_iter()
            .zip(output_slice.par_iter_mut())
            .for_each(|(&x, out)| *out = -x);
    });
}

// ============================================================================
// PARALLEL FILTER OPERATIONS
// ============================================================================

#[no_mangle]
pub extern "C" fn par_filter_positive_i64(input: *const i64, output: *mut i64, len: u64, result_len: *mut u64, num_threads: u64) {
    if len == 0 {
        unsafe { *result_len = 0 };
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let filtered: Vec<i64> = pool.install(|| {
        input_slice.par_iter()
            .filter(|&&x| x > 0)
            .copied()
            .collect()
    });
    
    unsafe {
        std::ptr::copy_nonoverlapping(filtered.as_ptr(), output, filtered.len());
        *result_len = filtered.len() as u64;
    }
}

#[no_mangle]
pub extern "C" fn par_filter_negative_i64(input: *const i64, output: *mut i64, len: u64, result_len: *mut u64, num_threads: u64) {
    if len == 0 {
        unsafe { *result_len = 0 };
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let filtered: Vec<i64> = pool.install(|| {
        input_slice.par_iter()
            .filter(|&&x| x < 0)
            .copied()
            .collect()
    });
    
    unsafe {
        std::ptr::copy_nonoverlapping(filtered.as_ptr(), output, filtered.len());
        *result_len = filtered.len() as u64;
    }
}

#[no_mangle]
pub extern "C" fn par_filter_even_i64(input: *const i64, output: *mut i64, len: u64, result_len: *mut u64, num_threads: u64) {
    if len == 0 {
        unsafe { *result_len = 0 };
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let filtered: Vec<i64> = pool.install(|| {
        input_slice.par_iter()
            .filter(|&&x| x % 2 == 0)
            .copied()
            .collect()
    });
    
    unsafe {
        std::ptr::copy_nonoverlapping(filtered.as_ptr(), output, filtered.len());
        *result_len = filtered.len() as u64;
    }
}

#[no_mangle]
pub extern "C" fn par_filter_odd_i64(input: *const i64, output: *mut i64, len: u64, result_len: *mut u64, num_threads: u64) {
    if len == 0 {
        unsafe { *result_len = 0 };
        return;
    }

    let input_slice = unsafe { std::slice::from_raw_parts(input, len as usize) };
    
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads as usize)
        .build()
        .unwrap();
    
    let filtered: Vec<i64> = pool.install(|| {
        input_slice.par_iter()
            .filter(|&&x| x % 2 != 0)
            .copied()
            .collect()
    });
    
    unsafe {
        std::ptr::copy_nonoverlapping(filtered.as_ptr(), output, filtered.len());
        *result_len = filtered.len() as u64;
    }
}
