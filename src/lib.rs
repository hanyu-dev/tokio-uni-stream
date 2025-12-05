#![doc = include_str!("../README.md")]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::multiple_inherent_impl)]
#![allow(clippy::must_use_candidate)]
#![allow(unknown_lints)]

#[cfg(unix)]
pub mod unix;
pub mod windows;

#[cfg(unix)]
pub use self::unix::{OwnedReadHalf, OwnedWriteHalf, UniListener, UniSocket, UniStream};
#[cfg(windows)]
pub use self::windows::{OwnedReadHalf, OwnedWriteHalf, UniListener, UniSocket, UniStream};
#[cfg(not(any(unix, windows)))]
compile_error!("Unsupported platform: only Unix and Windows are supported");
