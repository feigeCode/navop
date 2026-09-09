use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::Arc,
};

use tempfile::NamedTempFile;

use super::BlobStoreError;

#[derive(Debug)]
pub(super) enum BlobBackend {
    Memory(Arc<[u8]>),
    File(Arc<NamedTempFile>, usize),
}

impl BlobBackend {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Memory(data) => data.len(),
            Self::File(_, len) => *len,
        }
    }

    pub(super) fn spilled(&self) -> bool {
        matches!(self, Self::File(..))
    }

    pub(super) fn read(&self, start: usize, end: usize) -> Result<Vec<u8>, BlobStoreError> {
        match self {
            Self::Memory(data) => Ok(data[start..end].to_vec()),
            Self::File(file, _) => {
                let mut reader = file.reopen().map_err(io_error)?;
                reader
                    .seek(SeekFrom::Start(start as u64))
                    .map_err(io_error)?;
                let mut bytes = vec![0; end - start];
                reader.read_exact(&mut bytes).map_err(io_error)?;
                Ok(bytes)
            }
        }
    }
}

pub(super) enum UploadData {
    Memory(Vec<u8>),
    File(NamedTempFile),
}

impl UploadData {
    pub(super) fn append(
        &mut self,
        chunk: &[u8],
        next: usize,
        spill_threshold: usize,
    ) -> Result<(), BlobStoreError> {
        if matches!(self, Self::Memory(_)) && next > spill_threshold {
            let mut file = NamedTempFile::new().map_err(io_error)?;
            if let Self::Memory(existing) = self {
                file.write_all(existing).map_err(io_error)?;
            }
            file.write_all(chunk).map_err(io_error)?;
            *self = Self::File(file);
            return Ok(());
        }
        match self {
            Self::Memory(bytes) => bytes.extend_from_slice(chunk),
            Self::File(file) => append_file(file, chunk)?,
        }
        Ok(())
    }

    pub(super) fn seal(self, len: usize) -> BlobBackend {
        match self {
            Self::Memory(bytes) => BlobBackend::Memory(Arc::from(bytes)),
            Self::File(file) => BlobBackend::File(Arc::new(file), len),
        }
    }
}

fn append_file(file: &mut NamedTempFile, chunk: &[u8]) -> Result<(), BlobStoreError> {
    let original_len = file.as_file().metadata().map_err(io_error)?.len();
    if let Err(error) = file.write_all(chunk) {
        file.as_file_mut().set_len(original_len).map_err(io_error)?;
        file.as_file_mut()
            .seek(SeekFrom::Start(original_len))
            .map_err(io_error)?;
        return Err(io_error(error));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> BlobStoreError {
    BlobStoreError::Io(error.to_string())
}
