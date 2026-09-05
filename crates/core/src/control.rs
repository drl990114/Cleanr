use std::{error::Error, fmt};

/// Cancellation is control flow, separate from policy and plan validation failures.
#[derive(Debug)]
pub enum WorkError<E> {
    Cancelled,
    Failed(E),
}

impl<E: fmt::Display> fmt::Display for WorkError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("operation cancelled"),
            Self::Failed(error) => error.fmt(formatter),
        }
    }
}

impl<E: Error + 'static> Error for WorkError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cancelled => None,
            Self::Failed(error) => Some(error),
        }
    }
}

pub(crate) fn check_work<E>(cancelled: &dyn Fn() -> bool) -> Result<(), WorkError<E>> {
    if cancelled() {
        Err(WorkError::Cancelled)
    } else {
        Ok(())
    }
}

/// Only used by compatibility wrappers that pass a constant-false cancellation predicate.
pub(crate) fn uninterrupted<T, E>(result: Result<T, WorkError<E>>) -> Result<T, E> {
    match result {
        Ok(value) => Ok(value),
        Err(WorkError::Failed(error)) => Err(error),
        Err(WorkError::Cancelled) => unreachable!("constant-false cancellation predicate"),
    }
}
