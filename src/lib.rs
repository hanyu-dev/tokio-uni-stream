#![doc = include_str!("../README.md")]
#![allow(clippy::multiple_inherent_impl)]

#[cfg(not(unix))]
mod default;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub use self::default::{OwnedReadHalf, OwnedWriteHalf, UniStream};
#[cfg(unix)]
pub use self::unix::{OwnedReadHalf, OwnedWriteHalf, UniStream};
