use std::io::{self, BufRead, Read, Write};

use eyre::Result;
use log::*;

const BROTLI_BUFFER_SIZE: usize = 4096;
const BROTLI_Q: u32 = 42;
const BROTLI_LGWIN: u32 = 69;

const XZ_LEVEL: u32 = 6;

const ZSTD_LEVEL: i32 = 6;

fn detect_stream_characteristics<R: Read>(stream: &mut R) -> Result<(CompressionType, Vec<u8>)> {
    let mut buffer = [0; 6];
    let n = stream.read(&mut buffer)?;
    let buffer = &buffer[..n];
    let kind = detect_compression_type(buffer);

    Ok((kind, Vec::from(buffer)))
}

fn detect_compression_type(buffer: &[u8]) -> CompressionType {
    if buffer.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        CompressionType::Zstd
    } else if buffer.starts_with(&[0x1f, 0x8b]) {
        CompressionType::Gzip
    } else if buffer.starts_with(&[0x78, 0x01]) {
        CompressionType::Deflate
    } else if buffer.starts_with(&[0x78, 0x9c]) {
        CompressionType::Zlib
    } else if buffer.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        CompressionType::Xz
    } else {
        CompressionType::None
    }
}

pub struct Context<'a, R: Read, W: Write> {
    input_compression_type: CompressionType,
    output_compression_type: CompressionType,

    input_stream: &'a mut R,
    output_stream: &'a mut W,
    magic: Vec<u8>,
}

impl<'a, R: Read, W: Write> Context<'a, R, W> {
    pub fn autocompress(
        input_stream: &'a mut R,
        output_stream: &'a mut W,
        output_compression_type: CompressionType,
    ) -> Result<()> {
        debug!("starting new autocompression context: output={output_compression_type}");
        let mut context =
            Self::new_from_stream(input_stream, output_stream, output_compression_type)?;
        debug!(
            "detected compression type: {}",
            context.input_compression_type
        );
        context.translate_stream()
    }

    fn new_from_stream(
        mut input_stream: &'a mut R,
        output_stream: &'a mut W,
        output_compression_type: CompressionType,
    ) -> Result<Self> {
        let (kind, magic) = detect_stream_characteristics(&mut input_stream)?;

        Ok(Self {
            input_compression_type: kind,
            output_compression_type,
            input_stream,
            output_stream,
            magic,
        })
    }

