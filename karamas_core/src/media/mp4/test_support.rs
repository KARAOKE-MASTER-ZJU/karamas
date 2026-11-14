//! MP4 索引及 HTTP 测试共用的媒体夹具。
use super::{boxes::atom, indexed::Mp4Builder};

pub(super) fn box_of(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
    atom(kind, &body).unwrap()
}
pub(super) fn ints(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
pub(super) fn fixture_moov(edit: bool) -> Vec<u8> {
    let mut media_header = vec![0; 100];
    media_header[12..16].copy_from_slice(&1000u32.to_be_bytes());
    let mvhd = box_of(b"mvhd", media_header.clone());
    let mdhd = box_of(b"mdhd", media_header);
    let mut track_header = vec![0; 84];
    track_header[12..16].copy_from_slice(&1u32.to_be_bytes());
    let tkhd = box_of(b"tkhd", track_header);
    let stsd = box_of(b"stsd", ints(&[0, 0]));
    let stbl = box_of(b"stbl", stsd);
    let minf = box_of(b"minf", stbl);
    let mdia = box_of(b"mdia", [mdhd, minf].concat());
    let edts = if edit {
        box_of(b"edts", box_of(b"elst", ints(&[0, 1, 0, 100, 0x10000])))
    } else {
        vec![]
    };
    let trak = box_of(b"trak", [tkhd, edts, mdia].concat());
    let trex = box_of(b"trex", ints(&[0, 1, 1, 1000, 3, 0]));
    box_of(b"moov", [mvhd, trak, box_of(b"mvex", trex)].concat())
}
pub(super) fn fragment(time: u32, trun: Vec<u8>) -> Vec<u8> {
    box_of(
        b"moof",
        box_of(
            b"traf",
            [
                box_of(b"tfhd", ints(&[0x20000, 1])),
                box_of(b"tfdt", ints(&[0, time])),
                box_of(b"trun", trun),
            ]
            .concat(),
        ),
    )
}

pub(super) fn builder(edit: bool) -> Mp4Builder {
    Mp4Builder::new(
        &box_of(b"ftyp", b"isom\0\0\0\0isommp41".to_vec()),
        &fixture_moov(edit),
        0,
    )
    .unwrap()
}
