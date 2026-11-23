// The general idea is that we NEVER send cleartext history to the server
// This way the odds of anything private ending up where it should not are
// very low
// The server authenticates via the usual username and password. This has
// nothing to do with the encryption, and is purely authentication! The client
// generates its own secret key, and encrypts all shell history with libsodium's
// secretbox. The data is then sent to the server, where it is stored. All
// clients must share the secret in order to be able to sync, as it is needed
// to decrypt

use std::{io::prelude::*, path::PathBuf};

use base64::prelude::{BASE64_STANDARD, Engine};
pub use crypto_secretbox::Key;
use crypto_secretbox::{
    AeadCore, AeadInPlace, KeyInit, XSalsa20Poly1305,
    aead::{Nonce, OsRng},
};
use eyre::{Context, Result, bail, ensure, eyre};
use fs_err as fs;
use rmp::{Marker, decode::Bytes};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339, macros::format_description};

use crate::{history::History, settings::Settings};

#[derive(Debug, Serialize, Deserialize)]
pub struct EncryptedHistory {
    pub ciphertext: Vec<u8>,
    pub nonce: Nonce<XSalsa20Poly1305>,
}

pub fn generate_encoded_key() -> Result<(Key, String)> {
    let key = XSalsa20Poly1305::generate_key(&mut OsRng);
    let encoded = encode_key(&key)?;

    Ok((key, encoded))
}

pub fn new_key(settings: &Settings) -> Result<Key> {
    let path = settings.key_path.as_str();
    let path = PathBuf::from(path);

    if path.exists() {
        bail!("key already exists! cannot overwrite");
    }

    let (key, encoded) = generate_encoded_key()?;

    let mut file = fs::File::create(path)?;
    file.write_all(encoded.as_bytes())?;

    Ok(key)
}

// Loads the secret key, will create + save if it doesn't exist
pub fn load_key(settings: &Settings) -> Result<Key> {
    let path = settings.key_path.as_str();

    let key = if PathBuf::from(path).exists() {
        let key = fs_err::read_to_string(path)?;
        decode_key(key)?
    } else {
        new_key(settings)?
    };

    Ok(key)
}

pub fn encode_key(key: &Key) -> Result<String> {
    let mut buf = vec![];
    rmp::encode::write_array_len(&mut buf, key.len() as u32)
        .wrap_err("could not encode key to message pack")?;
    for b in key {
        rmp::encode::write_uint(&mut buf, *b as u64)
            .wrap_err("could not encode key to message pack")?;
    }
    let buf = BASE64_STANDARD.encode(buf);

    Ok(buf)
}

pub fn decode_key(key: String) -> Result<Key> {
    use rmp::decode;

    let buf = BASE64_STANDARD
        .decode(key.trim_end())
        .wrap_err("encryption key is not a valid base64 encoding")?;

    // old code wrote the key as a fixed length array of 32 bytes
    // new code writes the key with a length prefix
    match <[u8; 32]>::try_from(&*buf) {
        Ok(key) => Ok(key.into()),
        Err(_) => {
            let mut bytes = rmp::decode::Bytes::new(&buf);

            match Marker::from_u8(buf[0]) {
                Marker::Bin8 => {
                    let len = decode::read_bin_len(&mut bytes).map_err(|err| eyre!("{err:?}"))?;
                    ensure!(len == 32, "encryption key is not the correct size");
                    let key = <[u8; 32]>::try_from(bytes.remaining_slice())
                        .context("could not decode encryption key")?;
                    Ok(key.into())
                }
                Marker::Array16 => {
                    let len = decode::read_array_len(&mut bytes).map_err(|err| eyre!("{err:?}"))?;
                    ensure!(len == 32, "encryption key is not the correct size");

                    let mut key = Key::default();
                    for i in &mut key {
                        *i = rmp::decode::read_int(&mut bytes).map_err(|err| eyre!("{err:?}"))?;
                    }
                    Ok(key)
                }
                _ => bail!("could not decode encryption key"),
            }
        }
    }
}

pub fn encrypt(history: &History, key: &Key) -> Result<EncryptedHistory> {
    // serialize with msgpack
    let mut buf = encode(history)?;

    let nonce = XSalsa20Poly1305::generate_nonce(&mut OsRng);
    XSalsa20Poly1305::new(key)
        .encrypt_in_place(&nonce, &[], &mut buf)
        .map_err(|_| eyre!("could not encrypt"))?;

    Ok(EncryptedHistory {
        ciphertext: buf,
        nonce,
    })
}

