#![cfg_attr(not(feature = "std"), no_std)]

#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(feature = "std"), feature(alloc_error_handler))]
#![deny(rustdoc::broken_intra_doc_links)]
#![deny(missing_docs)]
#![feature(error_in_core)]
#![feature(asm_experimental_arch)]

extern crate alloc;

#[cfg(feature = "binfmt")]
pub mod binfmt;
mod control_id;
#[cfg(feature = "prove")]
mod exec;
#[cfg(any(target_os = "zkvm", doc))]
pub mod guest;
#[cfg(feature = "prove")]
mod opcode;
#[cfg(feature = "prove")]
pub mod prove;
// pub mod receipt;
// pub mod serde;
#[cfg(feature = "prove")]
mod session;
#[cfg(not(feature = "minimal"))]
pub mod sha;