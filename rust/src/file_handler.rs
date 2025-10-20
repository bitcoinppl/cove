use std::{io::Read as _, path::PathBuf};

use cove_util::result_ext::ResultExt as _;

use crate::multi_format::{MultiFormat, MultiFormatError};

#[derive(Debug, Clone, uniffi::Object)]
pub struct FileHandler {
    file_path: PathBuf,
}

#[derive(Debug, uniffi::Error, thiserror::Error)]
#[uniffi::export(Display)]
pub enum FileHandlerError {
    #[error("File not found")]
    FileNotFound,

    #[error("Unable to open file {0}")]
    OpenFile(String),

    #[error("Unable to read file {0}")]
    ReadFile(String),

    #[error("File is not a recognized format: {0:?}")]
    NotRecognizedFormat(#[from] MultiFormatError),
}

#[uniffi::export]
impl FileHandler {
    #[uniffi::constructor]
    pub fn new(file_path: String) -> Self {
        Self { file_path: PathBuf::from(file_path) }
    }

    #[uniffi::method]
    pub fn read(&self) -> Result<MultiFormat, FileHandlerError> {
        if !self.file_path.exists() {
            return Err(FileHandlerError::FileNotFound);
        }

        let mut file =
            std::fs::File::open(&self.file_path).map_err_str(FileHandlerError::OpenFile)?;

        let mut data = Vec::with_capacity(1024 * 128);
        file.read_to_end(&mut data).map_err_str(FileHandlerError::ReadFile)?;

        let string_or_data = crate::multi_format::StringOrData::new(data);

        let multi_format = string_or_data.try_into()?;
        Ok(multi_format)
    }
}
