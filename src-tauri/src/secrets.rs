//! Secrets live in the Windows Credential Manager, never in the database.
//!
//! One credential entry only holds so much text, and a Microsoft refresh token
//! does not fit in one. A secret that is too long is split across numbered
//! entries; callers never see the difference.

use crate::error::{Result, SkimError};

const SERVICE: &str = "Skim";

/// `ERROR_NOT_ENOUGH_MEMORY`. Credential Manager answers a write with this when
/// the user's vault won't take another entry — its size cap is shared with every
/// other app's stored credentials — not when the machine is short on memory.
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;

/// Credential Manager caps one credential blob at 2560 bytes, and a password is
/// stored as UTF-16 — so a single entry holds 1280 code units, no more.
const WINDOWS_LIMIT_UNITS: usize = 1280;

/// What we actually put in one entry. The margin costs nothing — a refresh token
/// needs the same number of pieces either way — and covers a terminator the
/// platform might count and we have no way to ask about.
const CHUNK_UNITS: usize = 1200;

/// Past this a string is not a credential. Refusing it keeps us from carpeting
/// the user's vault, and it bounds the sweep `delete` has to do.
const MAX_CHUNKS: usize = 8;

const _: () = assert!(CHUNK_UNITS >= 2 && CHUNK_UNITS <= WINDOWS_LIMIT_UNITS);

/// Prefix of the pointer left in the main entry when a secret had to be split.
/// The leading U+0001 cannot be typed into a form and appears in no OAuth token
/// or API key, so a real secret can never be mistaken for a pointer.
const MARKER: &str = "\u{1}skim-chunks-v1:";

/// The Win32 code behind a keyring failure, when there is one.
fn win32_code(e: &keyring::Error) -> Option<u32> {
    match e {
        keyring::Error::PlatformFailure(inner) | keyring::Error::NoStorageAccess(inner) => {
            inner.downcast_ref::<keyring::windows::Error>().map(|w| w.0)
        }
        _ => None,
    }
}

/// Give the UI a stable code to explain the failure by, and keep the raw Windows
/// text as the detail — it is what a bug report needs.
fn store_err(e: keyring::Error) -> SkimError {
    let code = if win32_code(&e) == Some(ERROR_NOT_ENOUGH_MEMORY) {
        "secrets_full"
    } else if matches!(e, keyring::Error::NoStorageAccess(_)) {
        "secrets_unavailable"
    } else {
        "secrets"
    };
    tracing::error!(error = %e, code, "credential store failed");
    SkimError::other(code, format!("credential store: {e}"))
}

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(store_err)
}

/// One credential entry, addressed by key. Everything above this line knows
/// about Windows; everything below it only knows about entries.
trait Vault {
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<String>>;
    /// Removing what isn't there is a success, not an error.
    fn delete(&self, key: &str) -> Result<()>;
}

struct Keyring;

impl Vault for Keyring {
    fn set(&self, key: &str, value: &str) -> Result<()> {
        entry(key)?.set_password(value).map_err(store_err)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        match entry(key)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
    }
}

/// Which set of chunk entries is live. A write always fills the *other* one, so
/// writing the marker is a single atomic commit: a crash before it leaves the
/// previous secret whole, instead of splicing half of each together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Generation {
    A,
    B,
}

const GENERATIONS: [Generation; 2] = [Generation::A, Generation::B];

impl Generation {
    fn other(self) -> Self {
        match self {
            Generation::A => Generation::B,
            Generation::B => Generation::A,
        }
    }

    fn tag(self) -> char {
        match self {
            Generation::A => 'a',
            Generation::B => 'b',
        }
    }

    fn from_tag(c: char) -> Option<Self> {
        match c {
            'a' => Some(Generation::A),
            'b' => Some(Generation::B),
            _ => None,
        }
    }
}

/// What the main entry turned out to hold.
enum Stored {
    Plain(String),
    Chunked(Generation, usize),
    /// A pointer we cannot follow. Only reachable if someone edited the vault by
    /// hand; treated as nothing stored.
    Broken,
}

fn classify(value: String) -> Stored {
    let Some(rest) = value.strip_prefix(MARKER) else {
        return Stored::Plain(value);
    };
    let parsed = rest.split_once(':').and_then(|(tag, count)| {
        let mut tag = tag.chars();
        let generation = tag.next().and_then(Generation::from_tag)?;
        if tag.next().is_some() {
            return None;
        }
        let n = count.parse::<usize>().ok()?;
        (1..=MAX_CHUNKS).contains(&n).then_some((generation, n))
    });
    match parsed {
        Some((generation, n)) => Stored::Chunked(generation, n),
        None => Stored::Broken,
    }
}

fn marker(generation: Generation, n: usize) -> String {
    format!("{MARKER}{}:{n}", generation.tag())
}

