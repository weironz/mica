pub mod inflate;
pub mod names;
pub mod reader;
pub mod writer;

pub use reader::{MAX_INFLATED_TOTAL, ZipFileEntry, read_zip, read_zip_within};
pub use writer::{ZipEntry, build_zip};
