//! Brotli codecs behind zlib's BrotliCompress / BrotliDecompress streams, on Dropbox's
//! pure-Rust port of Google's brotli (BSD-3-Clause AND MIT).
//!
//! `src/zlib_stream.rs` holds them as handles like its zlib codecs (modes BROTLI_DECODE = 8 and
//! BROTLI_ENCODE = 9, as Node's `zlib.constants`). Node's init params arrive as (key, value)
//! pairs: the encoder takes BROTLI_PARAM_MODE..LARGE_WINDOW (0-6), the decoder
//! DISABLE_RING_BUFFER_REALLOCATION and LARGE_WINDOW (0-1); a key the codec does not take or a
//! value it rejects fails like Node's init (ERR_ZLIB_INITIALIZATION_FAILED). Quality and
//! window are clamped as libbrotli sanitizes them. A decoder error carries Node's code, `ERR_`
//! plus libbrotli's error name (`ERR__ERROR_FORMAT_PADDING_2`), and its negative errno; input
//! that ends inside a stream is Z_BUF_ERROR "unexpected end of file"; bytes after a finished
//! stream are ignored.

use crate::zlib_stream::{ZlibError, FINISH, NO_FLUSH};
use brotli::enc::backward_references::BrotliEncoderParams;
use brotli::enc::encode::{set_parameter, BrotliEncoderParameter};
use brotli::enc::StandardAlloc;
use brotli::{BrotliDecompressStream, BrotliResult, BrotliState, CompressorWriter};
use brotli_decompressor::BrotliDecoderParameter;
use std::io::Write;

pub const BROTLI_DECODE: u32 = 8;
pub const BROTLI_ENCODE: u32 = 9;

const ENCODER_PARAMS: [BrotliEncoderParameter; 7] = [
    BrotliEncoderParameter::BROTLI_PARAM_MODE,
    BrotliEncoderParameter::BROTLI_PARAM_QUALITY,
    BrotliEncoderParameter::BROTLI_PARAM_LGWIN,
    BrotliEncoderParameter::BROTLI_PARAM_LGBLOCK,
    BrotliEncoderParameter::BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING,
    BrotliEncoderParameter::BROTLI_PARAM_SIZE_HINT,
    BrotliEncoderParameter::BROTLI_PARAM_LARGE_WINDOW,
];
const DECODER_PARAMS: [BrotliDecoderParameter; 2] = [
    BrotliDecoderParameter::BROTLI_DECODER_PARAM_DISABLE_RING_BUFFER_REALLOCATION,
    BrotliDecoderParameter::BROTLI_DECODER_PARAM_LARGE_WINDOW,
];

type State = BrotliState<StandardAlloc, StandardAlloc, StandardAlloc>;

pub struct Decoder {
    state: State,
    done: bool,
}

pub enum BrotliCodec {
    /// `None` once finished.
    Encode(Option<Box<CompressorWriter<Vec<u8>>>>),
    Decode(Box<Decoder>),
}

/// The encoder for Node's params, with quality and window clamped as libbrotli does.
fn encoder(params: &[(u32, u32)]) -> Result<BrotliCodec, ZlibError> {
    let mut p = BrotliEncoderParams::default();
    for &(key, value) in params {
        let param = ENCODER_PARAMS.get(key as usize).ok_or(ZlibError::Init)?;
        if !set_parameter(&mut p, *param, value) {
            return Err(ZlibError::Init);
        }
    }
    p.quality = p.quality.clamp(0, 11);
    p.lgwin = p.lgwin.clamp(10, if p.large_window { 30 } else { 24 });
    let writer = CompressorWriter::with_params(Vec::new(), 4096, &p);
    Ok(BrotliCodec::Encode(Some(Box::new(writer))))
}

fn decoder(params: &[(u32, u32)]) -> Result<BrotliCodec, ZlibError> {
    let alloc = StandardAlloc::default;
    let mut state = State::new(alloc(), alloc(), alloc());
    for &(key, value) in params {
        let param = DECODER_PARAMS.get(key as usize).ok_or(ZlibError::Init)?;
        state.set_parameter(*param, value);
    }
    Ok(BrotliCodec::Decode(Box::new(Decoder {
        state,
        done: false,
    })))
}

/// The codec for Node's Brotli `mode`, configured by `params`.
pub fn open(mode: u32, params: &[(u32, u32)]) -> Result<BrotliCodec, ZlibError> {
    if mode == BROTLI_ENCODE {
        encoder(params)
    } else {
        decoder(params)
    }
}

/// Node's code for a libbrotli decoder error: `ERR_` + its name without `BROTLI_DECODER`.
fn decode_error(state: &State) -> ZlibError {
    let name = format!("{:?}", state.error_code);
    ZlibError::Brotli {
        errno: state.error_code as i32,
        code: format!("ERR_{}", name.trim_start_matches("BROTLI_DECODER")),
    }
}

impl Decoder {
    /// Decodes `input` into `out`; true once the stream is complete.
    fn run(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<bool, ZlibError> {
        let mut buffer = vec![0; 16384];
        let (mut available_in, mut input_offset, mut total) = (input.len(), 0, 0);
        loop {
            let (mut available_out, mut output_offset) = (buffer.len(), 0);
            let result = BrotliDecompressStream(
                &mut available_in,
                &mut input_offset,
                input,
                &mut available_out,
                &mut output_offset,
                &mut buffer,
                &mut total,
                &mut self.state,
            );
            out.extend_from_slice(&buffer[..output_offset]);
            match result {
                BrotliResult::NeedsMoreOutput => continue,
                BrotliResult::ResultSuccess => return Ok(true),
                BrotliResult::NeedsMoreInput => return Ok(false),
                BrotliResult::ResultFailure => return Err(decode_error(&self.state)),
            }
        }
    }

    fn write(&mut self, input: &[u8], finish: bool, out: &mut Vec<u8>) -> Result<(), ZlibError> {
        if !self.done {
            self.done = self.run(input, out)?;
        }
        if finish && !self.done {
            return Err(ZlibError::Truncated);
        }
        Ok(())
    }
}

/// Compresses `input` and, for a flush, everything pending; Z_FINISH ends the stream.
fn encode(
    slot: &mut Option<Box<CompressorWriter<Vec<u8>>>>,
    input: &[u8],
    flush: u32,
    out: &mut Vec<u8>,
) {
    let writer = slot.as_mut().expect("a finished Brotli handle is closed");
    // A Vec sink takes every byte and the params were checked when the encoder was made.
    writer.write_all(input).expect("Brotli encoding into a Vec");
    if flush == FINISH {
        out.extend(slot.take().expect("open encoder").into_inner());
        return;
    }
    if flush != NO_FLUSH {
        writer.flush().expect("Brotli flush into a Vec");
    }
    out.append(writer.get_mut());
}

impl BrotliCodec {
    pub fn write(&mut self, input: &[u8], flush: u32, out: &mut Vec<u8>) -> Result<(), ZlibError> {
        match self {
            BrotliCodec::Encode(slot) => {
                encode(slot, input, flush, out);
                Ok(())
            }
            BrotliCodec::Decode(decoder) => decoder.write(input, flush == FINISH, out),
        }
    }
}

#[cfg(test)]
#[path = "brotli_stream_tests.rs"]
mod tests;