pub fn decrypt(mut encrypted_history: EncryptedHistory, key: &Key) -> Result<History> {
    XSalsa20Poly1305::new(key)
        .decrypt_in_place(
            &encrypted_history.nonce,
            &[],
            &mut encrypted_history.ciphertext,
        )
        .map_err(|_| eyre!("could not decrypt history"))?;
    let plaintext = encrypted_history.ciphertext;

    let history = decode(&plaintext)?;

    Ok(history)
}

fn format_rfc3339(ts: OffsetDateTime) -> Result<String> {
    // horrible hack. chrono AutoSI limits to 0, 3, 6, or 9 decimal places for nanoseconds.
    // time does not have this functionality.
    static PARTIAL_RFC3339_0: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    static PARTIAL_RFC3339_3: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");
    static PARTIAL_RFC3339_6: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");
    static PARTIAL_RFC3339_9: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z");

    let fmt = match ts.nanosecond() {
        0 => PARTIAL_RFC3339_0,
        ns if ns % 1_000_000 == 0 => PARTIAL_RFC3339_3,
        ns if ns % 1_000 == 0 => PARTIAL_RFC3339_6,
        _ => PARTIAL_RFC3339_9,
    };

    Ok(ts.format(fmt)?)
}

fn encode(h: &History) -> Result<Vec<u8>> {
    use rmp::encode;

    let mut output = vec![];
    // INFO: ensure this is updated when adding new fields
    encode::write_array_len(&mut output, 9)?;

    encode::write_str(&mut output, &h.id.0)?;
    encode::write_str(&mut output, &(format_rfc3339(h.timestamp)?))?;
    encode::write_sint(&mut output, h.duration)?;
    encode::write_sint(&mut output, h.exit)?;
    encode::write_str(&mut output, &h.command)?;
    encode::write_str(&mut output, &h.cwd)?;
    encode::write_str(&mut output, &h.session)?;
    encode::write_str(&mut output, &h.hostname)?;
    match h.deleted_at {
        Some(d) => encode::write_str(&mut output, &format_rfc3339(d)?)?,
        None => encode::write_nil(&mut output)?,
    }

    Ok(output)
}

fn decode(bytes: &[u8]) -> Result<History> {
    use rmp::decode::{self, DecodeStringError};

    let mut bytes = Bytes::new(bytes);

    let nfields = decode::read_array_len(&mut bytes).map_err(error_report)?;
    if nfields < 8 {
        bail!("malformed decrypted history")
    }
    if nfields > 9 {
        bail!("cannot decrypt history from a newer version of console history");
    }

    let bytes = bytes.remaining_slice();
    let (id, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;
    let (timestamp, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;

    let mut bytes = Bytes::new(bytes);
    let duration = decode::read_int(&mut bytes).map_err(error_report)?;
    let exit = decode::read_int(&mut bytes).map_err(error_report)?;

    let bytes = bytes.remaining_slice();
    let (command, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;
    let (cwd, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;
    let (session, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;
    let (hostname, bytes) = decode::read_str_from_slice(bytes).map_err(error_report)?;

    // if we have more fields, try and get the deleted_at
    let mut deleted_at = None;
    let mut bytes = bytes;
    if nfields > 8 {
        bytes = match decode::read_str_from_slice(bytes) {
            Ok((d, b)) => {
                deleted_at = Some(d);
                b
            }
            // we accept null here
            Err(DecodeStringError::TypeMismatch(Marker::Null)) => {
                // consume the null marker
                let mut c = Bytes::new(bytes);
                decode::read_nil(&mut c).map_err(error_report)?;
                c.remaining_slice()
            }
            Err(err) => return Err(error_report(err)),
        };
    }

    if !bytes.is_empty() {
        bail!("trailing bytes in encoded history. malformed")
    }

    Ok(History {
        id: id.to_owned().into(),
        timestamp: OffsetDateTime::parse(timestamp, &Rfc3339)?,
        duration,
        exit,
        command: command.to_owned(),
        cwd: cwd.to_owned(),
        session: session.to_owned(),
        hostname: hostname.to_owned(),
        author: History::author_from_hostname(hostname),
        intent: None,
        deleted_at: deleted_at
            .map(|t| OffsetDateTime::parse(t, &Rfc3339))
            .transpose()?,
    })
}

fn error_report<E: std::fmt::Debug>(err: E) -> eyre::Report {
    eyre!("{err:?}")
}

