use std::future::Future;

pub(crate) trait Reusable {
    fn endpoint(&self) -> &str;
    fn is_closed(&self) -> bool;
    fn close(&mut self) -> impl Future<Output = ()> + Send;
}

/// Preserve the current link until a replacement succeeds, but close its
/// transport before opening another Colab connection. Healthy links are reused.
pub(crate) async fn ensure<Connection, Factory, Opening>(
    current: &mut Option<Connection>,
    endpoint: &str,
    create: Factory,
) -> Result<(), String>
where
    Connection: Reusable,
    Factory: FnOnce() -> Opening,
    Opening: Future<Output = Result<Connection, String>>,
{
    if current
        .as_ref()
        .is_some_and(|connection| connection.endpoint() == endpoint && !connection.is_closed())
    {
        return Ok(());
    }
    if let Some(connection) = current.as_mut() {
        connection.close().await;
    }
    *current = Some(create().await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Link {
        endpoint: String,
        closed: bool,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Reusable for Link {
        fn endpoint(&self) -> &str {
            &self.endpoint
        }
        fn is_closed(&self) -> bool {
            self.closed
        }
        async fn close(&mut self) {
            tokio::task::yield_now().await;
            self.closed = true;
            self.events.lock().unwrap().push("closed");
        }
    }

    #[tokio::test]
    async fn repeated_boot_or_reconnect_reuses_the_live_transport() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut current = None;
        for _attempt in 0..3 {
            ensure(&mut current, "existing-instance", || async {
                events.lock().unwrap().push("opened");
                Ok(Link {
                    endpoint: "existing-instance".into(),
                    closed: false,
                    events: events.clone(),
                })
            })
            .await
            .unwrap();
        }
        assert_eq!(*events.lock().unwrap(), vec!["opened"]);
    }

    #[tokio::test]
    async fn replacement_waits_for_the_previous_transport_to_close() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut current = Some(Link {
            endpoint: "old-instance".into(),
            closed: false,
            events: events.clone(),
        });
        ensure(&mut current, "current-instance", || async {
            assert_eq!(*events.lock().unwrap(), vec!["closed"]);
            events.lock().unwrap().push("opened");
            Ok(Link {
                endpoint: "current-instance".into(),
                closed: false,
                events: events.clone(),
            })
        })
        .await
        .unwrap();
        assert_eq!(current.unwrap().endpoint(), "current-instance");
        assert_eq!(*events.lock().unwrap(), vec!["closed", "opened"]);
    }

    #[tokio::test]
    async fn failed_reconnect_retains_the_old_connection_record_for_retry() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut current = Some(Link {
            endpoint: "existing-instance".into(),
            closed: true,
            events: events.clone(),
        });
        assert!(ensure(&mut current, "existing-instance", || async {
            Err("HTTP 429".into())
        })
        .await
        .is_err());
        assert_eq!(current.as_ref().unwrap().endpoint(), "existing-instance");
        ensure(&mut current, "existing-instance", || async {
            Ok(Link {
                endpoint: "existing-instance".into(),
                closed: false,
                events: events.clone(),
            })
        })
        .await
        .unwrap();
        assert!(!current.unwrap().is_closed());
    }
}
