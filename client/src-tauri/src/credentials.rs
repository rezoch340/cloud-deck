use std::sync::Mutex;

/// Cache both success and refusal: background operations must not repeat OS prompts.
#[derive(Default)]
pub struct CredentialCache {
    value: Option<Result<Option<String>, String>>,
}

impl CredentialCache {
    pub fn read(
        &mut self,
        fetch: impl FnOnce() -> Result<Option<String>, String>,
    ) -> Result<Option<String>, String> {
        self.value.get_or_insert_with(fetch).clone()
    }

    pub fn replace(&mut self, value: Option<String>) {
        self.value = Some(Ok(value));
    }

    pub fn retry(&mut self) {
        if matches!(self.value, Some(Err(_))) {
            self.value = None;
        }
    }
}

pub static SESSION: Mutex<CredentialCache> = Mutex::new(CredentialCache { value: None });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_operations_and_refusals_read_the_store_once() {
        for response in [Ok(Some("session".into())), Ok(None), Err("denied".into())] {
            let mut cache = CredentialCache::default();
            let mut reads = 0;
            for _attempt in 0..8 {
                assert_eq!(
                    cache.read(|| {
                        reads += 1;
                        response.clone()
                    }),
                    response
                );
            }
            assert_eq!(reads, 1);
        }
    }

    #[test]
    fn explicit_retry_and_account_changes_replace_cached_credentials() {
        let mut cache = CredentialCache::default();
        assert!(cache.read(|| Err("denied".into())).is_err());
        cache.retry();
        assert_eq!(
            cache.read(|| Ok(Some("first".into()))).unwrap(),
            Some("first".into())
        );
        cache.replace(Some("second".into()));
        assert_eq!(
            cache.read(|| panic!("Unexpected keychain read")).unwrap(),
            Some("second".into())
        );
        cache.replace(None);
        assert_eq!(
            cache
                .read(|| panic!("Signed out session was reloaded"))
                .unwrap(),
            None
        );
    }
}
