use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::io;
use std::io::Cursor;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
use byteorder::LE;
use flate2::read;
use flate2::write;
use flate2::Compression;
use crate::hash;

mod pack;
pub use pack::pack_dir_to_bundle;
pub use pack::unpack_bundle_to_dir;
pub use pack::Merge;

#[derive(Clone)]
pub struct PackSettings {
    pub num_threads: usize,
}

pub trait IBundleUnpacker {
    fn bundle_reader(&mut self) -> io::Result<(&mut dyn Read, usize)>;

    // file: (name_hash, ext_hash)
    fn write_file(&mut self, file: (u64, u64), data: &[u8]) -> io::Result<()>;
    fn write_header(&mut self, data: &[u8]) -> io::Result<()>;

    fn unpack(&mut self, settings: &PackSettings) -> io::Result<()>
    where
        Self: Sized,
    {
        unpack_bundle(&mut *self, settings)
    }
}

pub trait IBundlePacker {
    fn bundle_writer(&mut self) -> io::Result<&mut (dyn Write + '_)>;

    // file: (name_hash, ext_hash)
    fn files(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<(u64, u64)>> + '_>>;
    fn read_file(&self, file: (u64, u64)) -> io::Result<Cow<[u8]>>;
    fn read_header(&self) -> io::Result<Cow<[u8]>>;

    fn repack(&mut self, settings: &PackSettings) -> io::Result<()>
    where
        Self: Sized,
    {
        repack_bundle(&mut *self, settings)
    }
}

#[derive(PartialEq)]
pub enum BundleFormat {
    // VT2
    Six,
    // VT2 mods, VT1 patch bundles (all patch_xxx and new bundles added in patches)
    Five,
    // VT1 original bundles
    Four,
}

fn unpack_bundle(unpack: &mut dyn IBundleUnpacker, settings: &PackSettings) -> io::Result<()> {
    let num_threads = settings.num_threads.min(1);
    let (reader, size) = unpack.bundle_reader().unwrap();
    assert!(size > 16);

    let mut header = [0; 12];
    reader.read_exact(&mut header).unwrap();
    let mut header = &header[..];
    let version = header.read_u16::<LE>().unwrap();
    // check if supported bundle version
    let format = match version {
        6 => BundleFormat::Six,
        5 => BundleFormat::Five,
        4 => BundleFormat::Four,
        f => unimplemented!("unsupported bundle format {f}"),
    };

    let _unknown = header.read_u16::<LE>().unwrap();
    let uncompressed_size = header.read_u32::<LE>().unwrap() as usize;
    let mut inf_buffer = vec![0; uncompressed_size];
    let mut def_buffer = vec![0; size];
    let mut inf_chunks = inf_buffer.chunks_mut(0x10000);

    let queue = Mutex::new(Vec::<(&[u8], &mut [u8])>::with_capacity(4096));
    let running = AtomicBool::new(true);
    thread::scope(|s| {
        let upper = usize::try_from(uncompressed_size).unwrap() / (16 * 1024 * 1024);
        for _ in 0..num_threads.min(upper).min(1) {
            s.spawn(|| {
                'pull: while let Some((in_chunk, buffer)) = {
                    let mut queue = queue.lock().unwrap();
                    if queue.len() == 0 {
                        drop(queue);
                        if running.load(Ordering::SeqCst) {
                            continue 'pull;
                        } else {
                            None
                        }
                    } else {
                        queue.pop()
                    }
                } {
                    if in_chunk.len() < 0x10004 {
                        let mut e = read::ZlibDecoder::new(&in_chunk[4..]);
                        e.read_exact(buffer).unwrap();
                    } else {
                        assert_eq!(in_chunk.len(), 0x10004);
                        buffer.copy_from_slice(&in_chunk[4..]);
                    }
                }
            });
        }

        let mut def_buffer = &mut def_buffer[12..];
        let mut buffer_offset = 0;
        let mut offset = 0;

        let mut current_chunk = None;
        while let Ok(read) = reader.read(&mut def_buffer[buffer_offset..]) {
            if read == 0 {
                break;
            }

            buffer_offset += read;
            loop {
                if let Some(chunk_size) = current_chunk {
                    if buffer_offset - offset >= chunk_size + 4 {
                        let advance = chunk_size + 4;
                        let in_chunk;
                        (in_chunk, def_buffer) = def_buffer.split_at_mut(advance);
                        let out_chunk = inf_chunks.next().unwrap();
                        {
                            let mut queue = queue.lock().unwrap();
                            queue.push((in_chunk, out_chunk));
                        }
                        offset = 0;
                        buffer_offset -= advance;
                        current_chunk = None;
                    } else {
                        break;
                    }
                } else if buffer_offset > offset + 4 {
                    current_chunk = Some(
                        usize::try_from(u32::from_le_bytes(
                            def_buffer[offset..offset + 4].try_into().unwrap()
                        )).unwrap()
                    );
                    assert!(current_chunk <= Some(0x10000), "{current_chunk:x?}");
                } else {
                    break;
                }
            }
        }
        running.store(false, Ordering::SeqCst);
    });

    std::fs::write(r"C:\modding\bundle.test_raw", &inf_buffer).unwrap();

    let mut rdr = Cursor::new(&inf_buffer);
    let num_files = rdr.read_u32::<LE>().unwrap() as usize;
    unpack.write_header(&inf_buffer[4..260]).unwrap();
    rdr.seek(SeekFrom::Current(256)).unwrap();

    let index_entry_size = match format {
        BundleFormat::Six => 24,
        BundleFormat::Five => 20,
        BundleFormat::Four => 16,
    };
    let mut offset = 260 + num_files * index_entry_size;
    let mut buffer = Vec::with_capacity(0x20000);
    for _ in 0..num_files {
        buffer.clear();
        let mut patch_size = false;

        let ext_hash = rdr.read_u64::<LE>().unwrap();
        let name_hash = rdr.read_u64::<LE>().unwrap();

        // file_size is sometimes unreliable in BundleFormat::Six since it only
        // has the size for the first localization variant.
        let (flags, file_size) = match format {
            BundleFormat::Six => (rdr.read_u32::<LE>().unwrap(), rdr.read_u32::<LE>().unwrap()),
            BundleFormat::Five => {
                patch_size = true;
                (rdr.read_u32::<LE>().unwrap(), 0)
            }
            BundleFormat::Four => {
                patch_size = true;
                (0, 0)
            }
        };


        buffer.write_u64::<LE>(ext_hash).unwrap();
        buffer.write_u64::<LE>(name_hash).unwrap();
        buffer.write_u32::<LE>(flags).unwrap();
        buffer.write_u32::<LE>(file_size).unwrap();

        let current = rdr.position();
        rdr.set_position(offset as u64);
        assert_eq!(ext_hash, rdr.read_u64::<LE>().unwrap(), "{offset}");
        assert_eq!(name_hash, rdr.read_u64::<LE>().unwrap());
        let num_localizations = rdr.read_u32::<LE>().unwrap();
        let _unknown = rdr.read_u32::<LE>().unwrap();
        let mut size = 24;
        for _ in 0..num_localizations {
            let _unknown = rdr.read_u32::<LE>().unwrap();
            let local_len = rdr.read_u32::<LE>().unwrap();
            let _unknown = rdr.read_u32::<LE>().unwrap();
            if patch_size {
                buffer[20..24].copy_from_slice(&local_len.to_le_bytes());
            }
            size += 12 + local_len as usize;
        }

        buffer.extend(&inf_buffer[offset..offset + size]);
        offset += size;
        rdr.set_position(current);
        unpack.write_file((name_hash, ext_hash), &buffer).unwrap();
    }

    Ok(())
}

fn repack_bundle(pack: &mut dyn IBundlePacker, settings: &PackSettings) -> io::Result<()> {
    let num_threads = settings.num_threads;
    let mut files = Vec::new();
    let mut total_size = 0;
    let header = pack.read_header()
        .unwrap_or(Cow::Borrowed(&[0; 256]))
        .to_vec();

    for file in pack.files().unwrap() {
        let (name_hash, ext_hash) = file?;
        files.push((
            name_hash,
            ext_hash,
        ));
    }

    files.sort_unstable_by(|(a_name, a_ext), (b_name, b_ext)|
        (*a_ext as u32).cmp(&(*b_ext as u32))
            .then(a_name.cmp(&b_name))
    );
    let num_files = files.len();

    let finished = AtomicBool::new(false);
    let queue = Mutex::new(Vec::<(usize, Vec<u8>)>::with_capacity(num_files * 4));
    let (tx, rx) = mpsc::channel();
    thread::scope(|s| {
        for _ in 0..num_threads {
            let tx = tx.clone();
            s.spawn(|| {
                let tx = tx;
                let mut def_buffer = None;
                'queue_loop: while let Some((chunk_index, buffer)) = {
                    let mut queue = queue.lock().unwrap();
                    if queue.len() > 0 || finished.load(Ordering::SeqCst) {
                        queue.pop()
                    } else {
                        drop(queue);
                        thread::sleep(Duration::from_millis(5));
                        continue 'queue_loop;
                    }
                } {
                    assert_eq!(buffer.len(), 0x10000);
                    let mut e = write::ZlibEncoder::new(def_buffer.take().unwrap_or(Vec::new()), Compression::default());
                    e.write_all(&buffer).unwrap();
                    let mut out = e.finish().unwrap();
                    if out.len() >= 0x10000 {
                        out.clear();
                        def_buffer = Some(out);
                        tx.send((chunk_index, buffer)).unwrap();
                    } else {
                        tx.send((chunk_index, out)).unwrap();
                    }
                }
            });
        }
        drop(tx);

        let entry_size = 260 + num_files * 24;
        total_size += entry_size;
        let first_offset = if entry_size % 0x10000 == 0 {
            0
        } else {
            0x10000 - entry_size % 0x10000
        };
        let mut entry = Vec::with_capacity(entry_size + first_offset);
        let mut entry_tail = Vec::with_capacity(first_offset);

        entry.write_u32::<LE>(u32::try_from(num_files).unwrap()).unwrap();
        entry.extend(&*header);

        //let mut test_raw = Vec::new();

        let mut removed_files = Some(Vec::new());
        let mut chunk = 0;
        let mut first = Some(&mut entry_tail);
        let mut buffer = Vec::with_capacity(0x10000);
        let mut count = 0;
        let mut iter: Box<dyn Iterator<Item = (u64, u64, Cow<[u8]>)>> =
            Box::new(files.iter().map(|f| (f.0, f.1, pack.read_file((f.0, f.1)).unwrap())));
        while let Some((name_hash, ext_hash, data)) = iter.next() {
            total_size += data.len() - 24;
            count += 1;

            // check for deleted file (0x01) in packed header and sort to end
            let flag = (&data[16..20]).read_u32::<LE>().unwrap();
            if (flag == 0x01 || flag == 0x02)
                && let Some(ref mut removed) = removed_files
            {
                removed.push((name_hash, ext_hash, flag, data));
                if count == files.len() {
                    let mut removed = removed_files.take().unwrap();
                    removed.sort_by(|(a_name, a_ext, a_flag, _), (b_name, b_ext, b_flag, _)|
                        a_flag.cmp(&b_flag)
                            .then(a_ext.cmp(&b_ext))
                            .then(a_name.cmp(&b_name))
                    );
                    iter = Box::new(removed.into_iter().map(|(a, b, _, c)| (a, b, c)));
                }
                continue;
            }
            entry.extend(&data[..24]);

            if count == files.len() && removed_files.is_some() {
                let mut removed = removed_files.take().unwrap();
                removed.sort_by(|(a_name, a_ext, a_flag, _), (b_name, b_ext, b_flag, _)|
                    a_flag.cmp(&b_flag)
                        .then(a_ext.cmp(&b_ext))
                        .then(a_name.cmp(&b_name))
                );
                iter = Box::new(removed.into_iter().map(|(a, b, _, c)| (a, b, c)));
            }

            // ignore file's index header packed at start
            let mut data = &data[24..];
            if let Some(ref mut buffer) = first {
                while data.len() > 0 {
                    let read = data.len().min(buffer.capacity() - buffer.len());
                    let copy;
                    (copy, data) = data.split_at(read);
                    buffer.extend(copy);
                    if buffer.len() == buffer.capacity() {
                        first = None;
                        break;
                    }
                }
            }

            while data.len() > 0 {
                let read = data.len().min(buffer.capacity() - buffer.len());
                let copy;
                (copy, data) = data.split_at(read);
                buffer.extend(copy);
                if buffer.len() == buffer.capacity() {
                    //test_raw.extend(&buffer);
                    {
                        let mut queue = queue.lock().unwrap();
                        queue.insert(0, (chunk, buffer));
                    }
                    buffer = Vec::with_capacity(0x10000);
                    chunk += 1;
                }
                assert!(buffer.len() < buffer.capacity());
            }
        }
        drop(iter);

        if buffer.len() > 0 {
            assert_eq!(buffer.capacity(), 0x10000);
            buffer.resize(buffer.capacity(), 0);
            //test_raw.extend(&buffer);
            {
                let mut queue = queue.lock().unwrap();
                queue.insert(0, (chunk, buffer));
            }
        }

        finished.store(true, Ordering::SeqCst);

        entry.extend(&entry_tail);
        if entry.len() % 0x10000 != 0 {
            entry.resize(entry.len() + (0x10000 - entry.len() % 0x10000), 0);
        }

        //let _: Vec<u8> = test_raw;
        //let mut test_file = File::create(r"C:\modding\bundle.test").unwrap();
        //test_file.write_all(&entry).unwrap();
        //test_file.write_all(&test_raw).unwrap();

        let writer = pack.bundle_writer().unwrap();
        writer.write_u16::<LE>(6).unwrap();
        writer.write_u16::<LE>(u16::swap_bytes(0x00f0)).unwrap();
        writer.write_u32::<LE>(u32::try_from(total_size).unwrap()).unwrap();
        writer.write_u32::<LE>(0).unwrap();

        let mut def_buffer = Vec::with_capacity(0x20000);
        for chunk in entry.chunks(0x10000) {
            def_buffer.clear();
            let mut e = write::ZlibEncoder::new(&mut def_buffer, Compression::default());
            e.write_all(chunk).unwrap();
            let buffer = e.finish().unwrap();
            if buffer.len() >= 0x10000 {
                writer.write_u32::<LE>(0x10000).unwrap();
                writer.write_all(chunk).unwrap();
            } else {
                writer.write_u32::<LE>(u32::try_from(buffer.len()).unwrap()).unwrap();
                writer.write_all(buffer).unwrap();
            }
        }

        let mut list = BTreeMap::new();
        let mut next = 0;
        for (mut chunk_index, mut chunk) in rx.iter() {
            assert!(chunk.len() <= 0x10000);
            if next == chunk_index  {
                while next == chunk_index {
                    writer.write_u32::<LE>(u32::try_from(chunk.len()).unwrap()).unwrap();
                    writer.write_all(&chunk).unwrap();
                    next += 1;
                    if let Some(next_chunk) = list.remove(&next) {
                        chunk = next_chunk;
                        chunk_index = next;
                    }
                }
            } else {
                list.insert(chunk_index, chunk);
            }
        }
        assert_eq!(list.iter().count(), 0);
    });

    Ok(())
}