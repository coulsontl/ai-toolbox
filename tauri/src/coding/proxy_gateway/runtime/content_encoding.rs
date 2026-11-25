//! HTTP content-encoding helpers for the local proxy gateway.
//!
//! Incoming clients (notably Codex Desktop while signed into official auth) may
//! send compressed JSON request bodies with `Content-Encoding: zstd`. Upstream
//! responses can also arrive compressed when Accept-Encoding is not forced to
//! identity. This module owns the shared decompress path for both sides.

use std::io::Read;

pub(super) const MAX_DECOMPRESSED_BODY_BYTES: usize = 128 * 1024 * 1024;
const DECOMPRESSED_BODY_TOO_LARGE_ERROR: &str =
    "Decompressed body exceeds the maximum allowed size";

/// Split a content-encoding value into ordered codings, dropping identity/empty.
///
/// HTTP allows stacked encodings such as `gzip, zstd`. Repeated headers are
/// equivalent to comma-joining values before calling this helper.
fn split_codings(content_encoding: &str) -> Vec<&str> {
    content_encoding
        .split(',')
        .map(str::trim)
        .filter(|coding| !coding.is_empty() && !coding.eq_ignore_ascii_case("identity"))
        .collect()
}

fn is_single_supported(coding: &str) -> bool {
    matches!(
        coding.to_ascii_lowercase().as_str(),
        "gzip" | "x-gzip" | "deflate" | "br" | "zstd" | "zst"
    )
}

fn read_decoder_with_limit(
    decoder: impl Read,
    output_limit: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let mut limited_decoder = decoder.take(output_limit.saturating_add(1) as u64);
    let mut decompressed = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = limited_decoder.read(&mut buffer)?;
        if read == 0 {
            return Ok(decompressed);
        }
        if decompressed.len().saturating_add(read) > output_limit {
            return Err(decompressed_body_too_large_error());
        }
        decompressed.extend_from_slice(&buffer[..read]);
    }
}

fn decompressed_body_too_large_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        DECOMPRESSED_BODY_TOO_LARGE_ERROR,
    )
}

pub(super) fn is_decompressed_body_too_large(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::InvalidData
        && error.to_string() == DECOMPRESSED_BODY_TOO_LARGE_ERROR
}

fn decompress_single(
    coding: &str,
    body: &[u8],
    output_limit: usize,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    match coding.to_ascii_lowercase().as_str() {
        "gzip" | "x-gzip" => {
            let decoder = flate2::read::GzDecoder::new(body);
            read_decoder_with_limit(decoder, output_limit).map(Some)
        }
        "deflate" => {
            // RFC 9110: deflate means zlib-wrapped. Some clients send raw deflate.
            let zlib = flate2::read::ZlibDecoder::new(body);
            match read_decoder_with_limit(zlib, output_limit) {
                Ok(decompressed) => Ok(Some(decompressed)),
                Err(error) if is_decompressed_body_too_large(&error) => Err(error),
                Err(_) => {
                    let raw = flate2::read::DeflateDecoder::new(body);
                    read_decoder_with_limit(raw, output_limit).map(Some)
                }
            }
        }
        "br" => {
            let decoder = brotli::Decompressor::new(body, 4096);
            read_decoder_with_limit(decoder, output_limit).map(Some)
        }
        "zstd" | "zst" => {
            // Codex official-login clients commonly use Compression::Zstd.
            let decoder = zstd::stream::read::Decoder::new(body)?;
            read_decoder_with_limit(decoder, output_limit).map(Some)
        }
        _ => Ok(None),
    }
}

/// Decompress body bytes for a content-encoding value, including stacked codings.
///
/// RFC 9110 §8.4 lists codings in application order, so decoding must reverse them.
/// Returns `Ok(None)` when any coding is unsupported so callers can keep the
/// original body and headers instead of half-decoding.
pub(super) fn decompress_body(
    content_encoding: &str,
    body: &[u8],
    output_limit: usize,
) -> Result<Option<Vec<u8>>, std::io::Error> {
    let codings = split_codings(content_encoding);
    if codings.is_empty() {
        return Ok(None);
    }
    if !codings.iter().all(|coding| is_single_supported(coding)) {
        return Ok(None);
    }

    let mut data: Option<Vec<u8>> = None;
    for coding in codings.iter().rev() {
        let input = data.as_deref().unwrap_or(body);
        match decompress_single(coding, input, output_limit)? {
            Some(decompressed) => data = Some(decompressed),
            None => return Ok(None),
        }
    }
    Ok(data)
}

