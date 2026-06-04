pub mod inflate;
pub mod names;
pub mod reader;
pub mod writer;

pub use reader::{ZipFileEntry, read_zip};
pub use writer::{ZipEntry, build_zip};
