pub mod inflate;
pub mod names;
pub mod reader;
pub mod writer;

pub use reader::{MAX_INFLATED_TOTAL, ZipFileEntry, ZipReadError, read_zip, read_zip_within};
pub use writer::{ZipEntry, ZipStreamWriter, build_zip};
