#![no_main]
#![forbid(unsafe_code)]

libfuzzer_sys::fuzz_target!(|bytes: &[u8]| {
    let _ = hdfour_fuzz::decode(hdfour::Codec::Rle, bytes);
});
