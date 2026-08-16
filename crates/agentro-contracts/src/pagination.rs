use thiserror::Error;

/// Default number of items requested when an endpoint does not specify one.
pub const DEFAULT_PAGE_ITEMS: u32 = 100;
/// Absolute cross-system upper bound for one list response.
pub const MAX_PAGE_ITEMS: u32 = 1_000;

/// A validated item budget for one page allocation or query.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageBudget {
    limit: u32,
}

impl PageBudget {
    /// Resolves an optional request against an endpoint-specific maximum.
    ///
    /// The endpoint maximum itself must be between one and
    /// [`MAX_PAGE_ITEMS`]. An absent request uses [`DEFAULT_PAGE_ITEMS`],
    /// capped by the endpoint maximum.
    ///
    /// # Errors
    ///
    /// Returns [`PageBudgetError`] when either bound is zero or when the
    /// requested item count exceeds the endpoint maximum.
    pub fn resolve(requested: Option<u32>, endpoint_maximum: u32) -> Result<Self, PageBudgetError> {
        if endpoint_maximum == 0 || endpoint_maximum > MAX_PAGE_ITEMS {
            return Err(PageBudgetError::InvalidEndpointMaximum);
        }

        let limit = requested.unwrap_or(DEFAULT_PAGE_ITEMS.min(endpoint_maximum));
        if limit == 0 {
            return Err(PageBudgetError::ZeroRequested);
        }
        if limit > endpoint_maximum {
            return Err(PageBudgetError::ExceedsEndpointMaximum {
                requested: limit,
                endpoint_maximum,
            });
        }

        Ok(Self { limit })
    }

    /// Returns the validated item limit as a protocol-sized integer.
    #[must_use]
    pub const fn limit(self) -> u32 {
        self.limit
    }
}

/// A page item budget invariant violation.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PageBudgetError {
    /// The endpoint maximum is zero or exceeds [`MAX_PAGE_ITEMS`].
    #[error("endpoint page maximum is outside the global bounds")]
    InvalidEndpointMaximum,
    /// The caller explicitly requested zero items.
    #[error("requested page size must be non-zero")]
    ZeroRequested,
    /// The caller's request exceeds the endpoint maximum.
    #[error("requested page size {requested} exceeds endpoint maximum {endpoint_maximum}")]
    ExceedsEndpointMaximum {
        /// The caller-requested item count.
        requested: u32,
        /// The endpoint-specific hard maximum.
        endpoint_maximum: u32,
    },
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{DEFAULT_PAGE_ITEMS, PageBudget, PageBudgetError};

    proptest! {
        #[test]
        fn accepted_page_budget_never_exceeds_endpoint(
            maximum in 1_u32..=1_000,
            requested in 1_u32..=1_000
        ) {
            let result = PageBudget::resolve(Some(requested), maximum);
            if requested <= maximum {
                prop_assert_eq!(result.map(PageBudget::limit), Ok(requested));
            } else {
                prop_assert_eq!(result, Err(PageBudgetError::ExceedsEndpointMaximum {
                    requested,
                    endpoint_maximum: maximum,
                }));
            }
        }
    }

    #[test]
    fn absent_page_size_uses_bounded_default() -> Result<(), PageBudgetError> {
        assert_eq!(PageBudget::resolve(None, 25)?.limit(), 25);
        assert_eq!(
            PageBudget::resolve(None, DEFAULT_PAGE_ITEMS + 1)?.limit(),
            DEFAULT_PAGE_ITEMS
        );
        Ok(())
    }
}
