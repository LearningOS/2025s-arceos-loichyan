#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]

#[cfg(feature = "axstd")]
use axstd::{print, println};

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    const COLOR_CODES: &[u8] = &[
        30, // black
        31, // red
        32, // green
        33, // yellow
        34, // blue
        35, // magenta
        36, // cyan
        37, // white
        90, // bright black
        91, // bright red
        92, // bright green
        93, // bright yellow
        94, // bright blue
        95, // bright magenta
        96, // bright cyan
        97, // bright white
    ];
    const RESET_CODE: u8 = 0;

    let str = "[WithColor]: Hello, Arceos!";
    let codes = core::iter::repeat_with({
        let mut i = 0;
        move || {
            let c = COLOR_CODES[i];
            i = (i + 1) % COLOR_CODES.len();
            c
        }
    });

    for (ch, cl) in str.chars().zip(codes) {
        if ch.is_alphanumeric() {
            print!("\x1b[{cl}m{ch}\x1b[{RESET_CODE}m");
        } else {
            print!("{ch}");
        }
    }
    println!();
}
