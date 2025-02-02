use std::io::{self, Read};

use rand::{Rng, RngCore};
use std::{cmp, iter};

use super::decoder::{DecoderReader, BUF_SIZE};
use crate::encode::encode_engine_string;
use crate::engine::fast_portable::FastPortable;
use crate::engine::DEFAULT_ENGINE;
use crate::tests::{random_alphabet, random_config, random_engine};
use crate::{decode_engine_vec, DecodeError};

#[test]
fn simple() {
    let tests: &[(&[u8], &[u8])] = &[
        (&b"0"[..], &b"MA=="[..]),
        (b"01", b"MDE="),
        (b"012", b"MDEy"),
        (b"0123", b"MDEyMw=="),
        (b"01234", b"MDEyMzQ="),
        (b"012345", b"MDEyMzQ1"),
        (b"0123456", b"MDEyMzQ1Ng=="),
        (b"01234567", b"MDEyMzQ1Njc="),
        (b"012345678", b"MDEyMzQ1Njc4"),
        (b"0123456789", b"MDEyMzQ1Njc4OQ=="),
    ][..];

    for (text_expected, base64data) in tests.iter() {
        // Read n bytes at a time.
        for n in 1..base64data.len() + 1 {
            let mut wrapped_reader = io::Cursor::new(base64data);
            let mut decoder = DecoderReader::from(&mut wrapped_reader, &DEFAULT_ENGINE);

            // handle errors as you normally would
            let mut text_got = Vec::new();
            let mut buffer = vec![0u8; n];
            while let Ok(read) = decoder.read(&mut buffer[..]) {
                if read == 0 {
                    break;
                }
                text_got.extend_from_slice(&buffer[..read]);
            }

            assert_eq!(
                text_got,
                *text_expected,
                "\nGot: {}\nExpected: {}",
                String::from_utf8_lossy(&text_got[..]),
                String::from_utf8_lossy(text_expected)
            );
        }
    }
}

// Make sure we error out on trailing junk.
#[test]
fn trailing_junk() {
    let tests: &[&[u8]] = &[&b"MDEyMzQ1Njc4*!@#$%^&"[..], b"MDEyMzQ1Njc4OQ== "][..];

    for base64data in tests.iter() {
        // Read n bytes at a time.
        for n in 1..base64data.len() + 1 {
            let mut wrapped_reader = io::Cursor::new(base64data);
            let mut decoder = DecoderReader::from(&mut wrapped_reader, &DEFAULT_ENGINE);

            // handle errors as you normally would
            let mut buffer = vec![0u8; n];
            let mut saw_error = false;
            loop {
                match decoder.read(&mut buffer[..]) {
                    Err(_) => {
                        saw_error = true;
                        break;
                    }
                    Ok(read) if read == 0 => break,
                    Ok(_) => (),
                }
            }

            assert!(saw_error);
        }
    }
}

#[test]
fn handles_short_read_from_delegate() {
    let mut rng = rand::thread_rng();
    let mut bytes = Vec::new();
    let mut b64 = String::new();
    let mut decoded = Vec::new();

    for _ in 0..10_000 {
        bytes.clear();
        b64.clear();
        decoded.clear();

        let size = rng.gen_range(0, 10 * BUF_SIZE);
        bytes.extend(iter::repeat(0).take(size));
        bytes.truncate(size);
        rng.fill_bytes(&mut bytes[..size]);
        assert_eq!(size, bytes.len());

        let engine = random_engine(&mut rng);
        encode_engine_string(&bytes[..], &mut b64, &engine);

        let mut wrapped_reader = io::Cursor::new(b64.as_bytes());
        let mut short_reader = RandomShortRead {
            delegate: &mut wrapped_reader,
            rng: &mut rng,
        };

        let mut decoder = DecoderReader::from(&mut short_reader, &engine);

        let decoded_len = decoder.read_to_end(&mut decoded).unwrap();
        assert_eq!(size, decoded_len);
        assert_eq!(&bytes[..], &decoded[..]);
    }
}

