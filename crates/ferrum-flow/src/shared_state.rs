//! Type-erased map for data shared between plugins on one [`crate::canvas::FlowCanvas`].
//!
//! Store values under their concrete Rust type (`TypeId`). Each type may appear at most once.
//! Prefer newtype wrappers per feature to avoid collisions (e.g. `struct MyPluginState(u32)`).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

/// Keyed by [`TypeId`]; values must be `'static` and [`Send`].
#[derive(Default)]
pub struct SharedState {
    inner: HashMap<TypeId, Box<dyn Any + Send>>,
}

impl fmt::Debug for SharedState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedState")
            .field("len", &self.inner.len())
            .finish()
    }
}

impl SharedState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a value, returning the previous one of the same type if any.
    pub fn insert<T: Any + Send + 'static>(&mut self, value: T) -> Option<T> {
        let id = TypeId::of::<T>();
        let old = self.inner.remove(&id);
        self.inner.insert(id, Box::new(value));
        old.and_then(|b| b.downcast::<T>().ok().map(|b| *b))
    }

    pub fn get<T: Any + Send + 'static>(&self) -> Option<&T> {
        self.inner.get(&TypeId::of::<T>())?.downcast_ref()
    }

    pub fn get_mut<T: Any + Send + 'static>(&mut self) -> Option<&mut T> {
        self.inner.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    pub fn remove<T: Any + Send + 'static>(&mut self) -> Option<T> {
        self.inner
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast::<T>().ok().map(|b| *b))
    }

    pub fn contains<T: Any + Send + 'static>(&self) -> bool {
        self.inner.contains_key(&TypeId::of::<T>())
    }
}
