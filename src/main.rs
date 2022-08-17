#![allow(dead_code)]
use std::collections::HashMap;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::fs;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::mem;
use std::path::Path;

#[macro_use]
mod cli;
use cli::Switch;
use cli::CommandBuilder;
mod hash;
use hash::MurmurHash64;
mod bundle;
mod index;

const PADDING: &str = "                                ";

const NUM_THREADS: Switch = Switch::short("t", "threads")
    .with_params(&["N"])
    .with_desc("Number of threads to use.");

const DICTIONARY: Switch = Switch::short("d", "dictionary")
    .with_params(&["FILE"])
    .with_desc("Dictionary to reverse lookup hashes.");

const SKIP_HASH: Switch = Switch::new("skip-hash")
    .with_desc("Don't print files that have an unknown name hash.");

const MERGE: CommandBuilder = command![
        NUM_THREADS,
    ].with_name("merge")
    .with_params(&["slow"]);

const UNPACK: CommandBuilder = command![
        NUM_THREADS,
    ].with_name("unpack")
    .with_short_desc("Unpack bundle into directory.")
    .with_params(&["bundle", "dir"]);

const REPACK: CommandBuilder = command![
        NUM_THREADS,
    ].with_name("repack")
    .with_short_desc("Pack files in directory into bundle.")
    .with_params(&["dir", "bundle"]);

const INDEX: CommandBuilder = command![
        DICTIONARY,
    ].with_name("index")
    .with_short_desc("List index of bundle.")
    .with_params(&["bundle"]);

const SCAN: CommandBuilder = command![
        NUM_THREADS,
        DICTIONARY,
        SKIP_HASH,
    ].with_name("scan")
    .with_short_desc("Index all bundles in directory.")
    .with_params(&["directory"]);

const HASH: CommandBuilder = command![]
    .with_name("hash")
    .with_short_desc("MurmurHash string.")
    .with_params(&["string"]);

const TEST: CommandBuilder = command![]
    .with_name("test")
    .with_short_desc("Test unpack and repack on bundle.")
    .with_params(&["bundle"]);

