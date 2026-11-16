#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
extern crate std;

pub mod brotli;
pub mod bzip2;
pub mod checksum;
pub mod deflate;
pub mod inflate;
pub mod lz4;
pub mod miniz;
pub mod xz;