/// `#` appears in no account id (a UUID) and in none of the AI key names, so a
/// chunk can never land on a key that means something else.
fn chunk_key(key: &str, generation: Generation, i: usize) -> String {
    format!("{key}#{}{i}", generation.tag())
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Pieces of at most `units` UTF-16 code units, never cutting a `char` in half.
/// Walking `char_indices` is what keeps a surrogate pair together: a non-BMP
/// char contributes both of its units in one step, so a piece cannot end between
/// them. `units >= 2` is what guarantees no piece comes out empty.
fn split_utf16(secret: &str, units: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut used) = (0usize, 0usize);
    for (i, c) in secret.char_indices() {
        let w = c.len_utf16();
        if used + w > units {
            parts.push(&secret[start..i]);
            start = i;
            used = 0;
        }
        used += w;
    }
    parts.push(&secret[start..]);
    parts
}

fn set_in(vault: &dyn Vault, key: &str, secret: &str) -> Result<()> {
    // Only used to pick a generation that is safe to overwrite. A read failure
    // is not fatal — a store we cannot read is one we should still be able to
    // replace — we just lose the crash-safety of this one write.
    let live = match vault.get(key) {
        Ok(Some(value)) => match classify(value) {
            Stored::Chunked(generation, _) => Some(generation),
            Stored::Plain(_) | Stored::Broken => None,
        },
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "cannot read the stored credential before replacing it");
            None
        }
    };
    let next = live.map_or(Generation::A, Generation::other);

    let written = if utf16_len(secret) <= CHUNK_UNITS {
        vault.set(key, secret)?;
        0
    } else {
        let parts = split_utf16(secret, CHUNK_UNITS);
        if parts.len() > MAX_CHUNKS {
            return Err(SkimError::other(
                "secrets",
                "credential store: this secret is too large to store",
            ));
        }
        // Dead space until the marker lands: nothing reads these yet, and `key`
        // still names the previous value with its own pieces untouched. A
        // failure here has to return before the marker is written.
        for (i, part) in parts.iter().enumerate() {
            vault.set(&chunk_key(key, next, i + 1), part)?;
        }
        vault.set(key, &marker(next, parts.len()))?; // the commit
        parts.len()
    };

    // Hygiene from here on. The secret is stored; failing now would make
    // `add_account` tear down an account that actually works. Sweep a fixed
    // range rather than an old marker's: an interrupted write leaves pieces no
    // marker describes.
    for generation in GENERATIONS {
        let from = if generation == next { written + 1 } else { 1 };
        for i in from..=MAX_CHUNKS {
            if let Err(e) = vault.delete(&chunk_key(key, generation, i)) {
                tracing::warn!(error = %e, "cannot clear a stale credential piece");
            }
        }
    }
    Ok(())
}

fn get_in(vault: &dyn Vault, key: &str) -> Result<Option<String>> {
    let Some(value) = vault.get(key)? else {
        return Ok(None);
    };
    let (generation, n) = match classify(value) {
        Stored::Plain(secret) => return Ok(Some(secret)),
        Stored::Chunked(generation, n) => (generation, n),
        Stored::Broken => {
            tracing::error!("a stored credential points at pieces we cannot make sense of");
            return Ok(None);
        }
    };
    let mut secret = String::new();
    for i in 1..=n {
        let Some(part) = vault.get(&chunk_key(key, generation, i))? else {
            // Half a token would reach the server as a baffling wrong password.
            // No credential at all is the honest answer, and every caller
            // already knows how to ask for a fresh sign-in.
            tracing::error!(piece = i, of = n, "a stored credential is incomplete");
            return Ok(None);
        };
        secret.push_str(&part);
    }
    Ok(Some(secret))
}