    fn translate_stream(&mut self) -> Result<()> {
        if self.input_compression_type == self.output_compression_type {
            debug!("no translation necessary");
            let mut input_stream = &mut self.magic.chain(&mut self.input_stream);
            io::copy(&mut input_stream, &mut self.output_stream)?;
            return Ok(());
        }

        debug!("translating stream...");
        let mut input_stream = &mut self.magic.chain(&mut self.input_stream);
        let mut decompressor: Box<dyn Decompressor> = match self.input_compression_type {
            CompressionType::Zstd => {
                let decoder = zstd::Decoder::new(&mut input_stream)?;
                Box::new(ZstdDecompressor(decoder))
            }
            CompressionType::Brotli => {
                let decoder = brotli::Decompressor::new(&mut input_stream, 4096);
                Box::new(BrotliDecompressor(decoder))
            }
            CompressionType::Gzip => {
                let decoder = flate2::read::GzDecoder::new(&mut input_stream);
                Box::new(GzipDecompressor(decoder))
            }
            CompressionType::Deflate => {
                let decoder = flate2::read::DeflateDecoder::new(&mut input_stream);
                Box::new(DeflateDecompressor(decoder))
            }
            CompressionType::Zlib => {
                let decoder = flate2::read::ZlibDecoder::new(&mut input_stream);
                Box::new(ZlibDecompressor(decoder))
            }
            CompressionType::Xz => {
                let decoder = xz2::read::XzDecoder::new(&mut input_stream);
                Box::new(XzDecompressor(decoder))
            }
            CompressionType::None => {
                let decoder = &mut input_stream;
                Box::new(NoneDecompressor(decoder))
            }
        };
        debug!("built decompressor");

        let mut compressor: Box<dyn Compressor> = match self.output_compression_type {
            CompressionType::Zstd => {
                let encoder =
                    zstd::Encoder::new(&mut self.output_stream, ZSTD_LEVEL)?.auto_finish();
                Box::new(ZstdCompressor(encoder))
            }
            CompressionType::Brotli => {
                let encoder = brotli::CompressorWriter::new(
                    &mut self.output_stream,
                    BROTLI_BUFFER_SIZE,
                    BROTLI_Q,
                    BROTLI_LGWIN,
                );
                Box::new(BrotliCompressor(encoder))
            }
            CompressionType::Gzip => {
                let encoder = flate2::write::GzEncoder::new(
                    &mut self.output_stream,
                    flate2::Compression::default(),
                );
                Box::new(GzipCompressor(encoder))
            }
            CompressionType::Deflate => {
                let encoder = flate2::write::DeflateEncoder::new(
                    &mut self.output_stream,
                    flate2::Compression::default(),
                );
                Box::new(DeflateCompressor(encoder))
            }
            CompressionType::Zlib => {
                let encoder = flate2::write::ZlibEncoder::new(
                    &mut self.output_stream,
                    flate2::Compression::default(),
                );
                Box::new(ZlibCompressor(encoder))
            }
            CompressionType::Xz => {
                let encoder = xz2::write::XzEncoder::new(&mut self.output_stream, XZ_LEVEL);
                Box::new(XzCompressor(encoder))
            }
            CompressionType::None => {
                let encoder = &mut self.output_stream;
                Box::new(NoneCompressor(encoder))
            }
        };
        debug!("built compressor");

        io::copy(&mut decompressor, &mut compressor)?;

        debug!("stream translated!");

        Ok(())
    }
}

#[allow(unused)]
#[derive(Debug, PartialEq, Eq, Clone, Copy, strum::Display)]
pub enum CompressionType {
    None,
    Brotli,
    Deflate,
    Gzip,
    Xz,
    Zlib,
    Zstd,
    // Lzma,
}

// Compression //

trait Compressor: Write {
    fn compress(&mut self, stream: Box<dyn Read>) -> Result<()>;
}

struct ZstdCompressor<'a, T: Write>(zstd::stream::write::AutoFinishEncoder<'a, T>);

impl<T: Write> Write for ZstdCompressor<'_, T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for ZstdCompressor<'_, T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct BrotliCompressor<T: Write>(brotli::CompressorWriter<T>);

impl<T: Write> Write for BrotliCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for BrotliCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct GzipCompressor<T: Write>(flate2::write::GzEncoder<T>);

impl<T: Write> Write for GzipCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for GzipCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct DeflateCompressor<T: Write>(flate2::write::DeflateEncoder<T>);

impl<T: Write> Write for DeflateCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for DeflateCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct ZlibCompressor<T: Write>(flate2::write::ZlibEncoder<T>);

impl<T: Write> Write for ZlibCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for ZlibCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct XzCompressor<T: Write>(xz2::write::XzEncoder<T>);

impl<T: Write> Write for XzCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: Write> Compressor for XzCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

struct NoneCompressor<T: Write>(T);

impl<T: Write> Compressor for NoneCompressor<T> {
    fn compress(&mut self, mut stream: Box<dyn Read>) -> Result<()> {
        io::copy(&mut stream, &mut self.0)?;
        Ok(())
    }
}

impl<T: Write> Write for NoneCompressor<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// Decompression //

trait Decompressor: Read {
    fn decompress(&mut self, stream: Box<dyn Write>) -> Result<()>;
}

struct ZstdDecompressor<'a, T: BufRead>(zstd::Decoder<'a, T>);

