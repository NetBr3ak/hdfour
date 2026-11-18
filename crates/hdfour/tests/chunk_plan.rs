use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use hdfour::{Limits, Memory, Result, Source, dd};

struct HeaderWatch {
    memory: Memory<Vec<u8>>,
    headers: Vec<(u64, usize)>,
    payloads: Vec<std::ops::Range<u64>>,
    reads: Arc<Mutex<BTreeMap<u64, usize>>>,
}

impl Source for HeaderWatch {
    fn len(&self) -> u64 {
        self.memory.len()
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<()> {
        let mut reads = self.reads.lock().expect("header reads");
        if self.headers.contains(&(offset, output.len())) {
            *reads.entry(offset).or_default() += 1;
        }
        let end = offset + output.len() as u64;
        if self
            .payloads
            .iter()
            .any(|range| offset < range.end && end > range.start)
        {
            assert_eq!(reads.len(), 4, "preflight all four selected headers first");
            assert!(reads.values().all(|&count| count == 1));
        }
        self.memory.read_at(offset, output)
    }
}

#[test]
fn partial_chunk_headers_are_prepared_once_before_payload_reads() {
    let bytes = include_bytes!("../../../fixtures/F08.hdf").to_vec();
    let memory = Memory::new(bytes.clone());
    let index = dd::parse(&memory, &Limits::default()).expect("index");
    let mut headers = Vec::new();
    let mut payloads = Vec::new();
    for descriptor in index.iter() {
        let Some(offset) = descriptor.offset else {
            continue;
        };
        if descriptor.is_special() {
            let start = usize::try_from(offset).expect("small offset");
            if bytes.get(start..start + 2) == Some(&[0, 3]) {
                headers.push((
                    offset,
                    usize::try_from(descriptor.length).expect("header length"),
                ));
            }
        } else if descriptor.tag == 40 {
            payloads.push(offset..offset + descriptor.length);
        }
    }
    assert!(!headers.is_empty());
    assert!(!payloads.is_empty());
    let reads = Arc::new(Mutex::new(BTreeMap::new()));
    let file = hdfour::File::from_source(
        HeaderWatch {
            memory,
            headers,
            payloads,
            reads: Arc::clone(&reads),
        },
        Limits::default(),
    )
    .expect("fixture metadata without chunk payload reads");
    assert!(reads.lock().expect("header reads").is_empty());
    let dataset = &file.datasets()[0];
    file.read_window(dataset, &[1, 1], &[3, 5])
        .expect("selected chunk plan");
    let reads = reads.lock().expect("header reads");
    assert_eq!(reads.len(), 4);
    assert!(reads.values().all(|&count| count == 1));
}
