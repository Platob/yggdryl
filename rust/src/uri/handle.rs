//! A [`Uri`] as a handle: the storage it names, resolved on first use.
//!
//! Every operation forwards to the one [`Holder`] the identifier resolves to
//! through [`Holder::from_url`], the dispatcher every location reaches its
//! backend through, which reads the identifier as it is and locates it
//! ([`Uri::locator`]) itself, so a URL, a URN and an ARN open exactly what
//! the same identifier opens anywhere else. The resolved handle is
//! kept in the value, so a staged write, an open scope and a cached answer
//! live as long as the `Uri` does; a clone starts unresolved, as its
//! rendering does. A resolution that fails is stored nowhere: it is the error
//! of every operation that returns one, and the empty answer of every
//! accessor that cannot.

use std::sync::OnceLock;

use crate::holder::Holder;
use crate::{IOBase, IOMedia, Result, Scheme, Url};

use super::Uri;

impl Uri {
    /// The handle this identifier names, resolved on the first call.
    pub(crate) fn held(&self) -> Result<&Holder> {
        if let Some(held) = self.held.get() {
            return Ok(held.as_ref());
        }
        let resolved = self.resolve()?;
        Ok(self.held.get_or_init(|| Box::new(resolved)).as_ref())
    }

    /// The handle this identifier names, mutably, resolved on the first call.
    fn held_mut(&mut self) -> Result<&mut Holder> {
        if self.held.get().is_none() {
            self.held = OnceLock::from(Box::new(self.resolve()?));
        }
        match self.held.get_mut() {
            Some(held) => Ok(held.as_mut()),
            None => unreachable!("a handle was resolved just above"),
        }
    }

    /// Open nothing: name the backend the location selects, as every other
    /// location does. [`Holder::from_url`] never answers [`Holder::Uri`], so
    /// resolving cannot recurse.
    fn resolve(&self) -> Result<Holder> {
        Holder::from_url(self, std::iter::empty::<(&str, &str)>())
    }

    /// Whether this identifier is a name - a URN or an ARN - rather than the
    /// location it resolves to.
    fn is_name(&self) -> bool {
        self.scheme() == &Scheme::URN || self.scheme() == &Scheme::ARN
    }

    /// The resolved handle, taken out of this value: what a reader holding
    /// many handles in turn streams from without borrowing this one.
    pub(crate) fn into_held(mut self) -> Result<Holder> {
        match self.held.take() {
            Some(held) => Ok(*held),
            None => self.resolve(),
        }
    }
}

impl IOBase for Uri {
    crate::__delegate_resolved_iobase!(held, held_mut, held);

    /// This identifier itself, with nothing resolved.
    fn uri(&self) -> Option<&Uri> {
        Some(self)
    }

    /// The location this handle opens; none for a name, which has no URL of
    /// its own to lend - [`uri`](Self::uri) is its address.
    fn url(&self) -> Option<&Url> {
        if self.is_name() {
            return None;
        }
        self.held().ok()?.url()
    }
}

/// Every record operation is the resolved handle's, so an HTTP location's
/// paginated rows, a table folder and the media a name infers answer as they
/// do on that handle.
impl IOMedia for Uri {
    crate::__delegate_resolved_iomedia!(held, held_mut);
}
