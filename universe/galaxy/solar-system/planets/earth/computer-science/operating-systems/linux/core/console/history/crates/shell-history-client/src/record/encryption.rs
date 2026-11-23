use shell_history_common::record::{
    AdditionalData, DecryptedData, EncryptedData, Encryption, HostId, RecordId, RecordIdx,
};
use base64::{Engine, engine::general_purpose};
use eyre::{Context, Result, ensure};
use rusty_paserk::{Key, KeyId, Local, PieWrappedKey};
use rusty_paseto::core::{
    ImplicitAssertion, Key as DataKey, Local as LocalPurpose, Paseto, PasetoNonce, Payload, V4,
};
use serde::{Deserialize, Serialize};

/// Use PASETO V4 Local encryption using the additional data as an implicit assertion.
#[allow(non_camel_case_types)]
pub struct PASETO_V4;

/*
Why do we use a random content-encryption key?
Originally I was planning on using a derived key for encryption based on additional data.
This would be a lot more secure than using the master key directly.

However, there's an established norm of using a random key. This scheme might be otherwise known as
- client-side encryption
- envelope encryption
- key wrapping

A HSM (Hardware Security Module) provider, eg: AWS, Azure, GCP, or even a physical device like a YubiKey
will have some keys that they keep to themselves. These keys never leave their physical hardware.
If they never leave the hardware, then encrypting large amounts of data means giving them the data and waiting.
This is not a practical solution. Instead, generate a unique key for your data, encrypt that using your HSM
and then store that with your data.

See
 - <https://docs.aws.amazon.com/wellarchitected/latest/financial-services-industry-lens/use-envelope-encryption-with-customer-master-keys.html>
 - <https://cloud.google.com/kms/docs/envelope-encryption>
 - <https://learn.microsoft.com/en-us/azure/storage/blobs/client-side-encryption?tabs=dotnet#encryption-and-decryption-via-the-envelope-technique>
 - <https://www.yubico.com/gb/product/yubihsm-2-fips/>
 - <https://cheatsheetseries.owasp.org/cheatsheets/Cryptographic_Storage_Cheat_Sheet.html#encrypting-stored-keys>

Why would we care? In the past we have received some requests for company solutions. If in future we can configure a
KMS service with little effort, then that would solve a lot of issues for their security team.

Even for personal use, if a user is not comfortable with sharing keys between hosts,
GCP HSM costs $1/month and $0.03 per 10,000 key operations. Assuming an active user runs
1000 shell_history records a day, that would only cost them $1 and 10 cent a month.

Additionally, key rotations are much simpler using this scheme. Rotating a key is as simple as re-encrypting the CEK, and not the message contents.
This makes it very fast to rotate a key in bulk.

For future reference, with asymmetric encryption, you can encrypt the CEK without the HSM's involvement, but decrypting
will need the HSM. This allows the encryption path to still be extremely fast (no network calls) but downloads/decryption
that happens in the background can make the network calls to the HSM
*/

impl Encryption for PASETO_V4 {
    fn re_encrypt(
        mut data: EncryptedData,
        _ad: AdditionalData,
        old_key: &[u8; 32],
        new_key: &[u8; 32],
    ) -> Result<EncryptedData> {
        let cek = Self::decrypt_cek(data.content_encryption_key, old_key)?;
        data.content_encryption_key = Self::encrypt_cek(cek, new_key);
        Ok(data)
    }

    fn encrypt(data: DecryptedData, ad: AdditionalData, key: &[u8; 32]) -> EncryptedData {
        // generate a random key for this entry
        // aka content-encryption-key (CEK)
        let random_key = Key::<V4, Local>::new_os_random();

        // encode the implicit assertions
        let assertions = Assertions::from(ad).encode();

        // build the payload and encrypt the token
        let payload = serde_json::to_string(&RunShellHistoryPayload {
            data: general_purpose::URL_SAFE_NO_PAD.encode(data.0),
        })
        .expect("json encoding can't fail");
        let nonce = DataKey::<32>::try_new_random().expect("could not source from random");
        let nonce = PasetoNonce::<V4, LocalPurpose>::from(&nonce);

        let token = Paseto::<V4, LocalPurpose>::builder()
            .set_payload(Payload::from(payload.as_str()))
            .set_implicit_assertion(ImplicitAssertion::from(assertions.as_str()))
            .try_encrypt(&random_key.into(), &nonce)
            .expect("error encrypting console history data");

        EncryptedData {
            data: token,
            content_encryption_key: Self::encrypt_cek(random_key, key),
        }
    }

