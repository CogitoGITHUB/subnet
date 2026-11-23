use console_plugin_protocol::{PluginInput, PluginOutput};
use console_protocol::{
    ShellError,
    shell_error::{self, io::IoError},
};
use serde::Deserialize;

use crate::{Encoder, PluginEncoder};

/// A `PluginEncoder` that enables the plugin to communicate with Nushell with JSON
/// serialized data.
///
/// Each message in the stream is followed by a newline when serializing, but is not required for
/// deserialization. The output is not pretty printed and each object does not contain newlines.
/// If it is more convenient, a plugin may choose to separate messages by newline.
#[derive(Clone, Copy, Debug)]
pub struct JsonSerializer;

impl PluginEncoder for JsonSerializer {
    fn name(&self) -> &str {
        "json"
    }
}

impl Encoder<PluginInput> for JsonSerializer {
    fn encode(
        &self,
        plugin_input: &PluginInput,
        writer: &mut impl std::io::Write,
    ) -> Result<(), console_protocol::ShellError> {
        serde_json::to_writer(&mut *writer, plugin_input).map_err(json_encode_err)?;
        writer.write_all(b"\n").map_err(|err| {
            ShellError::Io(IoError::new_internal(
                err,
                "Failed to write final line break",
            ))
        })
    }

    fn decode(
        &self,
        reader: &mut impl std::io::BufRead,
    ) -> Result<Option<PluginInput>, console_protocol::ShellError> {
        let mut de = serde_json::Deserializer::from_reader(reader);
        PluginInput::deserialize(&mut de)
            .map(Some)
            .or_else(json_decode_err)
    }
}

impl Encoder<PluginOutput> for JsonSerializer {
    fn encode(
        &self,
        plugin_output: &PluginOutput,
        writer: &mut impl std::io::Write,
    ) -> Result<(), ShellError> {
        serde_json::to_writer(&mut *writer, plugin_output).map_err(json_encode_err)?;
        writer.write_all(b"\n").map_err(|err| {
            ShellError::Io(IoError::new_internal(
                err,
                "JsonSerializer could not encode linebreak",
            ))
        })
    }

    fn decode(
        &self,
        reader: &mut impl std::io::BufRead,
    ) -> Result<Option<PluginOutput>, ShellError> {
        let mut de = serde_json::Deserializer::from_reader(reader);
        PluginOutput::deserialize(&mut de)
            .map(Some)
            .or_else(json_decode_err)
    }
}

/// Handle a `serde_json` encode error.
fn json_encode_err(err: serde_json::Error) -> ShellError {
    if err.is_io() {
        ShellError::Io(IoError::new_internal(
            shell_error::io::ErrorKind::from_std(err.io_error_kind().expect("is io")),
            "Could not encode with json",
        ))
    } else {
        ShellError::PluginFailedToEncode {
            msg: err.to_string(),
        }
    }
}

/// Handle a `serde_json` decode error. Returns `Ok(None)` on eof.
fn json_decode_err<T>(err: serde_json::Error) -> Result<Option<T>, ShellError> {
    if err.is_eof() {
        Ok(None)
    } else if err.is_io() {
        Err(ShellError::Io(IoError::new_internal(
            shell_error::io::ErrorKind::from_std(err.io_error_kind().expect("is io")),
            "Could not decode with json",
        )))
    } else {
        Err(ShellError::PluginFailedToDecode {
            msg: err.to_string(),
        })
    }
}

