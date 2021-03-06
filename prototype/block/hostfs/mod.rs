use std::path::PathBuf;

use crate::lock::RwLock;

mod dir;

pub use dir::Dir;

pub fn mount(mount_point: PathBuf) -> RwLock<Dir> {
    RwLock::new(Dir::new(mount_point))
}
