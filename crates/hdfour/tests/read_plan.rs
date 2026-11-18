use std::sync::{Arc, Mutex};

use hdfour::{File, Limits, Memory, Source};

struct RecordedSource {
    bytes: Memory<Vec<u8>>,
    reads: Arc<Mutex<Vec<(u64, usize)>>>,
}

impl Source for RecordedSource {
    fn len(&self) -> u64 {
        self.bytes.len()
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> hdfour::Result<()> {
        self.reads
            .lock()
            .expect("read log")
            .push((offset, output.len()));
        self.bytes.read_at(offset, output)
    }
}

#[test]
fn full_chunked_read_prepares_storage_once() {
    let bytes = include_bytes!("../../../fixtures/F08.hdf").to_vec();
    let index = hdfour::dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
    let reads = Arc::new(Mutex::new(Vec::new()));
    let file = File::from_source(
        RecordedSource {
            bytes: Memory::new(bytes),
            reads: reads.clone(),
        },
        Limits::default(),
    )
    .expect("F08");
    reads.lock().expect("read log").clear();
    let array = file.read(&file.datasets()[0]).expect("chunked values");
    assert_eq!(array.shape, file.datasets()[0].shape);
    let reads = reads.lock().expect("read log");
    eprintln!(
        "F08 full read: calls={}, bytes={}",
        reads.len(),
        reads.iter().map(|(_, length)| length).sum::<usize>()
    );
    for descriptor in index.iter().filter(|descriptor| descriptor.tag == 0x403d) {
        let offset = descriptor.offset.expect("stored chunk");
        let request = (
            offset,
            usize::try_from(descriptor.length).expect("chunk header size"),
        );
        assert_eq!(
            reads.iter().filter(|read| **read == request).count(),
            1,
            "chunk {} header reads",
            descriptor.reference
        );
    }
    let table = index.get(1962, 4).expect("F08 chunk table");
    let request = (
        table.offset.expect("table offset"),
        usize::try_from(table.length).expect("table size"),
    );
    assert_eq!(
        reads.iter().filter(|read| **read == request).count(),
        1,
        "chunk table header reads"
    );
}
