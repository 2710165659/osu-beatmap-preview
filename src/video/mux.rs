//! Shared H.264 NAL-unit parsing + MP4 muxing helpers.
//!
//! All encoder backends (NVENC, AMF, openh264) emit Annex-B bytestreams
//! (start-code-prefixed NAL units). This module splits them into SPS / PPS /
//! slice and length-prefixes the slices for the `mp4` crate's `Mp4Sample`,
//! matching the format the muxer expects.

/// NAL unit type is the lower 5 bits of the first byte (start code already
/// stripped by `nal_units`).
#[inline]
pub(crate) fn nal_type(nal: &[u8]) -> u8 {
    if nal.is_empty() {
        0
    } else {
        nal[0] & 0x1F
    }
}

/// Strip the Annex-B start-code prefix (`00 00 00 01` or `00 00 01`) that
/// hardware/software encoders prepend to every NAL.
#[allow(dead_code)]
pub(crate) fn nal_payload(nal: &[u8]) -> &[u8] {
    if nal.len() >= 4 && nal[0..4] == [0, 0, 0, 1] {
        &nal[4..]
    } else if nal.len() >= 3 && nal[0..3] == [0, 0, 1] {
        &nal[3..]
    } else {
        nal
    }
}

/// Split an Annex-B bytestream into individual NAL unit payloads (start codes
/// stripped). Handles both 4-byte (`00 00 00 01`) and 3-byte (`00 00 01`)
/// start codes, which is necessary because different encoders emit different
/// variants.
pub(crate) fn split_nals(annexb: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut i = 0;
    let len = annexb.len();
    while i + 3 <= len {
        // detect start code
        let sc_len = if i + 4 <= len && annexb[i..i + 4] == [0, 0, 0, 1] {
            4
        } else if annexb[i..i + 3] == [0, 0, 1] {
            3
        } else {
            i += 1;
            continue;
        };
        let payload_start = i + sc_len;
        // scan for the next start code
        let mut j = payload_start + 1;
        while j + 2 < len {
            if (j + 4 <= len && annexb[j..j + 4] == [0, 0, 0, 1])
                || annexb[j..j + 3] == [0, 0, 1]
            {
                // guard against 00 00 00 01 where the leading 00 is trailing
                // zero-byte of the previous NAL — handle by checking we're at a
                // real boundary (the 3-byte check already covers the 4-byte case
                // because it only looks at [0,0,1]).
                break;
            }
            j += 1;
        }
        let end = if j + 2 < len { j } else { len };
        // strip trailing zero bytes that are part of RBSP stopping/carry-over
        let mut nal_end = end;
        while nal_end > payload_start && annexb[nal_end - 1] == 0 {
            // only strip trailing zeros that precede a start code boundary;
            // a single trailing 0x00 may be legitimate RBSP, but multiple
            // trailing 00 00 before a start code are padding. Keep it simple:
            // only strip if there are >=2 trailing zeros AND we hit a boundary.
            if nal_end - payload_start >= 2
                && annexb[nal_end - 2] == 0
                && end < len
            {
                nal_end -= 1;
            } else {
                break;
            }
        }
        nals.push(&annexb[payload_start..nal_end]);
        i = end;
    }
    nals
}

/// Extract SPS (type 7), PPS (type 8), and length-prefixed slice NALs from an
/// Annex-B encoded bitstream.
///
/// NAL types 6 (SEI), 9 (AUD), and 12 (filler data) are silently dropped —
/// they are not needed for MP4 muxing and some hardware encoders emit them
/// by default. Slice NALs (types 1–5) are concatenated with big-endian
/// 4-byte length prefixes, matching the `mp4` crate's AVC sample format.
pub(crate) fn extract_nals_from_annexb(annexb: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>) {
    let mut sps = None;
    let mut pps = None;
    let mut slice = Vec::new();
    for nal in split_nals(annexb) {
        match nal_type(nal) {
            7 => sps = Some(nal.to_vec()),
            8 => pps = Some(nal.to_vec()),
            // drop SEI(6), AUD(9), filler(12) — not needed for mp4
            6 | 9 | 12 => {}
            _ => {
                slice.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                slice.extend_from_slice(nal);
            }
        }
    }
    (sps, pps, slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_nals_handles_4byte_startcodes() {
        // two NALs: SPS (type 7) + slice (type 5)
        let annexb: &[u8] = &[
            0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e, // SPS
            0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00, // IDR slice
        ];
        let nals = split_nals(annexb);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_type(nals[0]), 7);
        assert_eq!(nal_type(nals[1]), 5);
    }

    #[test]
    fn split_nals_handles_3byte_startcodes() {
        let annexb: &[u8] = &[
            0, 0, 1, 0x67, 0x42, // SPS
            0, 0, 1, 0x68, 0xCE, // PPS
        ];
        let nals = split_nals(annexb);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_type(nals[0]), 7);
        assert_eq!(nal_type(nals[1]), 8);
    }

    #[test]
    fn extract_drops_sei_aud_filler() {
        // SPS + SEI + AUD + slice
        let annexb: &[u8] = &[
            0, 0, 0, 1, 0x67, 0x01, // SPS (type 7)
            0, 0, 0, 1, 0x06, 0x02, // SEI (type 6)
            0, 0, 0, 1, 0x09, 0x10, // AUD (type 9)
            0, 0, 0, 1, 0x65, 0xAA, // slice (type 5)
        ];
        let (sps, pps, slice) = extract_nals_from_annexb(annexb);
        assert_eq!(sps, Some(vec![0x67, 0x01]));
        assert_eq!(pps, None);
        // slice should be length-prefixed: 4-byte BE len + 2-byte payload
        assert_eq!(slice, vec![0, 0, 0, 2, 0x65, 0xAA]);
    }
}