impl<T: BufRead> Read for ZstdDecompressor<'_, T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: BufRead> Decompressor for ZstdDecompressor<'_, T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct BrotliDecompressor<T: Read>(brotli::Decompressor<T>);

impl<T: Read> Read for BrotliDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for BrotliDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct GzipDecompressor<T: Read>(flate2::read::GzDecoder<T>);

impl<T: Read> Read for GzipDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for GzipDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct DeflateDecompressor<T: Read>(flate2::read::DeflateDecoder<T>);

impl<T: Read> Read for DeflateDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for DeflateDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct ZlibDecompressor<T: Read>(flate2::read::ZlibDecoder<T>);

impl<T: Read> Read for ZlibDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for ZlibDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct XzDecompressor<T: Read>(xz2::read::XzDecoder<T>);

impl<T: Read> Read for XzDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for XzDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

struct NoneDecompressor<T: Read>(T);

impl<T: Read> Read for NoneDecompressor<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: Read> Decompressor for NoneDecompressor<T> {
    fn decompress(&mut self, mut stream: Box<dyn Write>) -> Result<()> {
        io::copy(&mut self.0, &mut stream)?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use super::*;
    use color_eyre::Result;

    #[test]
    fn test_none_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx =
            Context::new_from_stream(&mut input_stream, &mut output_stream, CompressionType::None)?;

        ctx.translate_stream()?;

        assert_eq!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_zstd_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx =
            Context::new_from_stream(&mut input_stream, &mut output_stream, CompressionType::Zstd)?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let mut encoder = zstd::Encoder::new(&mut compressed_stream, ZSTD_LEVEL)?.auto_finish();
            encoder.write_all(expected.as_bytes())?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_brotli_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx = Context::new_from_stream(
            &mut input_stream,
            &mut output_stream,
            CompressionType::Brotli,
        )?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let mut encoder = brotli::CompressorWriter::new(
                &mut compressed_stream,
                BROTLI_BUFFER_SIZE,
                BROTLI_Q,
                BROTLI_LGWIN,
            );
            encoder.write_all(expected.as_bytes())?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_gzip_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx =
            Context::new_from_stream(&mut input_stream, &mut output_stream, CompressionType::Gzip)?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(
                &mut compressed_stream,
                flate2::Compression::default(),
            );
            let mut compressor = GzipCompressor(encoder);
            compressor.compress(Box::new(expected.as_bytes()))?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_deflate_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx = Context::new_from_stream(
            &mut input_stream,
            &mut output_stream,
            CompressionType::Deflate,
        )?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let encoder = flate2::write::DeflateEncoder::new(
                &mut compressed_stream,
                flate2::Compression::default(),
            );
            let mut compressor = DeflateCompressor(encoder);
            compressor.compress(Box::new(expected.as_bytes()))?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_zlib_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx =
            Context::new_from_stream(&mut input_stream, &mut output_stream, CompressionType::Zlib)?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let encoder = flate2::write::ZlibEncoder::new(
                &mut compressed_stream,
                flate2::Compression::default(),
            );
            let mut compressor = ZlibCompressor(encoder);
            compressor.compress(Box::new(expected.as_bytes()))?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }

    #[test]
    fn test_xz_compression_works() -> Result<()> {
        let expected = "this is a test";
        let mut input_stream = expected.as_bytes();
        let mut output_stream: Vec<u8> = Vec::new();

        let mut ctx =
            Context::new_from_stream(&mut input_stream, &mut output_stream, CompressionType::Xz)?;

        ctx.translate_stream()?;

        let mut compressed_stream: Vec<u8> = Vec::new();
        {
            let mut encoder = xz2::write::XzEncoder::new(&mut compressed_stream, XZ_LEVEL);
            encoder.write_all(expected.as_bytes())?;
        }

        assert!(!compressed_stream.is_empty());
        assert_eq!(compressed_stream, output_stream);
        assert_ne!(expected.as_bytes(), output_stream);

        Ok(())
    }
}
