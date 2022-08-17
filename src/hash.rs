use std::hash::Hash;
use std::hash::Hasher;

#[derive(PartialEq, Eq)]
pub struct MurmurHash64(u64);

impl MurmurHash64 {
    pub fn new<T: AsRef<[u8]>>(key: T) -> Self {
        Self(stingray_hash64(key.as_ref()))
    }

    pub fn from_u64(hash: u64) -> Self {
        Self(hash)
    }
}

impl Hash for MurmurHash64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0);
    }
}

pub(crate) fn extension_lookup(hash: u64) -> Option<&'static str> {
    Some(match hash {
        0x931e336d7646cc26 => "animation",
        0xdcfb9e18fff13984 => "animation_curves",
        0xaa5965f03029fa18 => "bik",
        0xe301e8af94e3b5a3 => "blend_set",
        0x18dead01056b72e9 => "bones",
        0xb7893adf7567506a => "chroma",
        0xfe9754bd19814a47 => "common_package",
        0x82645835e6b73232 => "config",
        0x69108ded1e3e634b => "crypto",
        0x8fd0d44d20650b68 => "data",
        0x9831ca893b0d087d => "entity",
        0x92d3ee038eeb610d => "flow",
        0x9efe0a916aae7880 => "font",
        0xd526a27da14f1dc5 => "ini",
        0xfa4a8e091a91201e => "ivf",
        0xa62f9297dc969e85 => "keys",
        0x2a690fd348fe9ac5 => "level",
        0xa14e8dfa2cd117e2 => "lua",
        0xeac0b497876adedf => "material",
        0x3fcdd69156a46417 => "mod",
        0xb277b11fe4a61d37 => "mouse_cursor",
        0x169de9566953d264 => "navdata",
        0x3b1fa9e8f6bac374 => "network_config",
        0xad9c6d9ed1e5e77a => "package",
        0xa8193123526fad64 => "particles",
        0xbf21403a3ab0bbb1 => "physics_properties",
        0x27862fe24795319c => "render_config",
        0x9d0a795bfe818d19 => "scene",
        0xcce8d5b5f5ae333f => "shader",
        0xe5ee32a477239a93 => "shader_library",
        0x9e5c3cc74575aeb5 => "shader_library_group",
        0xfe73c7dcff8a7ca5 => "shading_environment",
        0x250e0a11ac8e26f8 => "shading_environment_mapping",
        0xa27b4d04a9ba6f9e => "slug",
        0xa486d4045106165c => "state_machine",
        0x0d972bab10b40fd3 => "strings",
        0xad2d3fa30d9ab394 => "surface_properties",
        0xcd4238c6a0c69e32 => "texture",
        0x99736be1fff739a4 => "timpani_bank",
        0x00a3e6c59a2b9c6c => "timpani_master",
        0x19c792357c99f49b => "tome",
        0xe0a48d0be9a7453f => "unit",
        0xf7505933166d6755 => "vector_field",
        0x535a7bd3e650d799 => "wwise_bank",
        0xaf32095c82f2b070 => "wwise_dep",
        0xd50a8b7e1c82b110 => "wwise_metadata",
        0x504b55235d21440e => "wwise_stream",
        _ => return None,
    })
}

pub(crate) const fn stingray_hash64(key: &[u8]) -> u64 {
    murmur_hash64a(key, 0)
}

// https://github.com/badboy/murmurhash64-rs/blob/3f9a5821650de6ee12f3cc45701444171ce30ebf/src/lib.rs#L44
#[allow(clippy::identity_op, clippy::many_single_char_names)]
#[rustfmt::skip]
pub(crate) const fn murmur_hash64a(key: &[u8], seed: u64) -> u64 {
    let m : u64 = 0xc6a4a7935bd1e995;
    let r : u8 = 47;

    let len = key.len();
    let mut h : u64 = seed ^ ((len as u64).wrapping_mul(m));

    let endpos = len-(len&7);
    let mut i = 0;
    while i != endpos {
        let mut k : u64;

        k  = key[i+0] as u64;
        k |= (key[i+1] as u64) << 8;
        k |= (key[i+2] as u64) << 16;
        k |= (key[i+3] as u64) << 24;
        k |= (key[i+4] as u64) << 32;
        k |= (key[i+5] as u64) << 40;
        k |= (key[i+6] as u64) << 48;
        k |= (key[i+7] as u64) << 56;

        k = k.wrapping_mul(m);
        k ^= k >> r;
        k = k.wrapping_mul(m);
        h ^= k;
        h = h.wrapping_mul(m);

        i += 8;
    };

    let over = len & 7;
    if over == 7 { h ^= (key[i+6] as u64) << 48; }
    if over >= 6 { h ^= (key[i+5] as u64) << 40; }
    if over >= 5 { h ^= (key[i+4] as u64) << 32; }
    if over >= 4 { h ^= (key[i+3] as u64) << 24; }
    if over >= 3 { h ^= (key[i+2] as u64) << 16; }
    if over >= 2 { h ^= (key[i+1] as u64) << 8; }
    if over >= 1 { h ^= key[i+0] as u64; }
    if over >  0 { h = h.wrapping_mul(m); }

    h ^= h >> r;
    h = h.wrapping_mul(m);
    h ^= h >> r;
    h
}