    fn decrypt(data: EncryptedData, ad: AdditionalData, key: &[u8; 32]) -> Result<DecryptedData> {
        let token = data.data;
        let cek = Self::decrypt_cek(data.content_encryption_key, key)?;

        // encode the implicit assertions
        let assertions = Assertions::from(ad).encode();

        // decrypt the payload with the footer and implicit assertions
        let payload = Paseto::<V4, LocalPurpose>::try_decrypt(
            &token,
            &cek.into(),
            None,
            ImplicitAssertion::from(&*assertions),
        )
        .context("could not decrypt entry")?;

        let payload: RunShellHistoryPayload = serde_json::from_str(&payload)?;
        let data = general_purpose::URL_SAFE_NO_PAD.decode(payload.data)?;
        Ok(DecryptedData(data))
    }
}

impl PASETO_V4 {
    fn decrypt_cek(wrapped_cek: String, key: &[u8; 32]) -> Result<Key<V4, Local>> {
        let wrapping_key = Key::<V4, Local>::from_bytes(*key);

        // let wrapping_key = PasetoSymmetricKey::from(Key::from(key));

        let RunShellHistoryFooter { kid, wpk } = serde_json::from_str(&wrapped_cek)
            .context("wrapped cek did not contain the correct contents")?;

        // check that the wrapping key matches the required key to decrypt.
        // In future, we could support multiple keys and use this key to
        // look up the key rather than only allow one key.
        // For now though we will only support the one key and key rotation will
        // have to be a hard reset
        let current_kid = wrapping_key.to_id();

        ensure!(
            current_kid == kid,
            "attempting to decrypt with incorrect key. currently using {current_kid}, expecting {kid}"
        );

        // decrypt the random key
        Ok(wpk.unwrap_key(&wrapping_key)?)
    }

    fn encrypt_cek(cek: Key<V4, Local>, key: &[u8; 32]) -> String {
        // aka key-encryption-key (KEK)
        let wrapping_key = Key::<V4, Local>::from_bytes(*key);

        // wrap the random key so we can decrypt it later
        let wrapped_cek = RunShellHistoryFooter {
            wpk: cek.wrap_pie(&wrapping_key),
            kid: wrapping_key.to_id(),
        };
        serde_json::to_string(&wrapped_cek).expect("could not serialize wrapped cek")
    }
}

#[derive(Serialize, Deserialize)]
struct RunShellHistoryPayload {
    data: String,
}

#[derive(Serialize, Deserialize)]
/// Well-known footer claims for decrypting. This is not encrypted but is stored in the record.
/// <https://github.com/paseto-standard/paseto-spec/blob/master/docs/02-Implementation-Guide/04-Claims.md#optional-footer-claims>
struct RunShellHistoryFooter {
    /// Wrapped key
    wpk: PieWrappedKey<V4, Local>,
    /// ID of the key which was used to wrap
    kid: KeyId<V4, Local>,
}

/// Used in the implicit assertions. This is not encrypted and not stored in the data blob.
// This cannot be changed, otherwise it breaks the authenticated encryption.
#[derive(Debug, Copy, Clone, Serialize)]
struct Assertions<'a> {
    id: &'a RecordId,
    idx: &'a RecordIdx,
    version: &'a str,
    tag: &'a str,
    host: &'a HostId,
}

impl<'a> From<AdditionalData<'a>> for Assertions<'a> {
    fn from(ad: AdditionalData<'a>) -> Self {
        Self {
            id: ad.id,
            version: ad.version,
            tag: ad.tag,
            host: ad.host,
            idx: ad.idx,
        }
    }
}

impl Assertions<'_> {
    fn encode(&self) -> String {
        serde_json::to_string(self).expect("could not serialize implicit assertions")
    }
}

