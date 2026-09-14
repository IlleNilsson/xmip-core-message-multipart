//! The body: delimiters found, parts cut between them, each part's headers
//! read for its name and media type.

use message::{Part, Stop, parameter};

/// The boundary a body declares by opening with `--boundary` on its first
/// line, when no media type declared one.
pub fn opening_boundary(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(b"--")?;
    let end = rest.iter().position(|b| *b == b'\r' || *b == b'\n')?;
    let boundary = rest[..end].trim_ascii_end();
    let sound = !boundary.is_empty()
        && boundary.len() <= 70
        && boundary
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"'()+_,-./:=?".contains(b));
    sound.then_some(boundary)
}

/// The parts between the delimiters of `boundary`.
///
/// # Errors
/// No opening delimiter, no closing delimiter, or a part whose headers
/// never end.
pub fn parts(bytes: &[u8], boundary: &[u8]) -> Result<Vec<Part>, Stop> {
    let delimiter = [b"--", boundary].concat();
    let mut at = delimiter_at(bytes, &delimiter, 0).ok_or(("no opening boundary", 0))?;
    let mut parts = Vec::new();
    loop {
        at += delimiter.len();
        if bytes[at..].starts_with(b"--") {
            return Ok(parts);
        }
        let start = past_line(bytes, at);
        let Some(next) = delimiter_at(bytes, &delimiter, start) else {
            return Err(("no closing boundary", bytes.len()));
        };
        parts.push(part(&bytes[start..next], start)?);
        at = next;
    }
}

/// The next `delimiter` at the start of a line, at or after `from`.
fn delimiter_at(bytes: &[u8], delimiter: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    loop {
        let found = at
            + bytes[at..]
                .windows(delimiter.len())
                .position(|w| w == delimiter)?;
        let on_a_line = found == 0 || bytes[found - 1] == b'\n';
        if on_a_line && closes_the_line(&bytes[found + delimiter.len()..]) {
            return Some(found);
        }
        at = found + 1;
    }
}

/// Whether what follows a delimiter is a line's end — transport padding,
/// then a line break or the end of the body — or the `--` that closes.
fn closes_the_line(after: &[u8]) -> bool {
    let rest = after
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .map_or(&[][..], |n| &after[n..]);
    rest.is_empty() || rest.starts_with(b"--") || rest[0] == b'\r' || rest[0] == b'\n'
}

/// The byte after the line break that ends the line at `at`.
fn past_line(bytes: &[u8], at: usize) -> usize {
    bytes[at..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |n| at + n + 1)
}

/// One part from its headers and body, the body's trailing line break
/// (which belongs to the delimiter after it) removed.
fn part(section: &[u8], offset: usize) -> Result<Part, Stop> {
    let (headers, body) =
        split_headers(section).ok_or(("no blank line after the part headers", offset))?;
    let body = body
        .strip_suffix(b"\r\n")
        .or_else(|| body.strip_suffix(b"\n"))
        .unwrap_or(body);
    let headers = String::from_utf8_lossy(headers);
    let name = header(&headers, "content-disposition")
        .and_then(|disposition| parameter(disposition, "name").map(str::to_string))
        .or_else(|| header(&headers, "content-id").map(content_id));
    let media_type = header(&headers, "content-type").map(str::to_string);
    Ok(Part::new(name, body, media_type))
}

/// The headers and the body, cut at the first blank line.
fn split_headers(section: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut at = 0;
    while at < section.len() {
        let line_end = section[at..].iter().position(|b| *b == b'\n')?;
        let line = section[at..at + line_end]
            .strip_suffix(b"\r")
            .unwrap_or(&section[at..at + line_end]);
        if line.is_empty() {
            return Some((&section[..at], &section[at + line_end + 1..]));
        }
        at += line_end + 1;
    }
    None
}

/// The value of the header `name`, continuation lines joined.
fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

/// A Content-ID without its angle brackets.
fn content_id(value: &str) -> String {
    value
        .strip_prefix('<')
        .and_then(|v| v.strip_suffix('>'))
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delimiter_counts_only_at_the_start_of_a_line() {
        let bytes = b"x--b\r\n--b\r\n\r\n--bx\r\n--b--";
        assert_eq!(delimiter_at(bytes, b"--b", 0), Some(6));
        assert_eq!(delimiter_at(bytes, b"--b", 7), Some(19));
        let parts = parts(bytes, b"b").expect("cuts");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].bytes, b"--bx");
    }

    #[test]
    fn a_body_may_be_empty_and_headers_may_fold_case() {
        let bytes = b"--b\ncontent-type: A/B\nCONTENT-ID: <c>\n\n--b--";
        let parts = parts(bytes, b"b").expect("cuts");
        assert_eq!(parts[0].bytes, b"");
        assert_eq!(parts[0].media_type.as_deref(), Some("A/B"));
        assert_eq!(parts[0].name.as_deref(), Some("c"));
        assert_eq!(
            opening_boundary(b"--=_Part_1\r\n"),
            Some(b"=_Part_1".as_slice())
        );
        assert_eq!(opening_boundary(b"--a b\r\n"), None);
    }
}
