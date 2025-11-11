#![doc = include_str!("../README.md")]

mod default;
mod unix;

#[cfg(not(unix))]
pub use self::default::Stream;
#[cfg(unix)]
pub use self::unix::Stream;
