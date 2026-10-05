use super::{Delivery, DeliveryError, Provider, Published, ReviewState, deliver};
use std::{
    future::Future,
    io,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Failure {
    None,
    Find,
    Push,
    Create,
}

struct Fake {
    calls: Mutex<Vec<&'static str>>,
    found: AtomicBool,
    mismatch: bool,
    failure: Failure,
}
impl Provider for Fake {
    type Error = io::Error;
    async fn find(&self, _: &Delivery) -> io::Result<Option<Published>> {
        self.calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?
            .push("find");
        if self.failure == Failure::Find {
            return Err(io::Error::other("uncertain observation"));
        }
        Ok(self
            .found
            .load(Ordering::Relaxed)
            .then(|| published(self.mismatch)))
    }
    async fn push(&self, _: &Delivery) -> io::Result<()> {
        self.calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?
            .push("push");
        if self.failure == Failure::Push {
            return Err(io::Error::other("push failed"));
        }
        Ok(())
    }
    async fn create(&self, _: &Delivery) -> io::Result<Published> {
        self.calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?
            .push("create");
        self.found.store(true, Ordering::Relaxed);
        if self.failure == Failure::Create {
            return Err(io::Error::other("uncertain create response"));
        }
        Ok(published(false))
    }
}
fn published(mismatch: bool) -> Published {
    Published {
        number: 7,
        url: "https://provider/review/7".into(),
        state: ReviewState::Draft,
        head_revision: if mismatch { "other" } else { "head" }.into(),
        branch: "branch".into(),
        base_branch: "main".into(),
    }
}
fn request() -> Delivery {
    Delivery {
        branch: "branch".into(),
        base_branch: "main".into(),
        base_revision: "base".into(),
        head_revision: "head".into(),
        title: "title".into(),
        body: "body".into(),
    }
}
struct NoWake;
impl Wake for NoWake {
    fn wake(self: Arc<Self>) {}
}
fn ready<T>(future: impl Future<Output = T>) -> io::Result<T> {
    let waker = Waker::from(Arc::new(NoWake));
    match pin!(future).as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(value) => Ok(value),
        Poll::Pending => Err(io::Error::other("fixture unexpectedly suspended")),
    }
}
#[test]
fn existing_publication_is_observed_without_new_writes() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(true),
        mismatch: false,
        failure: Failure::None,
    };
    assert_eq!(
        ready(deliver(&provider, &request()))?
            .map_err(io::Error::other)?
            .number,
        7
    );
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find"]
    );
    Ok(())
}
#[test]
fn new_publication_pushes_only_after_confirmed_absence() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(false),
        mismatch: false,
        failure: Failure::None,
    };
    ready(deliver(&provider, &request()))?.map_err(io::Error::other)?;
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find", "push", "create"]
    );
    Ok(())
}
#[test]
fn uncertain_observation_never_authorizes_new_mutations() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(false),
        mismatch: false,
        failure: Failure::Find,
    };
    assert!(matches!(
        ready(deliver(&provider, &request()))?,
        Err(DeliveryError::Provider(_))
    ));
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find"]
    );
    Ok(())
}
#[test]
fn mismatched_existing_identity_cannot_complete_publication() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(true),
        mismatch: true,
        failure: Failure::None,
    };
    assert!(matches!(
        ready(deliver(&provider, &request()))?,
        Err(DeliveryError::IdentityConflict)
    ));
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find"]
    );
    Ok(())
}

#[test]
fn failed_push_does_not_create_a_review() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(false),
        mismatch: false,
        failure: Failure::Push,
    };
    assert!(matches!(
        ready(deliver(&provider, &request()))?,
        Err(DeliveryError::Provider(_))
    ));
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find", "push"]
    );
    Ok(())
}

#[test]
fn uncertain_create_is_recovered_without_repeating_provider_writes() -> io::Result<()> {
    let provider = Fake {
        calls: Mutex::new(Vec::new()),
        found: AtomicBool::new(false),
        mismatch: false,
        failure: Failure::Create,
    };
    assert!(matches!(
        ready(deliver(&provider, &request()))?,
        Err(DeliveryError::Provider(_))
    ));
    assert_eq!(
        ready(deliver(&provider, &request()))?
            .map_err(io::Error::other)?
            .number,
        7
    );
    assert_eq!(
        *provider
            .calls
            .lock()
            .map_err(|_| io::Error::other("fixture lock poisoned"))?,
        vec!["find", "push", "create", "find"]
    );
    Ok(())
}