fn delete_in(vault: &dyn Vault, key: &str) -> Result<()> {
    let mut failure = vault.delete(key).err();
    // Unconditional, because the marker may be missing or half-written and the
    // one thing this must guarantee is that no fragment of a token outlives the
    // account. Try every entry before giving up, so one bad one cannot strand
    // the rest.
    for generation in GENERATIONS {
        for i in 1..=MAX_CHUNKS {
            if let Err(e) = vault.delete(&chunk_key(key, generation, i)) {
                failure.get_or_insert(e);
            }
        }
    }
    match failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

pub fn set(account: &str, secret: &str) -> Result<()> {
    set_in(&Keyring, account, secret)
}

pub fn get(account: &str) -> Result<Option<String>> {
    get_in(&Keyring, account)
}

pub fn delete(account: &str) -> Result<()> {
    delete_in(&Keyring, account)
}

/// Key under which the mail credential for an account is stored. Holds the
/// password for `auth_kind = 'password'` or the OAuth refresh token for
/// `auth_kind = 'oauth'`.
pub fn mail_key(account_id: &str) -> String {
    format!("mail:{account_id}")
}

pub const ANTHROPIC_KEY: &str = "anthropic_api_key";
pub const OPENROUTER_KEY: &str = "openrouter_api_key";
/// Optional key for the user-supplied OpenAI-compatible endpoint.
pub const CUSTOM_KEY: &str = "custom_api_key";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    const KEY: &str = "mail:11111111-2222-3333-4444-555555555555";

    fn platform_failure(code: u32) -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(keyring::windows::Error(code)))
    }

    /// A stand-in for the Credential Manager. keyring's own mock cannot serve as
    /// one: it keeps state inside each `Entry`, so two keys never see each other.
    #[derive(Default)]
    struct MemVault {
        data: Mutex<BTreeMap<String, String>>,
        /// Writes left before `set` starts failing. `None` means never.
        writes_left: Mutex<Option<usize>>,
    }

    impl MemVault {
        fn fail_after(&self, writes: usize) {
            *self.writes_left.lock().unwrap() = Some(writes);
        }

        fn keys(&self) -> Vec<String> {
            self.data.lock().unwrap().keys().cloned().collect()
        }

        fn raw(&self, key: &str) -> Option<String> {
            self.data.lock().unwrap().get(key).cloned()
        }

        fn seed(&self, key: &str, value: &str) {
            self.data
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
        }

        fn forget(&self, key: &str) {
            self.data.lock().unwrap().remove(key);
        }

        fn pieces(&self) -> Vec<String> {
            let prefix = format!("{KEY}#");
            self.keys()
                .into_iter()
                .filter(|k| k.starts_with(&prefix))
                .collect()
        }
    }

    impl Vault for MemVault {
        fn set(&self, key: &str, value: &str) -> Result<()> {
            let mut left = self.writes_left.lock().unwrap();
            if let Some(n) = left.as_mut() {
                if *n == 0 {
                    return Err(SkimError::other("secrets", "credential store: refused"));
                }
                *n -= 1;
            }
            // Every write goes through here, so every chunking test doubles as a
            // test that we stay inside what Windows accepts.
            assert!(
                utf16_len(value) <= WINDOWS_LIMIT_UNITS,
                "a value Windows would refuse: {} units",
                utf16_len(value)
            );
            self.data
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.raw(key))
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.forget(key);
            Ok(())
        }
    }

    fn live_generation(vault: &MemVault) -> Generation {
        match classify(vault.raw(KEY).expect("the main entry")) {
            Stored::Chunked(generation, _) => generation,
            _ => panic!("expected a split secret"),
        }
    }

    #[test]
    fn a_full_vault_is_reported_as_such() {
        let err = store_err(platform_failure(ERROR_NOT_ENOUGH_MEMORY));
        assert_eq!(err.code(), "secrets_full");
        // The Windows code stays in the message, for bug reports.
        assert!(err.to_string().contains("Windows error code 8"));
    }

    #[test]
    fn other_failures_keep_their_own_codes() {
        assert_eq!(store_err(platform_failure(1)).code(), "secrets");
        assert_eq!(
            store_err(keyring::Error::NoStorageAccess(Box::new(
                keyring::windows::Error(1312)
            )))
            .code(),
            "secrets_unavailable"
        );
    }

    #[test]
    fn short_secrets_are_stored_verbatim_in_one_entry() {
        let vault = MemVault::default();
        let secret = "s".repeat(100);
        set_in(&vault, KEY, &secret).unwrap();
        assert_eq!(vault.keys(), vec![KEY.to_string()]);
        assert_eq!(vault.raw(KEY).as_deref(), Some(secret.as_str()));
        assert_eq!(get_in(&vault, KEY).unwrap(), Some(secret));
    }

    #[test]
    fn secrets_written_before_chunking_still_read_back() {
        let vault = MemVault::default();
        vault.seed(KEY, "hunter2");
        assert_eq!(get_in(&vault, KEY).unwrap().as_deref(), Some("hunter2"));
    }

    #[test]
    fn a_long_secret_is_split_and_reassembled() {
        let vault = MemVault::default();
        let secret = "t".repeat(4000);
        set_in(&vault, KEY, &secret).unwrap();
        assert!(vault.raw(KEY).unwrap().starts_with(MARKER));
        assert_eq!(vault.pieces().len(), 4);
        assert_eq!(get_in(&vault, KEY).unwrap(), Some(secret));
    }

    #[test]
    fn chunks_never_split_a_character() {
        let secret = "aй漢🙂".repeat(50);
        for units in [2usize, 3, 5, 7, 1200] {
            let parts = split_utf16(&secret, units);
            assert_eq!(parts.concat(), secret);
            assert!(parts.iter().all(|p| !p.is_empty()));
            assert!(parts.iter().all(|p| utf16_len(p) <= units));
        }
    }

    #[test]
    fn every_chunk_fits_the_windows_limit() {
        for sample in ["a", "й", "漢", "🙂"] {
            for len in [1199usize, 1200, 1201, 2400, 5000] {
                let secret = sample.repeat(len);
                for part in split_utf16(&secret, CHUNK_UNITS) {
                    assert!(utf16_len(part) <= WINDOWS_LIMIT_UNITS);
                }
            }
        }
    }

    #[test]
    fn a_secret_too_large_for_the_vault_is_refused_cleanly() {
        let vault = MemVault::default();
        let secret = "x".repeat(CHUNK_UNITS * MAX_CHUNKS + 1);
        assert_eq!(set_in(&vault, KEY, &secret).unwrap_err().code(), "secrets");
        assert!(vault.keys().is_empty());
    }

    #[test]
    fn shrinking_to_a_short_secret_leaves_no_stale_chunks() {
        let vault = MemVault::default();
        set_in(&vault, KEY, &"a".repeat(5000)).unwrap();
        set_in(&vault, KEY, "short").unwrap();
        assert_eq!(vault.keys(), vec![KEY.to_string()]);
        assert_eq!(get_in(&vault, KEY).unwrap().as_deref(), Some("short"));
    }

    #[test]
    fn shrinking_to_fewer_chunks_leaves_no_stale_chunks() {
        let vault = MemVault::default();
        set_in(&vault, KEY, &"a".repeat(5000)).unwrap();
        let shorter = "b".repeat(2500);
        set_in(&vault, KEY, &shorter).unwrap();
        assert_eq!(vault.pieces().len(), 3);
        assert_eq!(get_in(&vault, KEY).unwrap(), Some(shorter));
    }

    #[test]
    fn each_write_uses_the_other_generation() {
        let vault = MemVault::default();
        let secret = "a".repeat(3000);
        let mut tags = Vec::new();
        for _ in 0..3 {
            set_in(&vault, KEY, &secret).unwrap();
            tags.push(live_generation(&vault).tag());
            // Only the live generation is left standing.
            assert_eq!(vault.pieces().len(), 3);
        }
        assert_eq!(tags, vec!['a', 'b', 'a']);
    }

    #[test]
    fn a_failed_chunk_write_keeps_the_previous_secret() {
        let vault = MemVault::default();
        let first = "a".repeat(3000);
        set_in(&vault, KEY, &first).unwrap();
        vault.fail_after(1); // dies on the second piece
        assert!(set_in(&vault, KEY, &"b".repeat(3000)).is_err());
        assert_eq!(get_in(&vault, KEY).unwrap(), Some(first));
    }

    #[test]
    fn a_crash_before_the_marker_keeps_the_previous_secret() {
        let vault = MemVault::default();
        let first = "a".repeat(3000);
        set_in(&vault, KEY, &first).unwrap();
        vault.fail_after(3); // all three pieces land, the commit does not
        assert!(set_in(&vault, KEY, &"b".repeat(3000)).is_err());
        assert_eq!(get_in(&vault, KEY).unwrap(), Some(first));
    }

    #[test]
    fn delete_removes_the_marker_and_every_chunk() {
        let vault = MemVault::default();
        set_in(&vault, KEY, &"a".repeat(5000)).unwrap();
        delete_in(&vault, KEY).unwrap();
        assert!(vault.keys().is_empty());
    }

    #[test]
    fn delete_sweeps_chunks_even_without_a_marker() {
        let vault = MemVault::default();
        set_in(&vault, KEY, &"a".repeat(5000)).unwrap();
        vault.forget(KEY);
        delete_in(&vault, KEY).unwrap();
        assert!(vault.keys().is_empty());
    }

    #[test]
    fn a_missing_chunk_is_not_a_wrong_password() {
        let vault = MemVault::default();
        set_in(&vault, KEY, &"a".repeat(5000)).unwrap();
        vault.forget(&chunk_key(KEY, live_generation(&vault), 2));
        assert_eq!(get_in(&vault, KEY).unwrap(), None);
    }

    #[test]
    fn a_plain_value_is_never_mistaken_for_a_marker() {
        assert!(matches!(classify("hunter2".into()), Stored::Plain(_)));
        // The readable half of the marker, typed in as a password.
        assert!(matches!(
            classify("skim-chunks-v1:a:3".into()),
            Stored::Plain(_)
        ));
        assert!(matches!(
            classify(marker(Generation::B, 3)),
            Stored::Chunked(Generation::B, 3)
        ));
        for bad in ["c:3", "a:0", "a:99", "aa:3", "a", "a:x"] {
            assert!(
                matches!(classify(format!("{MARKER}{bad}")), Stored::Broken),
                "{bad} should not parse"
            );
        }
    }
}
