// Dialect restrictions apply with or without contracts.

pub fn danger(p: *const u8) -> u8 {
    unsafe { *p }
}

pub fn run_threads() {
    std::thread::spawn(|| println!("hello"));
}