fn main() {
    let app = app![
        //MERGE,
        UNPACK,
        REPACK,
        INDEX,
        SCAN,
        HASH,
        TEST,
    ];

    if let Ok(Some(app)) = app {
        let num_threads = if let Some(mut params) = app.switch_params(NUM_THREADS) {
            params.next()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse::<usize>().ok()).expect("failed to parse NUM_THREADS")
        } else {
            thread::available_parallelism().ok().map(|i| i.get()).unwrap_or(1)
        }.max(1);

        let dictionary = app.switch_params(DICTIONARY)
            .and_then(|mut params| params.next())
            .map(|d| {
                let dict = fs::read(d).unwrap();
                let lines = BufReader::new(&*dict).lines();
                let mut lookup = HashMap::new();
                for line in lines {
                    let line = line.unwrap();
                    lookup.insert(MurmurHash64::new(&line), line);
                }
                lookup
            });

        let settings = bundle::PackSettings {
            num_threads,
        };

        let mut params = app.params();
        if app.subcmd(&MERGE) {
        } else if app.subcmd(&UNPACK) {
            let bundle = params.next().expect("failed to parse parameter bundle");
            let dir = params.next().expect("failed to parse parameter directory");
            bundle::unpack_bundle_to_dir(bundle, dir, &settings).unwrap();
        } else if app.subcmd(&REPACK) {
            let dir = params.next().expect("failed to parse parameter directory");
            let bundle = params.next().expect("failed to parse parameter bundle");
            bundle::pack_dir_to_bundle(dir, bundle, &settings).unwrap();
        } else if app.subcmd(&INDEX) {
            let bundle = params.next().expect("failed to parse parameter bundle");
            let index = index::extract_index(bundle).unwrap();
            if !index.is_empty() {
                let mut longest = 0;
                for (ext_hash, ..) in index.iter() {
                    if let Some(ext) = hash::extension_lookup(*ext_hash) {
                        longest = longest.max(ext.len());
                    }
                }

                let padding = longest.saturating_sub(16);
                println!();
                println!(" flags        size {}    extension            name", &PADDING[..padding]);
                for (ext_hash, name_hash, flags, size) in index.iter() {
                    if let Some(flags) = flags {
                        print!("   {flags}  ");
                    } else {
                        print!("  N/A ");
                    }

                    if let Some(size) = size {
                        print!("  {size:>10} ");
                    } else {
                        print!("         N/A ");
                    }

                    if let Some(ext) = hash::extension_lookup(*ext_hash) {
                        print!(" {}{ext:^16} ",
                            &PADDING[..longest.saturating_sub(ext.len()).saturating_sub(16)]);
                    } else {
                        eprintln!("unknown extension hash 0x{ext_hash:016x}");
                        print!(" {}{ext_hash:16x} ",
                            &PADDING[..padding]);
                    };

                    if let Some(dict) = &dictionary
                        && let Some(name) = dict.get(&MurmurHash64::from_u64(*name_hash))
                    {
                        print!(" {name:^16}");
                    } else {
                        print!(" {name_hash:016x}");
                    }

                    println!();
                }
            }
        } else if app.subcmd(&SCAN) {
            let dir = params.next().expect("failed to parse parameter bundle");
            let skip_hashes = app.switch_active(&SKIP_HASH);
            let mut files = fs::read_dir(dir).unwrap()
                .filter_map(|entry| {
                    if let Ok(entry) = entry
                        && let Ok(meta) = entry.metadata()
                        && meta.is_file()
                        && let path = entry.path()
                        && let Some(stem) = path.file_stem()
                        && let Some(stem) = stem.to_str()
                        && stem.len() == 16
                        && 9 == path.extension().map(|ext| ext.len()).unwrap_or(9)
                    {
                        Some(path)
                    } else {
                        None
                    }
                }).collect::<Vec<_>>();
            files.sort_unstable();

            let (tx, rx) = mpsc::channel();
            let job = AtomicUsize::new(0);
            thread::scope(|s| {
                for _ in 0..num_threads {
                    let tx = tx.clone();
                    s.spawn(|| {
                        let tx = tx;
                        let mut buffer = Vec::with_capacity(0x10000);
                        let mut i = job.fetch_add(1, Ordering::SeqCst);
                        while let Some(path) = files.get(i) {
                            let bundle = path.file_name().unwrap().to_str().unwrap();
                            let index = index::extract_index(&path).unwrap();
                            for (ext_hash, name_hash, ..) in index.iter() {
                                let ext = hash::extension_lookup(*ext_hash).unwrap();
                                if let Some(dict) = &dictionary
                                    && let Some(name) = dict.get(&MurmurHash64::from_u64(*name_hash))
                                {
                                    writeln!(buffer, "{bundle:<26}   {name}.{ext}").unwrap();
                                } else if !skip_hashes {
                                    writeln!(buffer, "{bundle:<26}   {name_hash:016x}.{ext}").unwrap();
                                }
                            }
                            tx.send((i, mem::take(&mut buffer))).unwrap();
                            i = job.fetch_add(1, Ordering::SeqCst);
                        }
                    });

                }
                drop(tx);

                let mut count = 0;
                let mut list = BTreeMap::new();
                for (mut i, mut buffer) in rx.iter() {
                    if i == count {
                        while i == count {
                            io::stdout().write_all(&buffer).unwrap();
                            count += 1;
                            if let Some(entry) = list.remove(&count) {
                                i = count;
                                buffer = entry;
                            }
                        }
                    } else {
                        list.insert(i, buffer);
                    }
                }
            });
        } else if app.subcmd(&HASH) {
            let string = params.next().expect("failed to parse parameter bundle").to_string_lossy();
            println!("{:16x}", hash::stingray_hash64(string.as_bytes()));
        } else if app.subcmd(&TEST) {
            let bundle = params.next().expect("failed to parse parameter bundle");
            println!("testing \"{}\"", Path::new(bundle).file_name().unwrap().to_str().unwrap());
            let mut test = bundle::Merge::new(&settings);
            test.unpack_from(&bundle).unwrap();
            test.repack_to_write(&mut io::sink()).unwrap();
        } else {
            unimplemented!();
        }

        //println!("long   {}", app.switch_active("threads"));
        //println!("short  {}", app.switch_active("t"));
        //println!("switch {}", app.switch_active(NUM_THREADS));
        //println!("unused {}", app.unused_arguments());
        //println!("mem {}", std::mem::size_of_val(&app));
        //for param in app.params() {
        //    println!("{}", param.to_string_lossy());
        //}
    } else if let Err(e) = app {
        let _ = e;
    }
}

















