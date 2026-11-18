use hdfour::{Error, File, Limits, Memory, Result, Source, Value};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Counted {
    bytes: Memory<&'static [u8]>,
    requests: Arc<AtomicUsize>,
    transferred: Arc<AtomicUsize>,
}

impl Source for Counted {
    fn len(&self) -> u64 {
        self.bytes.len()
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<()> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.transferred.fetch_add(output.len(), Ordering::Relaxed);
        self.bytes.read_at(offset, output)
    }
}

#[test]
fn packed_rows_use_one_positional_read() -> Result<()> {
    let requests = Arc::<AtomicUsize>::default();
    let transferred = Arc::<AtomicUsize>::default();
    let source = Counted {
        bytes: Memory::new(include_bytes!("../../../fixtures/F01.hdf").as_slice()),
        requests: requests.clone(),
        transferred: transferred.clone(),
    };
    let file = File::from_source(source, Limits::default())?;
    let data = file.dataset("i16")?;
    let Value::I16(full) = file.read(data)?.data else {
        panic!("expected i16")
    };
    requests.store(0, Ordering::Relaxed);
    transferred.store(0, Ordering::Relaxed);
    let rows = file.read_window(data, &[1, 0], &[2, 5])?;
    assert_eq!(rows.shape, [2, 5]);
    assert_eq!(rows.data, Value::I16(full[5..].to_vec()));
    assert_eq!(requests.load(Ordering::Relaxed), 1);
    assert_eq!(transferred.load(Ordering::Relaxed), 20);
    Ok(())
}

#[test]
fn invalid_geometry_and_foreign_datasets_are_not_file_errors() -> Result<()> {
    let file = File::from_bytes(include_bytes!("../../../fixtures/F01.hdf"))?;
    let data = file.dataset("i16")?;
    for (start, count) in [
        (&[0][..], &[1, 1][..]),
        (&[0, 0], &[1]),
        (&[4, 0], &[0, 1]),
        (&[1, 2], &[1, u64::MAX]),
    ] {
        assert!(matches!(
            file.read_window(data, start, count),
            Err(Error::InvalidSelection { .. })
        ));
    }
    assert!(matches!(
        file.read_window(&data.clone(), &[0, 0], &[1, 1]),
        Err(Error::NotFound(_))
    ));
    let empty = file.read_window(data, &[3, 1], &[0, 4])?;
    assert_eq!(empty.shape, [0, 4]);
    assert_eq!(empty.data, Value::I16(vec![]));
    Ok(())
}