/// Whether every non-identity coding in the value can be decompressed.
pub(super) fn is_supported_content_encoding(content_encoding: &str) -> bool {
    let codings = split_codings(content_encoding);
    !codings.is_empty() && codings.iter().all(|coding| is_single_supported(coding))
}

/// Collect content-encoding from raw header pairs, joining repeated headers.
pub(super) fn get_content_encoding_from_pairs(headers: &[(String, String)]) -> Option<String> {
    let combined = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
        .to_ascii_lowercase();
    if split_codings(&combined).is_empty() {
        return None;
    }
    Some(combined)
}

/// Collect content-encoding from a reqwest/header map, joining repeated headers.
pub(super) fn get_content_encoding_from_header_map(
    headers: &reqwest::header::HeaderMap,
) -> Option<String> {
    let combined = headers
        .get_all(reqwest::header::CONTENT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
        .to_ascii_lowercase();
    if split_codings(&combined).is_empty() {
        return None;
    }
    Some(combined)
}

/// Decompress a body when content-encoding is present and supported.
///
/// Returns `(body, true)` after a successful decode, or `(body, false)` when no
/// encoding is present / only identity. Unsupported encodings leave the body
/// unchanged so callers can keep the original content-encoding header.
pub(super) fn maybe_decompress_encoded_body(
    content_encoding: Option<&str>,
    body: Vec<u8>,
    output_limit: usize,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let Some(encoding) = content_encoding else {
        return Ok((body, false));
    };
    match decompress_body(encoding, &body, output_limit)? {
        Some(decompressed) => Ok((decompressed, true)),
        None => Ok((body, false)),
    }
}

/// Drop entity headers that become stale after body decompression.
pub(super) fn strip_decoded_entity_headers(headers: &mut Vec<(String, String)>) {
    headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("content-encoding")
            && !name.eq_ignore_ascii_case("content-length")
            && !name.eq_ignore_ascii_case("transfer-encoding")
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue, CONTENT_ENCODING};
    use std::io::Write;

    const TEST_OUTPUT_LIMIT: usize = 1024 * 1024;

    #[test]
    fn decompress_body_deflate_handles_zlib_wrapped_per_rfc9110() {
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body("deflate", &compressed, TEST_OUTPUT_LIMIT)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_deflate_falls_back_to_raw_stream() {
        let payload = br#"{"ok":true}"#;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_body("deflate", &compressed, TEST_OUTPUT_LIMIT)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_gzip_and_x_gzip_roundtrip() {
        let payload = br#"{"hello":"gzip"}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        let compressed = encoder.finish().unwrap();

        assert_eq!(
            decompress_body("gzip", &compressed, TEST_OUTPUT_LIMIT)
                .unwrap()
                .unwrap(),
            payload
        );
        assert_eq!(
            decompress_body("x-gzip", &compressed, TEST_OUTPUT_LIMIT)
                .unwrap()
                .unwrap(),
            payload
        );
    }

    #[test]
    fn decompress_body_brotli_roundtrip() {
        let payload = br#"{"hello":"br"}"#;
        let mut compressed = Vec::new();
        brotli::BrotliCompress(
            &mut std::io::Cursor::new(payload),
            &mut compressed,
            &brotli::enc::BrotliEncoderParams::default(),
        )
        .unwrap();

        let decompressed = decompress_body("br", &compressed, TEST_OUTPUT_LIMIT)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_zstd_roundtrip() {
        let payload = br#"{"hello":"world","n":42}"#;
        let compressed = zstd::stream::encode_all(std::io::Cursor::new(&payload[..]), 0).unwrap();
        let decompressed = decompress_body("zstd", &compressed, TEST_OUTPUT_LIMIT)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
        assert_eq!(
            decompress_body("zst", &compressed, TEST_OUTPUT_LIMIT)
                .unwrap()
                .unwrap(),
            payload
        );
    }

    #[test]
    fn decompress_body_stacked_gzip_then_zstd_decodes_in_reverse() {
        let payload = br#"{"stacked":true}"#;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(payload).unwrap();
        let gzipped = gz.finish().unwrap();
        let stacked = zstd::stream::encode_all(std::io::Cursor::new(&gzipped[..]), 0).unwrap();

        let decompressed = decompress_body("gzip, zstd", &stacked, TEST_OUTPUT_LIMIT)
            .unwrap()
            .unwrap();
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn decompress_body_enforces_output_limit_for_every_supported_coding() {
        let payload = vec![b'x'; 1024];

        let mut gzip_encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip_encoder.write_all(&payload).unwrap();
        let gzip = gzip_encoder.finish().unwrap();

        let mut zlib_encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib_encoder.write_all(&payload).unwrap();
        let zlib = zlib_encoder.finish().unwrap();

        let mut raw_deflate_encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        raw_deflate_encoder.write_all(&payload).unwrap();
        let raw_deflate = raw_deflate_encoder.finish().unwrap();

        let mut brotli = Vec::new();
        brotli::BrotliCompress(
            &mut std::io::Cursor::new(&payload),
            &mut brotli,
            &brotli::enc::BrotliEncoderParams::default(),
        )
        .unwrap();

        let zstd = zstd::stream::encode_all(std::io::Cursor::new(&payload), 0).unwrap();

        for (coding, compressed) in [
            ("gzip", gzip),
            ("deflate", zlib),
            ("deflate", raw_deflate),
            ("br", brotli),
            ("zstd", zstd),
        ] {
            let error = decompress_body(coding, &compressed, 64).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{coding}");
            assert_eq!(error.to_string(), DECOMPRESSED_BODY_TOO_LARGE_ERROR);
        }
    }

    #[test]
    fn decompress_body_enforces_output_limit_on_each_stacked_layer() {
        let payload = vec![b'x'; 1024];
        let mut gzip_encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip_encoder.write_all(&payload).unwrap();
        let gzip = gzip_encoder.finish().unwrap();
        let stacked = zstd::stream::encode_all(std::io::Cursor::new(&gzip), 0).unwrap();

        let error = decompress_body("gzip, zstd", &stacked, 64).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), DECOMPRESSED_BODY_TOO_LARGE_ERROR);
    }

    #[test]
    fn decompress_body_stacked_with_unsupported_returns_none() {
        let result =
            decompress_body("snappy, zstd", b"\x00\x01\x02\x03", TEST_OUTPUT_LIMIT).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn decompress_body_unknown_encoding_returns_none_to_keep_headers() {
        let result = decompress_body("snappy", b"\x00\x01\x02\x03", TEST_OUTPUT_LIMIT).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn is_supported_content_encoding_matches_decompressable() {
        for encoding in [
            "gzip",
            "x-gzip",
            "deflate",
            "br",
            "zstd",
            "zst",
            "gzip, zstd",
            "GZIP",
            "Zstd",
        ] {
            assert!(
                is_supported_content_encoding(encoding),
                "{encoding} should be supported"
            );
        }
        for encoding in ["identity", "snappy", "compress", "", "gzip, snappy"] {
            assert!(
                !is_supported_content_encoding(encoding),
                "{encoding} should not be supported"
            );
        }
    }

    #[test]
    fn get_content_encoding_from_pairs_combines_repeated_headers() {
        let headers = vec![
            ("Content-Encoding".to_string(), "gzip".to_string()),
            ("content-encoding".to_string(), "zstd".to_string()),
        ];
        assert_eq!(
            get_content_encoding_from_pairs(&headers).as_deref(),
            Some("gzip, zstd")
        );
    }

    #[test]
    fn get_content_encoding_from_pairs_ignores_identity_only() {
        let headers = vec![("content-encoding".to_string(), "identity".to_string())];
        assert_eq!(get_content_encoding_from_pairs(&headers), None);
    }

    #[test]
    fn get_content_encoding_from_header_map_combines_repeated_headers() {
        let mut headers = HeaderMap::new();
        headers.append(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        headers.append(CONTENT_ENCODING, HeaderValue::from_static("zstd"));
        assert_eq!(
            get_content_encoding_from_header_map(&headers).as_deref(),
            Some("gzip, zstd")
        );
    }
}
