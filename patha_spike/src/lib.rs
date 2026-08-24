#[no_mangle]
pub extern "C" fn spike_add(a: i32, b: i32) -> i32 { a + b }

#[no_mangle]
pub extern "C" fn spike_multiply(a: i32, b: i32) -> i32 { a * b }
