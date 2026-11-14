use super::*;
use crate::media::mp4::test_support::*;

#[test]
fn warmup_uses_preceding_keyframe_and_clamps_to_eof() {
    let mut b = builder(false);
    // Explicit flags: sync, non-sync, sync at 0, 1, 2 seconds.
    b.add_fragment(0, 100, &fragment(0, ints(&[0x401, 3, 100, 0, 0x10000, 0])))
        .unwrap();
    let mp4 = b.finish().unwrap();
    let p = mp4.prefix.len() as u64;
    assert_eq!(mp4.seek_warmup_range(0), p..mp4.len);
    assert_eq!(mp4.seek_warmup_range(1), p..mp4.len);
    assert_eq!(mp4.seek_warmup_range(2), p + 6..mp4.len);
    assert_eq!(mp4.seek_warmup_range(u32::MAX), p + 6..mp4.len);
}

#[test]
fn source_tracks_with_same_id_get_unique_output_ids() {
    let mut b = builder(false);
    b.add_source(&fixture_moov(false), 1).unwrap();
    for source in [0, 1] {
        b.add_fragment(source, 100, &fragment(0, ints(&[1, 3, 100])))
            .unwrap();
    }
    let mp4 = b.finish().unwrap();
    let top = box_of(b"root", mp4.prefix[..mp4.prefix.len() - 16].to_vec());
    let moov = child(&top, b"moov").unwrap();
    let tracks: Vec<_> = children(moov)
        .unwrap()
        .into_iter()
        .filter(|c| &c[4..8] == b"trak")
        .collect();
    let ids: Vec<_> = tracks
        .iter()
        .map(|track| u32_at(child(track, b"tkhd").unwrap(), 20).unwrap())
        .collect();
    assert!(ids.iter().all(|id| *id != 0));
    assert_ne!(ids[0], ids[1]);
    assert!(child(moov, b"mvex").is_err());
    assert_eq!(mp4.len - mp4.prefix.len() as u64, 18);
}

#[test]
fn every_partial_range_matches_full_virtual_file() {
    let mut b = builder(false);
    b.add_source(&fixture_moov(false), 1).unwrap();
    for source in [0, 1] {
        b.add_fragment(source, 100, &fragment(0, ints(&[1, 3, 100])))
            .unwrap();
    }
    let mp4 = b.finish().unwrap();
    let sources = [b"abcdefghi", b"123456789"];
    let render = |range| {
        let mut data = Vec::new();
        for part in mp4.read_plan(range).unwrap() {
            match part {
                ReadPart::Header(r) => data.extend_from_slice(&mp4.prefix[r]),
                ReadPart::Source { source, range } => data.extend_from_slice(
                    &sources[source][range.start as usize - 200..range.end as usize - 200],
                ),
            }
        }
        data
    };
    let expected = [mp4.prefix.to_vec(), b"abc123def456ghi789".to_vec()].concat();
    assert_eq!(render(0..mp4.len), expected);
    let p = mp4.prefix.len() as u64;
    // Header/media boundary, arbitrary sample boundaries, EOF and empty ranges.
    for start in p - 5..=mp4.len {
        for end in start..=mp4.len {
            assert_eq!(render(start..end), expected[start as usize..end as usize]);
        }
    }
    assert!(mp4.read_plan(0..mp4.len + 1).is_err());
    assert!(mp4.read_plan(10..9).is_err());
}

#[test]
fn open_ended_edit_duration_is_materialized_without_losing_encoder_delay() {
    let mut b = builder(true);
    b.add_fragment(0, 100, &fragment(0, ints(&[1, 3, 100])))
        .unwrap();
    let mp4 = b.finish().unwrap();
    let top = box_of(b"root", mp4.prefix[..mp4.prefix.len() - 16].to_vec());
    let trak = child(child(&top, b"moov").unwrap(), b"trak").unwrap();
    let elst = child(child(trak, b"edts").unwrap(), b"elst").unwrap();
    assert_eq!(u32_at(elst, 16).unwrap(), 3000);
    assert_eq!(u32_at(elst, 20).unwrap(), 100);
}

#[test]
fn signed_composition_offsets_and_sample_flags_are_preserved() {
    let mut b = builder(false);
    let trun = ints(&[
        0x01000f01,
        2,
        100,
        700,
        5,
        0,
        (-20i32) as u32,
        800,
        6,
        0x10000,
        30,
    ]);
    b.add_fragment(0, 100, &fragment(0, trun)).unwrap();
    let mp4 = b.finish().unwrap();
    let top = box_of(b"root", mp4.prefix[..mp4.prefix.len() - 16].to_vec());
    let trak = child(child(&top, b"moov").unwrap(), b"trak").unwrap();
    let stbl = child(
        child(child(trak, b"mdia").unwrap(), b"minf").unwrap(),
        b"stbl",
    )
    .unwrap();
    assert_eq!(
        &child(stbl, b"ctts").unwrap()[8..],
        ints(&[0x01000000, 2, 1, (-20i32) as u32, 1, 30])
    );
    assert_eq!(
        &child(stbl, b"stts").unwrap()[8..],
        ints(&[0, 2, 1, 700, 1, 800])
    );
    assert_eq!(&child(stbl, b"stss").unwrap()[8..], ints(&[0, 1, 1]));
    assert_eq!(
        mp4.read_plan(mp4.prefix.len() as u64..mp4.len).unwrap(),
        vec![ReadPart::Source {
            source: 0,
            range: 200..211
        }]
    );
}

#[test]
fn truncated_boxes_and_discontinuous_timeline_fail() {
    assert!(header(&[0, 0, 0, 1, b'm', b'o', b'o', b'f']).is_err());
    assert!(time_scale(&box_of(b"mdhd", vec![])).is_err());
    assert_eq!(
        builder(false)
            .add_fragment(0, 100, &fragment(0, vec![]))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    let mut b = builder(false);
    assert!(
        b.add_fragment(0, 100, &fragment(5000, ints(&[1, 3, 100])))
            .is_err()
    );
    let good = fragment(0, ints(&[1, 3, 100]));
    for end in 0..good.len() {
        assert!(builder(false).add_fragment(0, 100, &good[..end]).is_err());
    }
}

#[test]
fn samples_must_stay_inside_their_own_payload() {
    let moof = fragment(0, ints(&[1, 3, 100]));
    builder(false)
        .add_fragment_in_media(0, 100, &moof, 200..209)
        .unwrap();
    assert!(
        builder(false)
            .add_fragment_in_media(0, 100, &moof, 201..209)
            .is_err()
    );
    assert!(
        builder(false)
            .add_fragment_in_media(0, 100, &moof, 200..208)
            .is_err()
    );
    let mut b = builder(false);
    b.add_fragment(0, 100, &moof).unwrap();
    let mp4 = b.finish().unwrap();
    mp4.validate_sources(&[209]).unwrap();
    assert!(mp4.validate_sources(&[208]).is_err());
    assert!(mp4.validate_sources(&[]).is_err());
}