#[test]
fn read_in_short_increments() {
    let mut rng = rand::thread_rng();
    let mut bytes = Vec::new();
    let mut b64 = String::new();
    let mut decoded = Vec::new();

    for _ in 0..10_000 {
        bytes.clear();
        b64.clear();
        decoded.clear();

        let size = rng.gen_range(0, 10 * BUF_SIZE);
        bytes.extend(iter::repeat(0).take(size));
        // leave room to play around with larger buffers
        decoded.extend(iter::repeat(0).take(size * 3));

        rng.fill_bytes(&mut bytes[..]);
        assert_eq!(size, bytes.len());

        let engine = random_engine(&mut rng);

        encode_engine_string(&bytes[..], &mut b64, &engine);

        let mut wrapped_reader = io::Cursor::new(&b64[..]);
        let mut decoder = DecoderReader::from(&mut wrapped_reader, &engine);

        consume_with_short_reads_and_validate(&mut rng, &bytes[..], &mut decoded, &mut decoder);
    }
}

#[test]
fn read_in_short_increments_with_short_delegate_reads() {
    let mut rng = rand::thread_rng();
    let mut bytes = Vec::new();
    let mut b64 = String::new();
    let mut decoded = Vec::new();

    for _ in 0..10_000 {
        bytes.clear();
        b64.clear();
        decoded.clear();

        let size = rng.gen_range(0, 10 * BUF_SIZE);
        bytes.extend(iter::repeat(0).take(size));
        // leave room to play around with larger buffers
        decoded.extend(iter::repeat(0).take(size * 3));

        rng.fill_bytes(&mut bytes[..]);
        assert_eq!(size, bytes.len());

        let engine = random_engine(&mut rng);

        encode_engine_string(&bytes[..], &mut b64, &engine);

        let mut base_reader = io::Cursor::new(&b64[..]);
        let mut decoder = DecoderReader::from(&mut base_reader, &engine);
        let mut short_reader = RandomShortRead {
            delegate: &mut decoder,
            rng: &mut rand::thread_rng(),
        };

        consume_with_short_reads_and_validate(&mut rng, &bytes[..], &mut decoded, &mut short_reader)
    }
}

#[test]
fn reports_invalid_last_symbol_correctly() {
    let mut rng = rand::thread_rng();
    let mut bytes = Vec::new();
    let mut b64 = String::new();
    let mut b64_bytes = Vec::new();
    let mut decoded = Vec::new();
    let mut bulk_decoded = Vec::new();

    for _ in 0..1_000 {
        bytes.clear();
        b64.clear();
        b64_bytes.clear();

        let size = rng.gen_range(1, 10 * BUF_SIZE);
        bytes.extend(iter::repeat(0).take(size));
        decoded.extend(iter::repeat(0).take(size));
        rng.fill_bytes(&mut bytes[..]);
        assert_eq!(size, bytes.len());

        let config = random_config(&mut rng);
        let alphabet = random_alphabet(&mut rng);
        // changing padding will cause invalid padding errors when we twiddle the last byte
        let engine = FastPortable::from(alphabet, config.with_encode_padding(false));
        encode_engine_string(&bytes[..], &mut b64, &engine);
        b64_bytes.extend(b64.bytes());
        assert_eq!(b64_bytes.len(), b64.len());

        // change the last character to every possible symbol. Should behave the same as bulk
        // decoding whether invalid or valid.
        for &s1 in alphabet.symbols.iter() {
            decoded.clear();
            bulk_decoded.clear();

            // replace the last
            *b64_bytes.last_mut().unwrap() = s1;
            let bulk_res = decode_engine_vec(&b64_bytes[..], &mut bulk_decoded, &engine);

            let mut wrapped_reader = io::Cursor::new(&b64_bytes[..]);
            let mut decoder = DecoderReader::from(&mut wrapped_reader, &engine);

            let stream_res = decoder.read_to_end(&mut decoded).map(|_| ()).map_err(|e| {
                e.into_inner()
                    .and_then(|e| e.downcast::<DecodeError>().ok())
            });

            assert_eq!(bulk_res.map_err(|e| Some(Box::new(e))), stream_res);
        }
    }
}

