//! Owned bundled faces. Adapters load bytes; decoding and registry lifetime live here.
use blinc_text::{FontFace, font::FontData, global_font_registry};
#[cfg(feature = "hashlink")]
use hl_abi::{define_prim, vbyte};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT_NAME: AtomicU64 = AtomicU64::new(1);

/// TTF or OpenType bytes: WOFF2 is unpacked, anything else is passed through.
fn plain(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.starts_with(b"wOF2") {
        woff2_patched::convert_woff2_to_ttf(&mut std::io::Cursor::new(bytes))
            .map_err(|e| format!("WOFF2: {e}"))
    } else {
        Ok(bytes)
    }
}

/// Registers the faces in `len` bytes of TTF, OpenType, a collection or WOFF2 as installed
/// fonts, found by the family names they carry; how many faces it added. A page has no system
/// fonts, so its program bundles its own.
#[cfg(feature = "hashlink")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hl_blinc_font_register(bytes: *const vbyte, len: i32) -> i32 {
    if bytes.is_null() || len <= 0 {
        return 0;
    }
    let _blocking = crate::hl::Blocking::enter();
    let data = unsafe { std::slice::from_raw_parts(bytes, len as usize) }.to_vec();
    let Ok(data) = plain(data) else {
        return 0;
    };
    let registry = global_font_registry();
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    let added = registry.load_font_data(data);
    if added > 0 {
        // A generic family that found no face before may find this one.
        registry.invalidate_generic_cache();
    }
    added as i32
}
#[cfg(feature = "hashlink")]
define_prim!(hlp_blinc_font_register, hl_blinc_font_register, "Bi_i");

pub struct Font {
    data: FontData,
    index: u32,
    aliases: Mutex<HashMap<u16, String>>,
}

impl Font {
    /// TTF, OpenType or WOFF2. The first face of a collection is used.
    pub fn decode(bytes: Vec<u8>) -> Result<Self, String> {
        let data = FontData::from_vec(plain(bytes)?);
        FontFace::from_font_data(data.clone(), 0).map_err(|e| e.to_string())?;
        Ok(Self {
            data,
            index: 0,
            aliases: Mutex::new(HashMap::new()),
        })
    }

    /// An installed face used by src: local(). No font bytes are copied.
    pub fn local(name: &str, weight: u16, italic: bool) -> Result<Self, String> {
        let face = global_font_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .load_font_with_style(name, weight, italic)
            .map_err(|e| e.to_string())?;
        let index = face.face_index();
        struct SharedFace(Arc<FontFace>);
        impl AsRef<[u8]> for SharedFace {
            fn as_ref(&self) -> &[u8] {
                self.0.data()
            }
        }
        Ok(Self {
            data: FontData::from_mapped(Arc::new(SharedFace(face))),
            index,
            aliases: Mutex::new(HashMap::new()),
        })
    }

    /// A private immutable family name for this file at the requested variable weight.
    /// Static faces also share one name per weight; CSS selects their descriptors separately.
    pub fn family(&self, weight: u16) -> Result<String, String> {
        if !(1..=1000).contains(&weight) {
            return Err("font weight must be 1..1000".into());
        }
        let mut aliases = self.aliases.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(name) = aliases.get(&weight) {
            return Ok(name.clone());
        }
        let mut face =
            FontFace::from_font_data(self.data.clone(), self.index).map_err(|e| e.to_string())?;
        face.set_variation_weight(weight);
        let name = format!(
            "blinc-private-font-{}",
            NEXT_NAME.fetch_add(1, Ordering::Relaxed)
        );
        let registered = global_font_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register_face(name.clone(), Arc::new(face));
        assert!(registered, "private font names must be unique");
        aliases.insert(weight, name.clone());
        Ok(name)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let aliases = self.aliases.get_mut().unwrap_or_else(|e| e.into_inner());
        if aliases.is_empty() {
            return;
        }
        let registry = global_font_registry();
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        for name in aliases.values() {
            registry.unregister_face(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_fonts_fail_without_registering() {
        for bytes in [vec![], b"not a font".to_vec(), b"wOF2broken".to_vec()] {
            assert!(Font::decode(bytes).is_err());
        }
    }
    #[test]
    fn bundled_faces_share_measure_and_render_registry_and_release_their_names() {
        let ttf =
            Font::decode(include_bytes!("../fixtures/font-face/regular.ttf").to_vec()).unwrap();
        let woff =
            Font::decode(include_bytes!("../fixtures/font-face/regular.woff2").to_vec()).unwrap();
        let a = ttf.family(400).unwrap();
        assert_eq!(ttf.family(400).unwrap(), a);
        let b = woff.family(700).unwrap();
        assert_ne!(a, b);
        let registry = global_font_registry();
        {
            let mut registry = registry.lock().unwrap();
            let font = registry.load_font_with_style(&a, 400, true).unwrap();
            assert_eq!(font.family_name(), "Blinc Embedded Fixture");
            assert_eq!(
                font.glyph_advance(font.glyph_id('M').unwrap()).unwrap(),
                500
            );
            let other = registry.load_font_with_style(&b, 900, false).unwrap();
            assert_eq!(
                font.glyph_advance(font.glyph_id('M').unwrap()),
                other.glyph_advance(other.glyph_id('M').unwrap())
            );
        }
        drop(ttf);
        let mut registry = registry.lock().unwrap();
        assert!(!registry.has_font(&a));
        assert!(registry.has_font(&b));
    }
}
