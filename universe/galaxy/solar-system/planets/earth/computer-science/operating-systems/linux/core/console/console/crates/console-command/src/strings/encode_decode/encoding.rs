use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use console_protocol::shell_error::generic::GenericError;
use console_protocol::{ShellError, Span, Spanned, Value};

pub fn detect_encoding_name(
    _head: Span,
    _input: Span,
    bytes: &[u8],
) -> Result<&'static Encoding, ShellError> {
    let mut detector = EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    let _non_ascii = detector.feed(bytes, false);
    //Guess(TLD=None(usually used in HTML), Allow_UTF8=True)
    let encoding = detector.guess(None, chardetng::Utf8Detection::Allow);
    Ok(encoding)
}

pub fn decode(
    head: Span,
    encoding_name: Spanned<String>,
    bytes: &[u8],
) -> Result<Value, ShellError> {
    // Workaround for a bug in the Encodings Specification.
    let encoding = if encoding_name.item.eq_ignore_ascii_case("utf16") {
        parse_encoding(encoding_name.span, "utf-16")
    } else {
        parse_encoding(encoding_name.span, &encoding_name.item)
    }?;
    let (result, ..) = encoding.decode(bytes);
    Ok(Value::string(result.into_owned(), head))
}

pub fn encode(
    head: Span,
    encoding_name: Spanned<String>,
    s: &str,
    s_span: Span,
    ignore_errors: bool,
) -> Result<Value, ShellError> {
    // Workaround for a bug in the Encodings Specification.
    let encoding = if encoding_name.item.eq_ignore_ascii_case("utf16") {
        parse_encoding(encoding_name.span, "utf-16")
    } else {
        parse_encoding(encoding_name.span, &encoding_name.item)
    }?;

    // Since the Encoding Standard doesn't specify encoders for "UTF-16BE" and "UTF-16LE"
    // Check if the encoding is one of them and return an error
    if ["UTF-16BE", "UTF-16LE"].contains(&encoding.name()) {
        return Err(ShellError::Generic(
            GenericError::new(
                format!("{} encoding is not supported", encoding_name.item),
                "invalid encoding",
                encoding_name.span,
            )
            .with_help("refer to https://docs.rs/encoding_rs/latest/encoding_rs/index.html#statics for a valid list of encodings"),
        ));
    }

    let (result, _actual_encoding, replacements) = encoding.encode(s);
    // Because encoding_rs is a Web-facing crate, it defaults to replacing unknowns with HTML entities.
    // This behaviour can be enabled with -i. Otherwise, it becomes an error.
    if replacements && !ignore_errors {
        // TODO: make GenericError accept two spans (including head)
        Err(ShellError::Generic(GenericError::new(
            "error while encoding string",
            format!("string contained characters not in {}", encoding_name.item),
            s_span,
        )))
    } else {
        Ok(Value::binary(result.into_owned(), head))
    }
}

fn parse_encoding(span: Span, label: &str) -> Result<&'static Encoding, ShellError> {
    // Workaround for a bug in the Encodings Specification.
    let label = if label.eq_ignore_ascii_case("utf16") {
        "utf-16"
    } else {
        label
    };
    match Encoding::for_label_no_replacement(label.as_bytes()) {
        None => Err(ShellError::Generic(
            GenericError::new(
                format!("{label} is not a valid encoding"),
                "invalid encoding",
                span,
            )
            .with_help("refer to https://docs.rs/encoding_rs/latest/encoding_rs/index.html#statics for a valid list of encodings"),
        )),
        Some(encoding) => Ok(encoding),
    }
}

