//! Clipping text a program can make as long as its heap holds (a specifier,
//! an import attribute's key, a path or message built from them) to a bound,
//! with a marker saying it was clipped and by how much, for the records a run
//! keeps outside V8's heap.

use std::borrow::Cow;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

/// `text`, when it's longer than `max` bytes, cut to at most `max` at a
/// character boundary and marked with its whole length:
/// `aaa… (1048576 bytes, clipped)`.
pub fn clip_str(text: &str, max: usize) -> Cow<'_, str> {
    if text.len() <= max {
        return Cow::Borrowed(text);
    }
    let head = &text[..text.floor_char_boundary(max)];
    Cow::Owned(format!("{head}… ({} bytes, clipped)", text.len()))
}

/// `text`, when it's longer than `max` bytes, clipped as [`clip_str`] clips
/// it, with the first 16 hex digits of the whole text's BLAKE3 hash in the
/// marker, so two texts that share their clipped head and their length
/// still differ: `aaa… (1048576 bytes, clipped, blake3 0123456789abcdef)`.
/// Text within the bound isn't hashed.
pub fn clip_str_hashed(text: &str, max: usize) -> Cow<'_, str> {
    if text.len() <= max {
        return Cow::Borrowed(text);
    }
    let head = &text[..text.floor_char_boundary(max)];
    let hash = blake3::hash(text.as_bytes()).to_hex();
    Cow::Owned(format!(
        "{head}… ({} bytes, clipped, blake3 {})",
        text.len(),
        &hash[..16]
    ))
}

/// [`clip_str`] of an owned `text`, which it returns as it is when it fits.
pub fn clip_string(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    clip_str(&text, max).into_owned()
}

/// [`clip_string`] of a boxed `text`.
pub fn clip_boxed(text: Box<str>, max: usize) -> Box<str> {
    if text.len() <= max {
        return text;
    }
    clip_str(&text, max).into()
}

/// `path`, when it's longer than `max` bytes, clipped as [`clip_str`] clips
/// text: at a character boundary when it's UTF-8, else at `max` bytes.
pub fn clip_path(path: PathBuf, max: usize) -> PathBuf {
    let length = path.as_os_str().len();
    if length <= max {
        return path;
    }
    match path.into_os_string().into_string() {
        Ok(text) => PathBuf::from(clip_string(text, max)),
        Err(raw) => {
            let mut bytes = raw.into_vec();
            bytes.truncate(max);
            bytes.extend_from_slice(format!("… ({length} bytes, clipped)").as_bytes());
            PathBuf::from(std::ffi::OsString::from_vec(bytes))
        }
    }
}

/// `text`, when it's longer than `max` bytes, with its middle cut out and
/// marked with how many bytes were: `cannot import 'aaa… (1048000 bytes
/// clipped) …aaa' from …`. A message's head and tail are kept, since a
/// message names what was refused first and, after it, the importer and why.
pub fn clip_middle(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    clip_middle_at(&text, max / 2, max)
}

/// `text`, when it's longer than `max` bytes, with its middle cut out as
/// [`clip_middle`] cuts it, keeping its first `head` bytes, at most `max`,
/// and as many of its last as make `max`.
pub fn clip_middle_at(text: &str, head: usize, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let head = head.min(max);
    let head_end = text.floor_char_boundary(head);
    let tail = text.ceil_char_boundary(text.len() - (max - head));
    format!(
        "{}… ({} bytes clipped) …{}",
        &text[..head_end],
        tail - head_end,
        &text[tail..]
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::{clip_middle, clip_middle_at, clip_path, clip_str, clip_str_hashed, clip_string};

    /// Text within the bound is kept as it is, borrowed; longer text is cut
    /// at a character boundary at or below the bound, marked with its whole
    /// length.
    #[test]
    fn text_past_the_bound_is_clipped_at_a_character_boundary() {
        assert!(matches!(
            clip_str("abc", 3),
            std::borrow::Cow::Borrowed("abc")
        ));
        assert_eq!(clip_str("abcd", 3), "abc… (4 bytes, clipped)");
        // 'é' is two bytes, so a cut at 2 falls inside it and backs off to 1
        assert_eq!(clip_str("aéz", 2), "a… (4 bytes, clipped)");
        assert_eq!(clip_string("abcd".to_owned(), 4), "abcd");
        assert_eq!(clip_string("abcd".to_owned(), 2), "ab… (4 bytes, clipped)");
    }

    /// A long text's marker carries the head of its whole hash, so two that
    /// share their clipped head and length differ, and a short one is
    /// borrowed as it is.
    #[test]
    fn a_hashed_clip_tells_texts_with_one_head_apart() {
        assert!(matches!(
            clip_str_hashed("abc", 3),
            std::borrow::Cow::Borrowed("abc")
        ));
        let hash = blake3::hash(b"abcd").to_hex();
        assert_eq!(
            clip_str_hashed("abcd", 3),
            format!("abc… (4 bytes, clipped, blake3 {})", &hash[..16])
        );
        assert_ne!(clip_str_hashed("abcd", 3), clip_str_hashed("abce", 3));
    }

    /// A path is clipped as text when it's UTF-8, and by bytes when it isn't.
    #[test]
    fn a_path_is_clipped_by_its_bytes() {
        assert_eq!(clip_path(PathBuf::from("/a/b"), 4), PathBuf::from("/a/b"));
        assert_eq!(
            clip_path(PathBuf::from("/a/bc"), 3),
            PathBuf::from("/a/… (5 bytes, clipped)")
        );
        let raw = PathBuf::from(OsString::from_vec(b"/a/\xff\xff".to_vec()));
        assert_eq!(
            clip_path(raw, 4).into_os_string().into_vec(),
            b"/a/\xff\xe2\x80\xa6 (5 bytes, clipped)"
        );
    }

    /// A long message keeps its head and tail, at character boundaries,
    /// marked with how many bytes of its middle were cut.
    #[test]
    fn a_message_is_clipped_in_its_middle() {
        assert_eq!(clip_middle("abcdef".to_owned(), 6), "abcdef");
        assert_eq!(
            clip_middle(format!("head {} tail", "x".repeat(100)), 10),
            "head … (100 bytes clipped) … tail"
        );
        // the cuts back off out of a two-byte character on both sides
        assert_eq!(
            clip_middle("aééééz".to_owned(), 4),
            "a… (8 bytes clipped) …z"
        );
    }

    /// A middle cut can keep more of the head than the tail, never more
    /// than the bound, and keeps text within the bound as it is.
    #[test]
    fn a_middle_cut_keeps_the_head_it_is_given() {
        assert_eq!(clip_middle_at("abcdef", 5, 6), "abcdef");
        assert_eq!(
            clip_middle_at("headxxxxxxtl", 4, 6),
            "head… (6 bytes clipped) …tl"
        );
        assert_eq!(
            clip_middle_at("headxxxxxxtl", 9, 6),
            "headxx… (6 bytes clipped) …"
        );
    }
}
