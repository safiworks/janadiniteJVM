pub use crate::error::*;

extern crate alloc;
use alloc::vec::Vec;

pub trait ClassReader {
    fn read_next(&mut self, buf: &mut [u8]) -> Result<usize>;
    #[inline(always)]
    fn read_next_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        match self.read_next(buf) {
            Ok(am) if am < buf.len() => Err(ErrorKind::UnexpectedEof.into()),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.read_next_exact(&mut buf)?;
        Ok(buf[0])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let mut buf = [0u8; 2];
        self.read_next_exact(&mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        self.read_next_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.read_next_exact(&mut buf)?;
        Ok(u64::from_be_bytes(buf))
    }

    #[inline(always)]
    fn decode<T: ClassDecode>(&mut self) -> Result<T> {
        T::decode_from(self)
    }

    fn decode_n<T: ClassDecode>(&mut self, n: usize) -> Result<Vec<T>> {
        let mut results = Vec::with_capacity(n);
        for _ in 0..n {
            results.push(self.decode()?);
        }

        Ok(results)
    }
}

pub trait ClassWriter {
    fn write_next(&mut self, buf: &[u8]) -> Result<usize>;
    #[inline(always)]
    fn write_next_exact(&mut self, buf: &[u8]) -> Result<()> {
        match self.write_next(buf) {
            Ok(am) if am < buf.len() => Err(ErrorKind::UnexpectedEof.into()),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn write_u8(&mut self, value: u8) -> Result<()> {
        let buf = value.to_be_bytes();
        self.write_next_exact(&buf)?;
        Ok(())
    }

    fn write_u16(&mut self, value: u16) -> Result<()> {
        let buf = value.to_be_bytes();
        self.write_next_exact(&buf)?;
        Ok(())
    }

    fn write_u32(&mut self, value: u32) -> Result<()> {
        let buf = value.to_be_bytes();
        self.write_next_exact(&buf)?;
        Ok(())
    }

    fn write_u64(&mut self, value: u64) -> Result<()> {
        let buf = value.to_be_bytes();
        self.write_next_exact(&buf)?;
        Ok(())
    }

    #[inline(always)]
    fn encode<T: ClassEncode>(&mut self, t: &T) -> Result<()> {
        T::encode_into(t, self)
    }

    fn encode_items<T: ClassEncode>(&mut self, items: &[T]) -> Result<()> {
        for item in items {
            self.encode(item)?;
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
mod _std_impl {
    use crate::io::{ClassReader, ClassWriter};

    impl<T: std::io::Read> ClassReader for T {
        fn read_next(&mut self, buf: &mut [u8]) -> super::Result<usize> {
            Ok(self.read(buf)?)
        }
        fn read_next_exact(&mut self, buf: &mut [u8]) -> super::Result<()> {
            Ok(self.read_exact(buf)?)
        }
    }

    impl<T: std::io::Write> ClassWriter for T {
        fn write_next(&mut self, buf: &[u8]) -> super::Result<usize> {
            Ok(self.write(buf)?)
        }
        fn write_next_exact(&mut self, buf: &[u8]) -> super::Result<()> {
            Ok(self.write_all(buf)?)
        }
    }
}

/// Provides a method to decode class bytes into an in-memory rust type.
///
/// TODO: repr??
///
/// This shouldn't be used directly instead a higher level implementation should provide class building and decoding.
pub trait ClassDecode: Sized {
    fn decode_from<R: ClassReader + ?Sized>(reader: &mut R) -> Result<Self>;
}

/// Provides a method to encode class bytes into an in-memory rust type.
///
/// TODO: repr??
///
/// This shouldn't be used directly instead a higher level implementation should provide class building and decoding.
pub trait ClassEncode: Sized {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> Result<()>;
}

impl ClassDecode for u8 {
    fn decode_from<R: ClassReader + ?Sized>(reader: &mut R) -> Result<Self> {
        reader.read_u8()
    }
}

impl ClassEncode for u8 {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> Result<()> {
        writer.write_u8(*self)
    }
}

impl ClassDecode for u16 {
    fn decode_from<R: ClassReader + ?Sized>(reader: &mut R) -> Result<Self> {
        reader.read_u16()
    }
}

impl ClassEncode for u16 {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> Result<()> {
        writer.write_u16(*self)
    }
}

impl ClassDecode for f32 {
    fn decode_from<R: ClassReader + ?Sized>(reader: &mut R) -> Result<Self> {
        reader.read_u32().map(|f| f32::from_bits(f))
    }
}

impl ClassEncode for f32 {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> Result<()> {
        let bits = if self.is_nan() {
            u32::MAX
        } else {
            self.to_bits()
        };

        writer.write_u32(bits)
    }
}

impl ClassDecode for f64 {
    fn decode_from<R: ClassReader + ?Sized>(reader: &mut R) -> Result<Self> {
        reader.read_u64().map(|bits| f64::from_bits(bits))
    }
}

impl ClassEncode for f64 {
    fn encode_into<W: ClassWriter + ?Sized>(&self, writer: &mut W) -> Result<()> {
        let bits = if self.is_nan() {
            u64::MAX
        } else {
            self.to_bits()
        };

        writer.write_u64(bits)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClassByteReader<'a> {
    bytes: &'a [u8],
}

impl<'a> ClassByteReader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl<'a> ClassReader for ClassByteReader<'a> {
    fn read_next(&mut self, buf: &mut [u8]) -> Result<usize> {
        let amount = buf.len().min(self.bytes.len());

        let (data, rest) = self.bytes.split_at(amount);
        buf[..amount].copy_from_slice(data);

        self.bytes = rest;
        Ok(amount)
    }
}
