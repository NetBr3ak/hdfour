//! Bounded positional reads without a shared seek cursor.

use crate::{Error, Result};

/// A byte source whose reads fill the requested buffer or return an error.
pub trait Source: Send + Sync {
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An owned byte container, retained without copying its contents.
#[derive(Debug, Clone)]
pub struct Memory<B> {
    bytes: B,
}

impl<B: AsRef<[u8]>> Memory<B> {
    pub fn new(bytes: B) -> Self {
        Self { bytes }
    }
}

impl<B: AsRef<[u8]> + Send + Sync> Source for Memory<B> {
    fn len(&self) -> u64 {
        self.bytes.as_ref().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        let end = check_range(offset, buf.len() as u64, self.len())?;
        let start = usize::try_from(offset).map_err(|_| bounds_error(offset))?;
        let end = usize::try_from(end).map_err(|_| bounds_error(offset))?;
        let bytes = self
            .bytes
            .as_ref()
            .get(start..end)
            .ok_or_else(|| bounds_error(offset))?;
        buf.copy_from_slice(bytes);
        Ok(())
    }
}

/// A file read through platform positional I/O, without changing its cursor.
#[cfg(any(unix, windows))]
#[derive(Debug)]
pub struct FileSource {
    file: std::fs::File,
    length: u64,
}

#[cfg(any(unix, windows))]
impl FileSource {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        check_range(0, length, length)?;
        Ok(Self { file, length })
    }
}

#[cfg(any(unix, windows))]
impl Source for FileSource {
    fn len(&self) -> u64 {
        self.length
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        check_range(offset, buf.len() as u64, self.length)?;
        read_file_at(&self.file, offset, buf)
    }
}

#[cfg(unix)]
fn read_file_at(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)?;
    Ok(())
}

#[cfg(windows)]
fn read_file_at(file: &std::fs::File, mut offset: u64, mut buf: &mut [u8]) -> Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let read = match file.seek_read(buf, offset) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| bounds_error(offset))?;
        buf = buf.get_mut(read..).ok_or_else(|| bounds_error(offset))?;
    }
    Ok(())
}

fn check_range(offset: u64, length: u64, source_len: u64) -> Result<u64> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| bounds_error(offset))?;
    if end > source_len || end > 2_147_483_647 {
        return Err(bounds_error(offset));
    }
    Ok(end)
}

fn bounds_error(offset: u64) -> Error {
    Error::Malformed {
        offset,
        what: "read outside the file or HDF4 address range",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_source_bounds() -> Result<()> {
        let source = Memory::new(vec![10, 20, 30]);
        assert!(!source.is_empty());
        let mut bytes = [0; 2];
        source.read_at(1, &mut bytes)?;
        assert_eq!(bytes, [20, 30]);
        assert!(matches!(
            source.read_at(2, &mut bytes),
            Err(Error::Malformed { offset: 2, .. })
        ));
        assert!(matches!(
            source.read_at(u64::MAX, &mut bytes),
            Err(Error::Malformed { .. })
        ));
        assert_eq!(bytes, [20, 30]);
        source.read_at(3, &mut [])?;
        assert!(source.read_at(4, &mut []).is_err());
        Ok(())
    }

    #[test]
    fn empty_memory_source() -> Result<()> {
        let source = Memory::new(Vec::<u8>::new());
        assert!(source.is_empty());
        assert_eq!(source.len(), 0);
        source.read_at(0, &mut [])?;
        assert!(matches!(
            source.read_at(0, &mut [0]),
            Err(Error::Malformed { .. })
        ));
        Ok(())
    }

    #[test]
    fn source_rejects_hdf4_address_overflow() {
        assert!(check_range(2_147_483_647, 0, u64::MAX).is_ok());
        assert!(check_range(2_147_483_647, 1, u64::MAX).is_err());
        assert!(check_range(0, 2_147_483_648, u64::MAX).is_err());
        assert!(check_range(u64::MAX, 1, u64::MAX).is_err());
    }

    #[cfg(any(unix, windows))]
    fn temporary_path() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "hdfour-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn file_source_bounds_and_position_independence() -> Result<()> {
        let path = temporary_path();
        std::fs::write(&path, [10, 20, 30])?;
        let source = FileSource::open(&path)?;
        let mut bytes = [0; 2];
        source.read_at(1, &mut bytes)?;
        assert_eq!(bytes, [20, 30]);
        source.read_at(0, &mut bytes)?;
        assert_eq!(bytes, [10, 20]);
        assert!(matches!(
            source.read_at(2, &mut bytes),
            Err(Error::Malformed { .. })
        ));
        source.read_at(3, &mut [])?;
        drop(source);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn file_source_short_read() -> Result<()> {
        let path = temporary_path();
        std::fs::write(&path, [1, 2, 3])?;
        let source = FileSource::open(&path)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)?
            .set_len(1)?;
        let error = source
            .read_at(0, &mut [0; 3])
            .expect_err("truncated file must fail");
        assert!(
            matches!(error, Error::Io(ref error) if error.kind() == std::io::ErrorKind::UnexpectedEof)
        );
        drop(source);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn empty_file_source() -> Result<()> {
        let path = temporary_path();
        std::fs::write(&path, [])?;
        let source = FileSource::open(&path)?;
        assert!(source.is_empty());
        assert_eq!(source.len(), 0);
        source.read_at(0, &mut [])?;
        assert!(matches!(
            source.read_at(0, &mut [0]),
            Err(Error::Malformed { .. })
        ));
        drop(source);
        std::fs::remove_file(path)?;
        Ok(())
    }
}
