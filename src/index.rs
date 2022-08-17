use std::fs::File;
use std::io;
use std::io::Read;
use std::path::Path;
use byteorder::ReadBytesExt;
use byteorder::LE;
use flate2::read;

use crate::bundle::BundleFormat;

fn extract_index_(path: &Path) -> io::Result<Vec<(u64, u64, Option<u32>, Option<u32>)>> {
    let mut index = Vec::new();
    let mut def_buffer = vec![0; 0x80000];
    let mut inf_buffer = vec![0; 0x80000];
    let mut file = File::open(path)?;
    let read = file.read(&mut def_buffer)?;
    let mut read_chunks = 8;
    let mut chunks_read = 0;

    let format = (&def_buffer[..2]).read_u16::<LE>().unwrap();
    let format = match format {
        6 => BundleFormat::Six,
        5 => BundleFormat::Five,
        4 => BundleFormat::Four,
        i => panic!("unsupported bundle format {i}"),
    };

    let mut data = &def_buffer[12..read];
    let mut buffer = &mut inf_buffer[..];
    let mut first = true;
    while chunks_read < read_chunks {
        if data.len() < 4 {
            break;
        }
        let size = data.read_u32::<LE>()?;
        if size as usize > data.len() {
            break;
        }
        assert!(size < 0x10000);
        let copy;
        (copy, data) = data.split_at(size as usize);
        let mut e = read::ZlibDecoder::new(copy);

        let dest;
        (dest, buffer) = buffer.split_at_mut(0x10000);
        e.read_exact(dest)?;

        // check for fastpath
        if first {
            let num_files = (&dest[..4]).read_u32::<LE>()?;
            let entry_index_size = match format {
                BundleFormat::Six => 24,
                BundleFormat::Five => 20,
                BundleFormat::Four => 16,
            };
            let inf_size = 260 + num_files * entry_index_size;
            read_chunks = inf_size / 0x10000 + (inf_size % 0x10000 > 0).then(|| 1).unwrap_or(0);
            assert!(read_chunks <= 8);
            first = false;
        }
        chunks_read += 1;
    }

    let num_files = (&inf_buffer[..4]).read_u32::<LE>()?;
    let (_, mut buffer) = inf_buffer.split_at(260);
    for _ in 0..num_files {
        let ext_hash = buffer.read_u64::<LE>()?;
        let name_hash = buffer.read_u64::<LE>()?;
        let (flags, size) = match format {
            BundleFormat::Six => (Some(buffer.read_u32::<LE>()?), Some(buffer.read_u32::<LE>()?)),
            BundleFormat::Five => (Some(buffer.read_u32::<LE>()?), None),
            BundleFormat::Four => (None, None),
        };
        index.push((
            ext_hash,
            name_hash,
            flags,
            size,
        ));
    }

    Ok(index)
}

pub fn extract_index<P: AsRef<Path>>(path: P) -> io::Result<Vec<(u64, u64, Option<u32>, Option<u32>)>> {
    extract_index_(path.as_ref())
}
