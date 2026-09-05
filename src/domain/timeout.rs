use crate::domain::errors::{PreviewError, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub(crate) struct RequestDeadline {
    inner: Arc<DeadlineInner>,
}

#[derive(Debug)]
struct DeadlineInner {
    started: Instant,
    timeout: Duration,
    format: String,
    cancelled: AtomicBool,
}

impl RequestDeadline {
    pub(crate) fn new(started: Instant, format: impl Into<String>, timeout: Duration) -> Self {
        Self {
            inner: Arc::new(DeadlineInner {
                started,
                timeout,
                format: format.into().to_ascii_uppercase(),
                cancelled: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.inner.cancelled.load(Ordering::Relaxed)
            || self.inner.started.elapsed() >= self.inner.timeout
        {
            self.inner.cancelled.store(true, Ordering::Relaxed);
            return Err(self.error());
        }
        Ok(())
    }

    pub(crate) fn remaining(&self) -> Result<Duration> {
        self.check()?;
        Ok(self
            .inner
            .timeout
            .saturating_sub(self.inner.started.elapsed()))
    }

    pub(crate) fn cap(&self, duration: Duration) -> Result<Duration> {
        Ok(duration.min(self.remaining()?))
    }

    pub(crate) fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Relaxed);
    }

    fn error(&self) -> PreviewError {
        PreviewError::render(format!(
            "{} preview request timed out after {}s",
            self.inner.format,
            self.inner.timeout.as_secs()
        ))
    }
}

#[cfg(test)]
impl RequestDeadline {
    pub(crate) fn configured_timeout(&self) -> Duration {
        self.inner.timeout
    }

    pub(crate) fn format(&self) -> &str {
        &self.inner.format
    }
}

#[cfg(test)]
mod tests {
    use super::RequestDeadline;
    use std::time::{Duration, Instant};

    #[test]
    fn expired_deadline_has_stable_render_error() {
        let deadline = RequestDeadline::new(
            Instant::now() - Duration::from_secs(2),
            "gif",
            Duration::from_secs(1),
        );
        let error = deadline.check().unwrap_err();
        assert_eq!(
            error.to_string(),
            "render error: GIF preview request timed out after 1s"
        );
    }

    #[test]
    fn cancellation_uses_same_timeout_error() {
        let deadline = RequestDeadline::new(Instant::now(), "png", Duration::from_secs(300));
        deadline.cancel();
        assert_eq!(
            deadline.check().unwrap_err().to_string(),
            "render error: PNG preview request timed out after 300s"
        );
    }
}