#[test]
fn reports_invalid_byte_correctly() {
    let mut rng = rand::thread_rng();
    let mut bytes = Vec::new();
    let mut b64 = String::new();
    let mut decoded = Vec::new();

    for _ in 0..10_000 {
        bytes.clear();
        b64.clear();
        decoded.clear();

        let size = rng.gen_range(1, 10 * BUF_SIZE);
        bytes.extend(iter::repeat(0).take(size));
        rng.fill_bytes(&mut bytes[..size]);
        assert_eq!(size, bytes.len());

        let engine = random_engine(&mut rng);

        encode_engine_string(&bytes[..], &mut b64, &engine);
        // replace one byte, somewhere, with '*', which is invalid
        let bad_byte_pos = rng.gen_range(0, &b64.len());
        let mut b64_bytes = b64.bytes().collect::<Vec<u8>>();
        b64_bytes[bad_byte_pos] = b'*';

        let mut wrapped_reader = io::Cursor::new(b64_bytes.clone());
        let mut decoder = DecoderReader::from(&mut wrapped_reader, &engine);

        // some gymnastics to avoid double-moving the io::Error, which is not Copy
        let read_decode_err = decoder
            .read_to_end(&mut decoded)
            .map_err(|e| {
                let kind = e.kind();
                let inner = e
                    .into_inner()
                    .and_then(|e| e.downcast::<DecodeError>().ok());
                inner.map(|i| (*i, kind))
            })
            .err()
            .and_then(|o| o);

        let mut bulk_buf = Vec::new();
        let bulk_decode_err = decode_engine_vec(&b64_bytes[..], &mut bulk_buf, &engine).err();

        // it's tricky to predict where the invalid data's offset will be since if it's in the last
        // chunk it will be reported at the first padding location because it's treated as invalid
        // padding. So, we just check that it's the same as it is for decoding all at once.
        assert_eq!(
            bulk_decode_err.map(|e| (e, io::ErrorKind::InvalidData)),
            read_decode_err
        );
    }
}

fn consume_with_short_reads_and_validate<R: Read>(
    rng: &mut rand::rngs::ThreadRng,
    expected_bytes: &[u8],
    decoded: &mut Vec<u8>,
    short_reader: &mut R,
) -> () {
    let mut total_read = 0_usize;
    loop {
        assert!(
            total_read <= expected_bytes.len(),
            "tr {} size {}",
            total_read,
            expected_bytes.len()
        );
        if total_read == expected_bytes.len() {
            assert_eq!(expected_bytes, &decoded[..total_read]);
            // should be done
            assert_eq!(0, short_reader.read(&mut decoded[..]).unwrap());
            // didn't write anything
            assert_eq!(expected_bytes, &decoded[..total_read]);

            break;
        }
        let decode_len = rng.gen_range(1, cmp::max(2, expected_bytes.len() * 2));

        let read = short_reader
            .read(&mut decoded[total_read..total_read + decode_len])
            .unwrap();
        total_read += read;
    }
}

/// Limits how many bytes a reader will provide in each read call.
/// Useful for shaking out code that may work fine only with typical input sources that always fill
/// the buffer.
struct RandomShortRead<'a, 'b, R: io::Read, N: rand::Rng> {
    delegate: &'b mut R,
    rng: &'a mut N,
}

impl<'a, 'b, R: io::Read, N: rand::Rng> io::Read for RandomShortRead<'a, 'b, R, N> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        // avoid 0 since it means EOF for non-empty buffers
        let effective_len = cmp::min(self.rng.gen_range(1, 20), buf.len());

        self.delegate.read(&mut buf[..effective_len])
    }
}
