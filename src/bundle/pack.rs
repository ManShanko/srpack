use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use super::*;

const HEADER_FILE: &'static str = "_HEADER";

pub struct Merge {
    settings: PackSettings,
    pub header: Option<[u8; 256]>,
    pub files: HashMap<(u64, u64), Vec<u8>>,
}

impl Merge {
    pub fn new(settings: &PackSettings) -> Self {
        Self {
            settings: settings.clone(),
            header: None,
            files: HashMap::new(),
        }
    }

    fn unpack_from_(&mut self, bundle: &Path) -> io::Result<()> {
        let target = File::open(bundle)?;
        let mut work = MemoryUnpack {
            target,
            header: [0; 256],
            files: HashMap::new(),
        };
        work.unpack(&self.settings)?;
        let files = mem::take(&mut self.files);

        let f_cap = files.capacity();
        let w_cap = work.files.capacity();
        let len = if f_cap > w_cap * 2 {
            f_cap
        } else {
            w_cap
        };

        self.files = HashMap::with_capacity(len);
        for (hash, data) in files.into_iter().chain(work.files.into_iter()) {
            self.files.insert(hash, data);
        }

        if self.header.is_none() {
            self.header = Some(work.header);
        } else if self.header != Some(work.header) {
            self.header = Some([0; 256]);
        }

        Ok(())
    }

    pub fn unpack_from<P: AsRef<Path>>(&mut self, bundle: P) -> io::Result<()> {
        self.unpack_from_(bundle.as_ref())
    }

    fn repack_to_(self, bundle: &Path) -> io::Result<()> {
        let mut target = File::create(bundle)?;
        self.repack_to_write(&mut target)?;
        Ok(())
    }
    pub fn repack_to<P: AsRef<Path>>(self, bundle: P) -> io::Result<()> {
        self.repack_to_(bundle.as_ref())
    }

    pub fn repack_to_write(self, target: &mut dyn Write) -> io::Result<()> {
        let mut repack = MemoryRepack {
            target,
            header: self.header.unwrap_or([0; 256]),
            files: self.files,
        };
        repack.repack(&self.settings)?;
        Ok(())
    }
}
struct MemoryUnpack {
    target: File,
    header: [u8; 256],
    files: HashMap<(u64, u64), Vec<u8>>,
}

impl IBundleUnpacker for MemoryUnpack {
    fn bundle_reader(&mut self) -> io::Result<(&mut dyn Read, usize)> {
        let size = self.target.metadata()?.len();
        Ok((&mut self.target, usize::try_from(size).unwrap()))
    }

    fn write_file(&mut self, file: (u64, u64), data: &[u8]) -> io::Result<()> {
        debug_assert!(!self.files.contains_key(&file));
        self.files.insert(file, data.to_vec());
        Ok(())
    }

    fn write_header(&mut self, data: &[u8]) -> io::Result<()> {
        self.header.copy_from_slice(data);
        Ok(())
    }
}

struct MemoryRepack<'a> {
    target: &'a mut dyn Write,
    header: [u8; 256],
    // profile sort performance with either (name_hash, ext_hash) or (ext_hash, name_hash)
    files: HashMap<(u64, u64), Vec<u8>>,
}

impl<'a> IBundlePacker for MemoryRepack<'a> {
    fn bundle_writer(&mut self) -> io::Result<&mut (dyn Write + '_)> {
        Ok(&mut self.target)
    }

    fn files(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<(u64, u64)>> + '_>> {
        Ok(Box::new(self.files.iter().map(|(hash, _)| Ok(*hash))))
    }

    fn read_file(&self, file: (u64, u64)) -> io::Result<Cow<[u8]>> {
        self.files
            .get(&file)
            .map(|data| Cow::Borrowed(&data[..]))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not load file data"))
    }

    fn read_header(&self) -> io::Result<Cow<[u8]>> {
        Ok(Cow::Borrowed(&self.header[..]))
    }
}

struct Unpack {
    bundle: File,
    dir: PathBuf,
}

impl IBundleUnpacker for Unpack {
    fn bundle_reader(&mut self) -> io::Result<(&mut dyn Read, usize)> {
        let size = self.bundle.metadata()?.len();
        Ok((&mut self.bundle, usize::try_from(size).unwrap()))
    }

    fn write_file(&mut self, file: (u64, u64), data: &[u8]) -> io::Result<()> {
        match hash::extension_lookup(file.1) {
            Some(ext) => self.dir.push(format!("{:016x}.{}", file.0, ext)),
            None => panic!("unknown extension hash {:016x}", file.1),
        }
        fs::write(&self.dir, data)?;
        self.dir.pop();
        Ok(())
    }

    fn write_header(&mut self, data: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        self.dir.push(HEADER_FILE);
        fs::write(&self.dir, data)?;
        self.dir.pop();
        Ok(())
    }
}

struct Repack {
    bundle: File,
    dir: PathBuf,
}

impl IBundlePacker for Repack {
    fn bundle_writer(&mut self) -> io::Result<&mut (dyn Write + '_)> {
        Ok(&mut self.bundle)
    }

    fn files(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<(u64, u64)>> + '_>> {
        let mut out = Vec::new();
        let files = fs::read_dir(&self.dir)?;
        for file in files {
            let file = file?;
            let metadata = file.metadata()?;
            if metadata.is_file() {
                let path = file.path();
                if let Some(stem) = path.file_stem()
                    && let Some(stem) = stem.to_str()
                    && stem.len() == 16
                    && let Some(ext) = path.extension()
                    && let Some(ext) = ext.to_str()
                {
                    let name_hash = u64::from_str_radix(&stem, 16).unwrap();
                    let ext_hash = hash::stingray_hash64(ext.as_bytes());
                    out.push((name_hash, ext_hash));
                }
            }
        }
        Ok(Box::new(out.into_iter().map(|hash| Ok(hash))))
    }

    fn read_file(&self, file: (u64, u64)) -> io::Result<Cow<[u8]>> {
        let file = self.dir.join(&format!(
            "{:016x}.{}",
            file.0,
            hash::extension_lookup(file.1).unwrap()
        ));
        let data = fs::read(&file)?;
        Ok(Cow::Owned(data))
    }

    fn read_header(&self) -> io::Result<Cow<[u8]>> {
        let file = self.dir.join(HEADER_FILE);
        let data = fs::read(&file)?;
        Ok(Cow::Owned(data))
    }
}

pub fn unpack_bundle_to_dir<B: AsRef<Path>, D: AsRef<Path>>(
    bundle: B,
    dir: D,
    settings: &PackSettings,
) -> io::Result<()> {
    let bundle = bundle.as_ref();
    let dir = dir.as_ref();
    assert!(bundle.exists());
    let bundle = File::open(bundle)?;
    let mut unpack = Unpack {
        bundle,
        dir: dir.to_path_buf(),
    };
    unpack.unpack(settings)
}

pub fn pack_dir_to_bundle<D: AsRef<Path>, B: AsRef<Path>>(
    dir: D,
    bundle: B,
    settings: &PackSettings,
) -> io::Result<()> {
    let dir = dir.as_ref();
    let bundle = bundle.as_ref();
    assert!(dir.exists());
    let bundle = File::create(bundle)?;
    let mut pack = Repack {
        dir: dir.to_path_buf(),
        bundle,
    };
    pack.repack(settings)
}













