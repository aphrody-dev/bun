//! The C ABI of `src/jsc/bindings/RegularExpression.cpp` that `bun_yarr` links against, on `regress` (Boa's
//! regular expressions) instead of JavaScriptCore's Yarr. `bun lint` reaches it through `bun_lint`.

use core::ffi::c_void;

/// `bun_yarr::flags`
const IGNORE_CASE: u16 = 1 << 2;
const MULTILINE: u16 = 1 << 3;
const DOT_ALL: u16 = 1 << 4;
const UNICODE: u16 = 1 << 5;
const UNICODE_SETS: u16 = 1 << 6;
const STICKY: u16 = 1 << 7;

/// `bun_yarr::NO_MATCH`
const NO_MATCH: u32 = u32::MAX;

struct Expression {
    regex: regress::Regex,
    unicode: bool,
    sticky: bool,
}

/// # Safety
/// `characters` has `length` characters, of one byte if `is_8_bit`, of two otherwise.
unsafe fn units<'a>(characters: *const c_void, length: u32, is_8_bit: bool) -> std::borrow::Cow<'a, [u16]> {
    if length == 0 {
        return std::borrow::Cow::Borrowed(&[]);
    }
    if is_8_bit {
        // SAFETY: the caller's.
        let bytes = unsafe { core::slice::from_raw_parts(characters.cast::<u8>(), length as usize) };
        std::borrow::Cow::Owned(bytes.iter().map(|&b| u16::from(b)).collect())
    } else {
        // SAFETY: the caller's.
        std::borrow::Cow::Borrowed(unsafe { core::slice::from_raw_parts(characters.cast::<u16>(), length as usize) })
    }
}

/// The code points of a pattern: pairs of surrogates are one only with the `u` or `v` flag.
fn code_points(units: &[u16], unicode: bool) -> Vec<u32> {
    if !unicode {
        return units.iter().map(|&u| u32::from(u)).collect();
    }
    char::decode_utf16(units.iter().copied()).map(|it| it.map_or_else(|e| u32::from(e.unpaired_surrogate()), u32::from)).collect()
}

/// # Safety
/// As `Yarr__RegularExpression__compile` in RegularExpression.cpp.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Yarr__RegularExpression__compile(
    characters: *const c_void,
    length: u32,
    is_8_bit: bool,
    flags: u16,
    offsets_size: &mut u32,
    refusal: &mut u8,
) -> *mut c_void {
    // SAFETY: the caller's.
    let pattern = unsafe { units(characters, length, is_8_bit) };
    let unicode = flags & (UNICODE | UNICODE_SETS) != 0;
    let options = regress::Flags {
        icase: flags & IGNORE_CASE != 0,
        multiline: flags & MULTILINE != 0,
        dot_all: flags & DOT_ALL != 0,
        unicode: flags & UNICODE != 0,
        unicode_sets: flags & UNICODE_SETS != 0,
        no_opt: false,
    };
    let points = code_points(&pattern, unicode);
    match regress::Regex::from_unicode(points.iter().copied(), options) {
        Ok(regex) => {
            // At least two for each group and two for the match: a `(` starts every group.
            let groups = pattern.iter().filter(|&&u| u == u16::from(b'(')).count() as u32;
            *offsets_size = 2 * (groups + 1);
            let sticky = flags & STICKY != 0;
            Box::into_raw(Box::new(Expression { regex, unicode, sticky })).cast()
        }
        Err(err) => {
            let text = err.to_string();
            *refusal = if text.contains("property") || text.contains("Property") { 2 } else { 3 };
            core::ptr::null_mut()
        }
    }
}

/// # Safety
/// As `Yarr__RegularExpression__exec` in RegularExpression.cpp: `offsets` has room for the `offsets_size` of
/// the compile that made `this`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Yarr__RegularExpression__exec(
    this: *mut c_void,
    characters: *const c_void,
    length: u32,
    is_8_bit: bool,
    start: u32,
    offsets: *mut u32,
) -> bool {
    // SAFETY: made by compile.
    let this = unsafe { &*this.cast::<Expression>() };
    // SAFETY: the caller's.
    let text = unsafe { units(characters, length, is_8_bit) };
    let found = if this.unicode {
        this.regex.find_from_utf16(&text, start as usize).next()
    } else {
        this.regex.find_from_ucs2(&text, start as usize).next()
    };
    let Some(found) = found.filter(|m| !this.sticky || m.range.start == start as usize) else {
        return false;
    };
    let size = 2 * (found.captures.len() + 1);
    // SAFETY: compile counted at least as many groups as regress has.
    let offsets = unsafe { core::slice::from_raw_parts_mut(offsets, size) };
    let ranges = core::iter::once(Some(found.range.clone())).chain(found.captures.iter().cloned());
    for (slot, range) in offsets.chunks_exact_mut(2).zip(ranges) {
        match range {
            Some(range) => {
                slot[0] = range.start as u32;
                slot[1] = range.end as u32;
            }
            None => slot[0] = NO_MATCH,
        }
    }
    true
}

/// # Safety
/// `this` is from compile and nothing uses it after.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Yarr__RegularExpression__deinit(this: *mut c_void) {
    // SAFETY: the caller's.
    drop(unsafe { Box::from_raw(this.cast::<Expression>()) });
}

/// Nothing to start: regress has no options to read.
#[unsafe(no_mangle)]
pub fn __bun_yarr_initialize() {}
