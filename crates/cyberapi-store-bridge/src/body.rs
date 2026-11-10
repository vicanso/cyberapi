//! A large store or send body goes to the bridge as gzip. The bridge
//! undoes it before the handler reads the JSON. Small calls stay plain:
//! a login is not worth a compressor.

use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;

/// Bodies shorter than this stay as they are.
const COMPRESS_AT: usize = 8 * 1024;

/// `body`, gzipped when that is smaller. The second value is the
/// `Content-Encoding` to send with it.
pub fn encoded_body(body: Vec<u8>) -> (Vec<u8>, Option<&'static str>) {
    if body.len() < COMPRESS_AT {
        return (body, None);
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    if encoder.write_all(&body).is_err() {
        return (body, None);
    }
    match encoder.finish() {
        Ok(compressed) if compressed.len() < body.len() => (compressed, Some("gzip")),
        _ => (body, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn a_short_body_is_not_compressed() {
        let (body, encoding) = encoded_body(b"{}".to_vec());
        assert_eq!(body, b"{}");
        assert_eq!(encoding, None);
    }

    #[test]
    fn a_long_json_body_round_trips_through_gzip() {
        let raw = vec![b'{'; COMPRESS_AT];
        let (body, encoding) = encoded_body(raw.clone());
        assert_eq!(encoding, Some("gzip"));
        assert!(body.len() < raw.len());
        let mut decoder = flate2::read::GzDecoder::new(body.as_slice());
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).expect("gzip");
        assert_eq!(out, raw);
    }
}
