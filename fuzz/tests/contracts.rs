use hdfour::{Codec, Error};
use hdfour_fuzz::{decode, open_read_all, parse_odl, read_fixture};

#[test]
fn valid_fixture_reaches_every_dataset() {
    let bytes = include_bytes!("../../fixtures/F01.hdf");
    assert_eq!(read_fixture(bytes).unwrap(), 10);
}

#[test]
fn malformed_file_is_an_expected_error() {
    assert!(matches!(open_read_all(b"not HDF4"), Err(Error::NotHdf4)));
}

#[test]
fn codec_length_prefix_is_not_part_of_the_stream() {
    assert_eq!(decode(Codec::Rle, &[3, 0, 0x80, 42]).unwrap(), [42; 3]);
    assert_eq!(
        decode(Codec::Deflate, &[0, 0, 120, 156, 3, 0, 0, 0, 0, 1]).unwrap(),
        []
    );
}

#[test]
fn codec_rejects_short_prefix_and_wrong_output_size() {
    assert!(decode(Codec::Rle, &[0]).is_err());
    assert!(decode(Codec::Rle, &[2, 0, 0x80, 42]).is_err());
}

#[test]
fn utf8_and_structural_metadata_reach_the_parser() {
    assert!(parse_odl(b"END").is_ok());
    assert!(parse_odl(&[0xff]).is_err());
    assert!(parse_odl(include_bytes!("../../fixtures/odl/grid.odl")).is_ok());
}

#[test]
fn fixture_reads_propagate_errors() {
    let bytes = include_bytes!("../../fixtures/F10.hdf");
    assert!(matches!(
        read_fixture(bytes),
        Err(Error::Unsupported {
            feature: "skipping-huffman",
            ..
        })
    ));
}
