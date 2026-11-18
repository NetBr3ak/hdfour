use std::path::Path;

use hdfour::{Error, File, Limits, Memory, dd, vset};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.hdf")))
        .expect("committed fixture")
}

#[test]
fn unsupported_sd_types_leave_grid_and_swath_fields_unlinked() {
    for fixture_name in ["F15", "F16", "F18"] {
        let bytes = fixture(fixture_name);
        let original = File::from_bytes(bytes.clone()).expect("supported EOS file");
        let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("descriptors");
        for descriptor in index.iter().filter(|descriptor| descriptor.tag == 106) {
            let mut changed = bytes.clone();
            let start = usize::try_from(descriptor.offset.expect("written NT")).expect("offset");
            changed[start] = 2;
            let file = File::from_bytes(changed).expect("unsupported SD type must not reject EOS");
            assert!(file.datasets().len() < original.datasets().len());
            assert!(
                file.skipped()
                    .iter()
                    .any(|entry| entry.feature == "number-type")
            );
            let eos = file.eos().expect("EOS metadata remains available");
            let fields: Vec<_> = eos
                .grids
                .iter()
                .chain(&eos.swaths)
                .flat_map(|object| &object.fields)
                .collect();
            for field in original
                .eos()
                .expect("original EOS")
                .grids
                .iter()
                .chain(&original.eos().expect("original EOS").swaths)
                .flat_map(|object| &object.fields)
                .filter(|field| field.dataset.is_some())
            {
                let skipped = file
                    .skipped()
                    .iter()
                    .any(|entry| entry.feature == "number-type" && entry.object == field.name);
                if skipped {
                    assert!(fields.iter().any(|changed| {
                        changed.name == field.name && changed.dataset.is_none()
                    }));
                }
            }
            if fixture_name == "F15" {
                assert!(
                    fields
                        .iter()
                        .any(|field| { field.name == "temperature" && field.dataset.is_none() })
                );
            }
            for field in fields {
                if let Some(index) = field.dataset {
                    let dataset = &file.datasets()[index];
                    assert_eq!(field.name, dataset.name);
                    file.read(dataset).expect("retained field remains readable");
                }
            }
        }
    }
}

#[test]
fn missing_eos_ndg_reference_is_still_malformed() {
    let mut bytes = fixture("F15");
    let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("descriptors");
    let mut changed = false;
    for descriptor in index.iter().filter(|descriptor| descriptor.tag == 1965) {
        let start = usize::try_from(descriptor.offset.expect("written Vgroup")).expect("offset");
        let end = start + usize::try_from(descriptor.length).expect("length");
        let group = vset::parse_vgroup(&bytes[start..end]).expect("Vgroup");
        if group.class == "Var0.0" {
            continue;
        }
        if let Some(member) = group.members.iter().position(|(tag, _)| *tag == 720) {
            let reference = start + 2 + group.members.len() * 2 + member * 2;
            bytes[reference..reference + 2].copy_from_slice(&65000_u16.to_be_bytes());
            changed = true;
            break;
        }
    }
    assert!(changed, "fixture must contain an EOS dataset reference");
    assert!(matches!(
        File::from_bytes(bytes),
        Err(Error::Malformed {
            what: "EOS NDG has no dataset",
            ..
        })
    ));
}
