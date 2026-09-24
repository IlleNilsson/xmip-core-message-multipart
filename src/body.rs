//! The body: its parts cut by `codec::mime`, the estate's one multipart
//! reader, each named by its `Content-Disposition` name, else its
//! `Content-ID`, and carrying its own `Content-Type` as its media type.

use codec::mime::{self, parameter};
use message::{Part, Stop};

/// The parts between the delimiters of `boundary`, as the shape names them.
///
/// # Errors
/// No opening delimiter, no closing delimiter, or a part whose headers
/// never end.
pub fn parts(bytes: &[u8], boundary: &str) -> Result<Vec<Part>, Stop> {
    let parts = mime::read(bytes, boundary).map_err(|refusal| (refusal.reason, refusal.offset))?;
    Ok(parts
        .into_iter()
        .map(|part| {
            let name = part
                .header_value("Content-Disposition")
                .and_then(|disposition| parameter(disposition, "name"))
                .or_else(|| part.content_id())
                .map(str::to_string);
            let media_type = part.content_type().map(str::to_string);
            Part::new(name, part.body, media_type)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_named_by_its_disposition_else_its_content_id() {
        let bytes = b"--b\ncontent-type: A/B\nCONTENT-ID: <c>\n\n--b\n\
Content-Disposition: form-data; name=\"file\"\nContent-ID: <d>\n\nx\n--b--";
        let cut = parts(bytes, "b").expect("cuts");
        assert_eq!(cut[0].bytes, b"");
        assert_eq!(cut[0].media_type.as_deref(), Some("A/B"));
        assert_eq!(cut[0].name.as_deref(), Some("c"));
        assert_eq!(cut[1].name.as_deref(), Some("file"));
        assert_eq!(cut[1].media_type, None);
        assert_eq!(
            parts(b"--b\n", "b").expect_err("open").0,
            "no closing boundary"
        );
    }
}
