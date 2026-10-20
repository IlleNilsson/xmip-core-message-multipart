#![forbid(unsafe_code)]

//! Multipart: a body of parts, each with its own headers, split on the
//! boundary the media type declares — `multipart/form-data; boundary=...`,
//! `multipart/mixed`, `multipart/related` — or, when no media type says,
//! on the `--` boundary line the body opens with. Each part is named by its
//! `Content-Disposition` name, else its `Content-ID`, and carries its own
//! `Content-Type` as its media type. The announced type is the `type`
//! parameter of `multipart/related` when there is one: what an AS2 or a
//! SOAP-with-attachments body says its root part is (ADR-0047).
//!
//! The body is cut by `codec::mime`, the one multipart reader the estate
//! has, which the transports that carry MIME read with too: the delimiters
//! are RFC 2046's — `--boundary` on a line of its own, the last one
//! followed by `--` — and a part's bytes are exactly what lies between its
//! blank line and the next delimiter, less the line break that belongs to
//! the delimiter. The preamble and the epilogue are dropped. The shape
//! reads the headers for the three names above and nothing else.

mod body;

use codec::mime::{self, parameter};
use message::{Shape, ShapeError, Shaped, media_type_of};
use stream::Stream;

/// The multipart shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct Multipart;

impl Shape for Multipart {
    fn technology(&self) -> &'static str {
        "multipart"
    }

    fn media_types(&self) -> &'static [&'static str] {
        &[
            "multipart/form-data",
            "multipart/mixed",
            "multipart/related",
            "multipart/alternative",
            "multipart/signed",
            "multipart/encrypted",
        ]
    }

    fn recognizes(&self, bytes: &[u8]) -> bool {
        mime::opening_boundary(bytes).is_some()
    }

    fn shape(&self, stream: &Stream) -> Result<Shaped, ShapeError> {
        let declared = stream.media_type().and_then(|m| parameter(m, "boundary"));
        let boundary = match declared {
            Some(boundary) => boundary,
            None => mime::opening_boundary(stream.bytes())
                .ok_or_else(|| ShapeError::new("multipart", "no boundary").at(0))?,
        };
        let parts = body::parts(stream.bytes(), boundary)
            .map_err(|stop| ShapeError::refused("multipart", stop))?;
        let message_type = stream
            .media_type()
            .filter(|_| media_type_of(stream).as_deref() == Some("multipart/related"))
            .and_then(|m| parameter(m, "type"))
            .map(str::to_string);
        Ok(Shaped {
            parts,
            message_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::StreamId;

    const FORM: &[u8] = b"preamble\r\n--b1\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n\
Content-Type: text/plain\r\n\r\nhello\r\n\r\n--b1\r\n\
Content-Disposition: form-data; name=\"note\"\r\n\r\nx\r\n--b1--\r\nepilogue";

    fn stream(bytes: &[u8], media: Option<&str>) -> Stream {
        Stream::new(StreamId::new(1), bytes.to_vec(), media.map(str::to_string))
    }

    #[test]
    fn a_body_splits_on_its_boundary_into_named_parts_with_their_own_media_types() {
        let media = Some("multipart/form-data; boundary=b1");
        let shaped = Multipart.shape(&stream(FORM, media)).expect("well-formed");
        assert_eq!(shaped.parts.len(), 2);
        assert_eq!(shaped.parts[0].name.as_deref(), Some("file"));
        assert_eq!(shaped.parts[0].bytes, b"hello\r\n");
        assert_eq!(shaped.parts[0].media_type.as_deref(), Some("text/plain"));
        assert_eq!(shaped.parts[1].name.as_deref(), Some("note"));
        assert_eq!(shaped.parts[1].bytes, b"x");
        assert_eq!(shaped.parts[1].media_type, None);
        assert_eq!(shaped.message_type, None);

        let related =
            b"--r\r\nContent-Type: application/xop+xml\r\nContent-ID: <root@x>\r\n\r\n<a/>\r\n\
--r\r\nContent-ID: <img@x>\r\nContent-Type: image/png\r\n\r\n\x89PNG\r\n--r--";
        let media = Some("Multipart/Related; type=\"application/xop+xml\"; boundary=\"r\"");
        let shaped = Multipart
            .shape(&stream(related, media))
            .expect("well-formed");
        assert_eq!(shaped.parts[0].name.as_deref(), Some("root@x"));
        assert_eq!(shaped.parts[1].name.as_deref(), Some("img@x"));
        assert_eq!(shaped.parts[1].bytes, b"\x89PNG");
        assert_eq!(shaped.message_type.as_deref(), Some("application/xop+xml"));
    }

    #[test]
    fn a_body_without_its_boundary_or_its_blank_line_or_its_close_is_refused_where_it_fails() {
        let missing = Multipart
            .shape(&stream(
                b"--x\r\n\r\nbody\r\n--x--",
                Some("multipart/mixed; boundary=b1"),
            ))
            .expect_err("no delimiter");
        assert_eq!(missing.offset, Some(0));
        assert_eq!(
            missing.to_string(),
            "multipart: no opening boundary at byte 0"
        );

        let unclosed = Multipart
            .shape(&stream(
                b"--b1\r\n\r\nbody\r\n",
                Some("multipart/mixed; boundary=b1"),
            ))
            .expect_err("no close");
        assert_eq!(unclosed.offset, Some(14));
        assert_eq!(unclosed.reason, "no closing boundary");

        let headers = Multipart
            .shape(&stream(
                b"--b1\r\nContent-Type: text/plain\r\nbody\r\n--b1--",
                None,
            ))
            .expect_err("no blank line");
        assert_eq!(headers.offset, Some(6));
        assert_eq!(headers.reason, "no blank line after the part headers");

        let none = Multipart
            .shape(&stream(b"plain", Some("multipart/mixed")))
            .expect_err("no boundary");
        assert_eq!(none.reason, "no boundary");
    }

    #[test]
    fn the_shape_claims_the_multipart_types_and_recognizes_a_leading_boundary() {
        assert_eq!(Multipart.technology(), "multipart");
        assert!(Multipart.media_types().contains(&"multipart/form-data"));
        assert!(Multipart.media_types().contains(&"multipart/related"));
        assert!(Multipart.recognizes(b"--abc\r\n\r\nx\r\n--abc--"));
        assert!(Multipart.recognizes(b"--abc\n\nx\n--abc--"));
        assert!(!Multipart.recognizes(b"-- not a boundary"));
        assert!(!Multipart.recognizes(b"--\r\n"));
        assert!(!Multipart.recognizes(b"{}"));
    }

    #[test]
    fn choose_picks_multipart_by_media_type_and_by_look_and_shapes_without_a_media_type() {
        let shapes: [&dyn Shape; 1] = [&Multipart];
        let by_media = message::choose(&shapes, &stream(b"x", Some("multipart/mixed; boundary=q")));
        assert_eq!(by_media.map(Shape::technology), Some("multipart"));
        let bare = stream(b"--q\n\none\n--q\nContent-ID: two\n\n2\n--q--\n", None);
        let by_look = message::choose(&shapes, &bare).expect("recognized");
        let sectioned = by_look.shape(&bare).expect("well-formed");
        assert_eq!(sectioned.parts.len(), 2);
        assert_eq!(sectioned.parts[0].name, None);
        assert_eq!(sectioned.parts[0].bytes, b"one");
        assert_eq!(sectioned.parts[1].name.as_deref(), Some("two"));
        assert!(message::choose(&shapes, &stream(b"plain", None)).is_none());
    }
}
