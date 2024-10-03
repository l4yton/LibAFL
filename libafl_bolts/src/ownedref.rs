//! Wrappers that abstracts references (or pointers) and owned data accesses.
// The serialization is towards owned, allowing to serialize pointers without troubles.

use alloc::{
    boxed::Box,
    slice::{Iter, IterMut},
    vec::Vec,
};
use core::{
    clone::Clone,
    fmt::Debug,
    ops::{Deref, DerefMut, RangeBounds},
    slice,
    slice::SliceIndex,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{shmem::ShMem, AsSlice, AsSliceMut, IntoOwned, Truncate};

/// Private part of the unsafe marker, making sure this cannot be initialized directly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct UnsafeMarkerInner;

/// A struct or enum containing this [`UnsafeMarker`] cannot directly be instantiated.
/// Usually, this means you'll have to use a constructor like `unsafe { Self::new() }` or similar.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnsafeMarker(UnsafeMarkerInner);

impl UnsafeMarker {
    /// Return a new unsafe marker.
    /// This usually means you're about to do something unsafe.
    ///
    /// # Safety
    /// Instantiating an [`UnsafeMarker`] isn't unsafe, but the location you need it for
    /// will likely have potential unsafety.
    unsafe fn new() -> Self {
        Self(UnsafeMarkerInner)
    }
}

impl<'a, T> Truncate for &'a [T] {
    fn truncate(&mut self, len: usize) {
        *self = &self[..len];
    }
}

impl<'a, T> Truncate for &'a mut [T] {
    fn truncate(&mut self, len: usize) {
        let mut value = core::mem::take(self);
        value = unsafe { value.get_unchecked_mut(..len) };
        let _: &mut [T] = core::mem::replace(self, value);
    }
}

/// Wrap a reference and convert to a [`Box`] on serialize
#[derive(Debug)]
pub enum OwnedRef<'a, T>
where
    T: 'a + ?Sized,
{
    /// A pointer to a type
    RefRaw(*const T, UnsafeMarker),
    /// A ref to a type
    Ref(&'a T),
    /// An owned [`Box`] of a type
    Owned(Box<T>),
}

/// Special case, &\[u8] is a fat pointer containing the size implicitly.
impl<'a> Clone for OwnedRef<'a, [u8]> {
    fn clone(&self) -> Self {
        match self {
            Self::RefRaw(_, _) => panic!("Cannot clone"),
            Self::Ref(slice) => Self::Ref(slice),
            Self::Owned(elt) => Self::Owned(elt.clone()),
        }
    }
}

impl<'a, T> Clone for OwnedRef<'a, T>
where
    T: 'a + Sized + Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::RefRaw(ptr, mrkr) => Self::RefRaw(*ptr, mrkr.clone()),
            Self::Ref(slice) => Self::Ref(slice),
            Self::Owned(elt) => Self::Owned(elt.clone()),
        }
    }
}

impl<'a, T> OwnedRef<'a, T>
where
    T: 'a + ?Sized,
{
    /// Returns a new [`OwnedRef`], wrapping a pointer of type `T`.
    ///
    /// # Safety
    /// The pointer needs to point to a valid object of type `T`.
    /// Any use of this [`OwnedRef`] will dereference the pointer accordingly.
    pub unsafe fn from_ptr(ptr: *const T) -> Self {
        assert!(
            !ptr.is_null(),
            "Null pointer passed to OwnedRef::ref_raw constructor!"
        );
        Self::RefRaw(ptr, UnsafeMarker::new())
    }

    /// Returns true if the inner ref is a raw pointer, false otherwise.
    #[must_use]
    pub fn is_raw(&self) -> bool {
        matches!(self, OwnedRef::Ref(_))
    }

    /// Return the inner value, if owned by the given object
    #[must_use]
    pub fn into_owned(self) -> Option<Box<T>> {
        match self {
            Self::Owned(val) => Some(val),
            _ => None,
        }
    }
}

impl<'a, T> OwnedRef<'a, T>
where
    T: Sized + 'static,
{
    /// Returns a new [`OwnedRef`], pointing to the given [`ShMem`].
    ///
    /// # Panics
    /// Panics if the given shared mem is too small
    ///
    /// # Safety
    /// The shared memory needs to start with a valid object of type `T`.
    /// Any use of this [`OwnedRef`] will dereference a pointer to the shared memory accordingly.
    pub unsafe fn from_shmem<S: ShMem>(shmem: &mut S) -> Self {
        Self::from_ptr(shmem.as_mut_ptr_of().unwrap())
    }

    /// Returns a new [`OwnedRef`], owning the given value.
    pub fn owned(val: T) -> Self {
        Self::Owned(Box::new(val))
    }
}

impl<'a, T> Serialize for OwnedRef<'a, T>
where
    T: 'a + ?Sized + Serialize,
{
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            OwnedRef::RefRaw(r, _) => unsafe { (*r).as_ref().unwrap() }.serialize(se),
            OwnedRef::Ref(r) => r.serialize(se),
            OwnedRef::Owned(b) => b.serialize(se),
        }
    }
}

impl<'de, 'a, T> Deserialize<'de> for OwnedRef<'a, T>
where
    T: 'a + ?Sized,
    Box<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer).map(OwnedRef::Owned)
    }
}

impl<'a> AsRef<[u8]> for OwnedRef<'a, [u8]> {
    #[must_use]
    fn as_ref(&self) -> &[u8] {
        match self {
            OwnedRef::RefRaw(r, _) => unsafe { (*r).as_ref().unwrap() },
            OwnedRef::Ref(r) => r,
            OwnedRef::Owned(v) => v.as_ref(),
        }
    }
}

impl<'a, T> AsRef<T> for OwnedRef<'a, T>
where
    T: Sized,
{
    #[must_use]
    fn as_ref(&self) -> &T {
        match self {
            OwnedRef::RefRaw(r, _) => unsafe { (*r).as_ref().unwrap() },
            OwnedRef::Ref(r) => r,
            OwnedRef::Owned(v) => v.as_ref(),
        }
    }
}

impl<'a, T> IntoOwned for OwnedRef<'a, T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self {
            OwnedRef::RefRaw(..) | OwnedRef::Ref(_) => false,
            OwnedRef::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        match self {
            OwnedRef::RefRaw(r, _) => {
                OwnedRef::Owned(Box::new(unsafe { r.as_ref().unwrap().clone() }))
            }
            OwnedRef::Ref(r) => OwnedRef::Owned(Box::new(r.clone())),
            OwnedRef::Owned(v) => OwnedRef::Owned(v),
        }
    }
}

/// Wrap a mutable reference and convert to a Box on serialize
#[derive(Debug)]
pub enum OwnedRefMut<'a, T>
where
    T: 'a + ?Sized,
{
    /// A mutable pointer to a type
    RefRaw(*mut T, UnsafeMarker),
    /// A mutable ref to a type
    Ref(&'a mut T),
    /// An owned [`Box`] of a type
    Owned(Box<T>),
}

impl<'a, T> OwnedRefMut<'a, T>
where
    T: 'a + ?Sized,
{
    /// Returns a new [`OwnedRefMut`], wrapping a mutable pointer.
    ///
    /// # Panics
    /// Panics if the given pointer is `null`
    ///
    /// # Safety
    /// The pointer needs to point to a valid object of type `T`.
    /// Any use of this [`OwnedRefMut`] will dereference the pointer accordingly.
    pub unsafe fn from_mut_ptr(ptr: *mut T) -> Self {
        assert!(
            !ptr.is_null(),
            "Null pointer passed to OwnedRefMut::from_mut_ptr constructor!"
        );
        Self::RefRaw(ptr, UnsafeMarker::new())
    }
}

impl<'a, T> OwnedRefMut<'a, T>
where
    T: Sized + 'static,
{
    /// Returns a new [`OwnedRefMut`], pointing to the given [`ShMem`].
    ///
    /// # Panics
    /// Panics if the given shared mem is too small
    ///
    /// # Safety
    /// The shared memory needs to start with a valid object of type `T`.
    /// Any use of this [`OwnedRefMut`] will dereference a pointer to the shared memory accordingly.
    pub unsafe fn from_shmem<S: ShMem>(shmem: &mut S) -> Self {
        Self::from_mut_ptr(shmem.as_mut_ptr_of().unwrap())
    }

    /// Returns a new [`OwnedRefMut`], owning the given value.
    pub fn owned(val: T) -> Self {
        Self::Owned(Box::new(val))
    }
}

impl<'a, T: 'a + ?Sized + Serialize> Serialize for OwnedRefMut<'a, T> {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            OwnedRefMut::Ref(r) => r.serialize(se),
            OwnedRefMut::RefRaw(r, _) => unsafe { r.as_ref().unwrap().serialize(se) },
            OwnedRefMut::Owned(b) => b.serialize(se),
        }
    }
}

impl<'de, 'a, T: 'a + ?Sized> Deserialize<'de> for OwnedRefMut<'a, T>
where
    Box<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer).map(OwnedRefMut::Owned)
    }
}

impl<'a, T: ?Sized> AsRef<T> for OwnedRefMut<'a, T> {
    #[must_use]
    fn as_ref(&self) -> &T {
        match self {
            OwnedRefMut::RefRaw(r, _) => unsafe { r.as_ref().unwrap() },
            OwnedRefMut::Ref(r) => r,
            OwnedRefMut::Owned(v) => v.as_ref(),
        }
    }
}

impl<'a, T: ?Sized> AsMut<T> for OwnedRefMut<'a, T> {
    #[must_use]
    fn as_mut(&mut self) -> &mut T {
        match self {
            OwnedRefMut::RefRaw(r, _) => unsafe { r.as_mut().unwrap() },
            OwnedRefMut::Ref(r) => r,
            OwnedRefMut::Owned(v) => v.as_mut(),
        }
    }
}

impl<'a, T> IntoOwned for OwnedRefMut<'a, T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self {
            OwnedRefMut::RefRaw(..) | OwnedRefMut::Ref(_) => false,
            OwnedRefMut::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        match self {
            OwnedRefMut::RefRaw(r, _) => unsafe {
                OwnedRefMut::Owned(Box::new(r.as_ref().unwrap().clone()))
            },
            OwnedRefMut::Ref(r) => OwnedRefMut::Owned(Box::new(r.clone())),
            OwnedRefMut::Owned(v) => OwnedRefMut::Owned(v),
        }
    }
}

/// Wrap a slice and convert to a Vec on serialize
#[derive(Clone, Debug)]
enum OwnedSliceInner<'a, T: 'a + Sized> {
    /// A ref to a raw slice and length
    RefRaw(*const T, usize, UnsafeMarker),
    /// A ref to a slice
    Ref(&'a [T]),
    /// A ref to an owned [`Vec`]
    Owned(Vec<T>),
}

impl<'a, T: 'a + Sized + Serialize> Serialize for OwnedSliceInner<'a, T> {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            OwnedSliceInner::RefRaw(rr, len, _) => unsafe {
                slice::from_raw_parts(*rr, *len).serialize(se)
            },
            OwnedSliceInner::Ref(r) => r.serialize(se),
            OwnedSliceInner::Owned(b) => b.serialize(se),
        }
    }
}

impl<'de, 'a, T: 'a + Sized> Deserialize<'de> for OwnedSliceInner<'a, T>
where
    Vec<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer).map(OwnedSliceInner::Owned)
    }
}

/// Wrap a slice and convert to a Vec on serialize.
/// We use a hidden inner enum so the public API can be safe,
/// unless the user uses the unsafe [`OwnedSlice::from_raw_parts`]
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Debug, Serialize, Deserialize)]
pub struct OwnedSlice<'a, T: 'a + Sized> {
    inner: OwnedSliceInner<'a, T>,
}

impl<'a, T: 'a + Clone> Clone for OwnedSlice<'a, T> {
    fn clone(&self) -> Self {
        Self {
            inner: OwnedSliceInner::Owned(self.as_slice().to_vec()),
        }
    }
}

impl<'a, T> OwnedSlice<'a, T> {
    /// Create a new [`OwnedSlice`] from a raw pointer and length
    ///
    /// # Safety
    ///
    /// The pointer must be valid and point to a map of the size `size_of<T>() * len`
    /// The contents will be dereferenced in subsequent operations.
    #[must_use]
    pub unsafe fn from_raw_parts(ptr: *const T, len: usize) -> Self {
        Self {
            inner: OwnedSliceInner::RefRaw(ptr, len, UnsafeMarker::new()),
        }
    }

    /// Truncate the inner slice or vec returning the old size on success or `None` on failure
    pub fn truncate(&mut self, new_len: usize) -> Option<usize> {
        match &mut self.inner {
            OwnedSliceInner::RefRaw(_rr, len, _) => {
                let tmp = *len;
                if new_len <= tmp {
                    *len = new_len;
                    Some(tmp)
                } else {
                    None
                }
            }
            OwnedSliceInner::Ref(r) => {
                let tmp = r.len();
                if new_len <= tmp {
                    r.truncate(new_len);
                    Some(tmp)
                } else {
                    None
                }
            }
            OwnedSliceInner::Owned(v) => {
                let tmp = v.len();
                if new_len <= tmp {
                    v.truncate(new_len);
                    Some(tmp)
                } else {
                    None
                }
            }
        }
    }

    /// Returns an iterator over the slice.
    pub fn iter(&self) -> Iter<'_, T> {
        <&Self as IntoIterator>::into_iter(self)
    }

    /// Returns a subslice of the slice.
    #[must_use]
    pub fn slice<R: RangeBounds<usize> + SliceIndex<[T], Output = [T]>>(
        &'a self,
        range: R,
    ) -> OwnedSlice<'a, T> {
        OwnedSlice {
            inner: OwnedSliceInner::Ref(&self[range]),
        }
    }
}

impl<'a, 'it, T> IntoIterator for &'it OwnedSlice<'a, T> {
    type Item = <Iter<'it, T> as Iterator>::Item;
    type IntoIter = Iter<'it, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

/// Create a new [`OwnedSlice`] from a vector
impl<'a, T> From<Vec<T>> for OwnedSlice<'a, T> {
    fn from(vec: Vec<T>) -> Self {
        Self {
            inner: OwnedSliceInner::Owned(vec),
        }
    }
}

/// Create a new [`OwnedSlice`] from a vector reference
impl<'a, T> From<&'a Vec<T>> for OwnedSlice<'a, T> {
    fn from(vec: &'a Vec<T>) -> Self {
        Self {
            inner: OwnedSliceInner::Ref(vec),
        }
    }
}

/// Create a new [`OwnedSlice`] from a reference to a slice
impl<'a, T> From<&'a [T]> for OwnedSlice<'a, T> {
    fn from(r: &'a [T]) -> Self {
        Self {
            inner: OwnedSliceInner::Ref(r),
        }
    }
}

/// Create a new [`OwnedSlice`] from a [`OwnedMutSlice`]
impl<'a, T> From<OwnedMutSlice<'a, T>> for OwnedSlice<'a, T> {
    fn from(mut_slice: OwnedMutSlice<'a, T>) -> Self {
        Self {
            inner: match mut_slice.inner {
                OwnedMutSliceInner::RefRaw(ptr, len, unsafe_marker) => {
                    OwnedSliceInner::RefRaw(ptr as _, len, unsafe_marker)
                }
                OwnedMutSliceInner::Ref(r) => OwnedSliceInner::Ref(r as _),
                OwnedMutSliceInner::Owned(v) => OwnedSliceInner::Owned(v),
            },
        }
    }
}

impl<'a, T: Sized> Deref for OwnedSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            OwnedSliceInner::Ref(r) => r,
            OwnedSliceInner::RefRaw(rr, len, _) => unsafe { slice::from_raw_parts(*rr, *len) },
            OwnedSliceInner::Owned(v) => v.as_slice(),
        }
    }
}

impl<'a, T> IntoOwned for OwnedSlice<'a, T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self.inner {
            OwnedSliceInner::RefRaw(..) | OwnedSliceInner::Ref(_) => false,
            OwnedSliceInner::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        match self.inner {
            OwnedSliceInner::RefRaw(rr, len, _) => Self {
                inner: OwnedSliceInner::Owned(unsafe { slice::from_raw_parts(rr, len).to_vec() }),
            },
            OwnedSliceInner::Ref(r) => Self {
                inner: OwnedSliceInner::Owned(r.to_vec()),
            },
            OwnedSliceInner::Owned(v) => Self {
                inner: OwnedSliceInner::Owned(v),
            },
        }
    }
}

/// Create a vector from an [`OwnedMutSlice`], or return the owned vec.
impl<'a, T> From<OwnedSlice<'a, T>> for Vec<T>
where
    T: Clone,
{
    fn from(slice: OwnedSlice<'a, T>) -> Self {
        let slice = slice.into_owned();
        match slice.inner {
            OwnedSliceInner::Owned(vec) => vec,
            _ => panic!("Could not own slice!"),
        }
    }
}

/// Wrap a mutable slice and convert to a Vec on serialize.
/// We use a hidden inner enum so the public API can be safe,
/// unless the user uses the unsafe [`OwnedMutSlice::from_raw_parts_mut`]
#[derive(Debug)]
pub enum OwnedMutSliceInner<'a, T: 'a + Sized> {
    /// A raw ptr to a memory location and a length
    RefRaw(*mut T, usize, UnsafeMarker),
    /// A ptr to a mutable slice of the type
    Ref(&'a mut [T]),
    /// An owned [`Vec`] of the type
    Owned(Vec<T>),
}

impl<'a, T: 'a + Sized + Serialize> Serialize for OwnedMutSliceInner<'a, T> {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            OwnedMutSliceInner::RefRaw(rr, len, _) => {
                unsafe { slice::from_raw_parts_mut(*rr, *len) }.serialize(se)
            }
            OwnedMutSliceInner::Ref(r) => r.serialize(se),
            OwnedMutSliceInner::Owned(b) => b.serialize(se),
        }
    }
}

impl<'de, 'a, T: 'a + Sized> Deserialize<'de> for OwnedMutSliceInner<'a, T>
where
    Vec<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer).map(OwnedMutSliceInner::Owned)
    }
}

/// Wrap a mutable slice and convert to a Vec on serialize
#[allow(clippy::unsafe_derive_deserialize)]
#[derive(Debug, Serialize, Deserialize)]
pub struct OwnedMutSlice<'a, T: 'a + Sized> {
    inner: OwnedMutSliceInner<'a, T>,
}

impl<'a, 'it, T> IntoIterator for &'it mut OwnedMutSlice<'a, T> {
    type Item = <IterMut<'it, T> as Iterator>::Item;
    type IntoIter = IterMut<'it, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice_mut().iter_mut()
    }
}

impl<'a, 'it, T> IntoIterator for &'it OwnedMutSlice<'a, T> {
    type Item = <Iter<'it, T> as Iterator>::Item;
    type IntoIter = Iter<'it, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a, T: 'a + Sized> OwnedMutSlice<'a, T> {
    /// Create a new [`OwnedMutSlice`] from a raw pointer and length
    ///
    /// # Safety
    ///
    /// The pointer must be valid and point to a map of the size `size_of<T>() * len`
    /// The contents will be dereferenced in subsequent operations.
    #[must_use]
    pub unsafe fn from_raw_parts_mut(ptr: *mut T, len: usize) -> OwnedMutSlice<'a, T> {
        if ptr.is_null() || len == 0 {
            Self {
                inner: OwnedMutSliceInner::Owned(Vec::new()),
            }
        } else {
            Self {
                inner: OwnedMutSliceInner::RefRaw(ptr, len, UnsafeMarker::new()),
            }
        }
    }

    /// Truncate the inner slice or vec returning the old size on success or `None` on failure
    pub fn truncate(&mut self, new_len: usize) -> Option<usize> {
        match &mut self.inner {
            OwnedMutSliceInner::RefRaw(_rr, len, _) => {
                let tmp = *len;
                if new_len <= tmp {
                    *len = new_len;
                    Some(tmp)
                } else {
                    None
                }
            }
            OwnedMutSliceInner::Ref(r) => {
                let tmp = r.len();
                if new_len <= tmp {
                    r.truncate(new_len);
                    Some(tmp)
                } else {
                    None
                }
            }
            OwnedMutSliceInner::Owned(v) => {
                let tmp = v.len();
                if new_len <= tmp {
                    v.truncate(new_len);
                    Some(tmp)
                } else {
                    None
                }
            }
        }
    }

    /// Returns an iterator over the slice.
    pub fn iter(&self) -> Iter<'_, T> {
        <&Self as IntoIterator>::into_iter(self)
    }

    /// Returns a mutable iterator over the slice.
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        <&mut Self as IntoIterator>::into_iter(self)
    }
}

impl<'a, T: Sized> Deref for OwnedMutSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        match &self.inner {
            OwnedMutSliceInner::RefRaw(rr, len, _) => unsafe { slice::from_raw_parts(*rr, *len) },
            OwnedMutSliceInner::Ref(r) => r,
            OwnedMutSliceInner::Owned(v) => v.as_slice(),
        }
    }
}

impl<'a, T: Sized> DerefMut for OwnedMutSlice<'a, T> {
    fn deref_mut(&mut self) -> &mut [T] {
        match &mut self.inner {
            OwnedMutSliceInner::RefRaw(rr, len, _) => unsafe {
                slice::from_raw_parts_mut(*rr, *len)
            },
            OwnedMutSliceInner::Ref(r) => r,
            OwnedMutSliceInner::Owned(v) => v.as_slice_mut(),
        }
    }
}

impl<'a, T> IntoOwned for OwnedMutSlice<'a, T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self.inner {
            OwnedMutSliceInner::RefRaw(..) | OwnedMutSliceInner::Ref(_) => false,
            OwnedMutSliceInner::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        let vec = match self.inner {
            OwnedMutSliceInner::RefRaw(rr, len, _) => unsafe {
                slice::from_raw_parts_mut(rr, len).to_vec()
            },
            OwnedMutSliceInner::Ref(r) => r.to_vec(),
            OwnedMutSliceInner::Owned(v) => v,
        };
        Self {
            inner: OwnedMutSliceInner::Owned(vec),
        }
    }
}

impl<'a, T: 'a + Clone> Clone for OwnedMutSlice<'a, T> {
    fn clone(&self) -> Self {
        Self {
            inner: OwnedMutSliceInner::Owned(self.as_slice().to_vec()),
        }
    }
}

/// Create a new [`OwnedMutSlice`] from a vector
impl<'a, T> From<Vec<T>> for OwnedMutSlice<'a, T> {
    fn from(vec: Vec<T>) -> Self {
        Self {
            inner: OwnedMutSliceInner::Owned(vec),
        }
    }
}

/// Create a vector from an [`OwnedMutSlice`], or return the owned vec.
impl<'a, T> From<OwnedMutSlice<'a, T>> for Vec<T>
where
    T: Clone,
{
    fn from(slice: OwnedMutSlice<'a, T>) -> Self {
        let slice = slice.into_owned();
        match slice.inner {
            OwnedMutSliceInner::Owned(vec) => vec,
            _ => panic!("Could not own slice!"),
        }
    }
}

/// Create a new [`OwnedMutSlice`] from a vector reference
impl<'a, T> From<&'a mut Vec<T>> for OwnedMutSlice<'a, T> {
    fn from(vec: &'a mut Vec<T>) -> Self {
        Self {
            inner: OwnedMutSliceInner::Ref(vec),
        }
    }
}

/// Create a new [`OwnedMutSlice`] from a reference to ref to a slice
impl<'a, T> From<&'a mut [T]> for OwnedMutSlice<'a, T> {
    fn from(r: &'a mut [T]) -> Self {
        Self {
            inner: OwnedMutSliceInner::Ref(r),
        }
    }
}

/// Create a new [`OwnedMutSlice`] from a reference to ref to a slice
#[allow(clippy::mut_mut)] // This makes use in some iterators easier
impl<'a, T> From<&'a mut &'a mut [T]> for OwnedMutSlice<'a, T> {
    fn from(r: &'a mut &'a mut [T]) -> Self {
        Self {
            inner: OwnedMutSliceInner::Ref(r),
        }
    }
}

/// Wrap a C-style pointer and convert to a Box on serialize
#[derive(Clone, Debug)]
pub enum OwnedPtr<T: Sized> {
    /// Ptr to the content
    Ptr(*const T),
    /// Ptr to an owned [`Box`] of the content.
    Owned(Box<T>),
}

impl<T: Sized> OwnedPtr<T> {
    /// Creates a new [`OwnedPtr`] from a raw pointer
    ///
    /// # Safety
    /// The raw pointer will later be dereferenced.
    /// It must outlive this `OwnedPtr` type and remain valid.
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        Self::Ptr(ptr)
    }
}

impl<T: Sized + Serialize> Serialize for OwnedPtr<T> {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(se)
    }
}

impl<'de, T: Sized + serde::de::DeserializeOwned> Deserialize<'de> for OwnedPtr<T>
where
    Vec<T>: Deserialize<'de>,
{
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(de).map(OwnedPtr::Owned)
    }
}

impl<T: Sized> AsRef<T> for OwnedPtr<T> {
    #[must_use]
    fn as_ref(&self) -> &T {
        match self {
            OwnedPtr::Ptr(p) => unsafe { p.as_ref().unwrap() },
            OwnedPtr::Owned(v) => v.as_ref(),
        }
    }
}

impl<T> IntoOwned for OwnedPtr<T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self {
            OwnedPtr::Ptr(_) => false,
            OwnedPtr::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        match self {
            OwnedPtr::Ptr(p) => unsafe { OwnedPtr::Owned(Box::new(p.as_ref().unwrap().clone())) },
            OwnedPtr::Owned(v) => OwnedPtr::Owned(v),
        }
    }
}

/// Wrap a C-style mutable pointer and convert to a Box on serialize
#[derive(Clone, Debug)]
pub enum OwnedMutPtr<T: Sized> {
    /// A mut ptr to the content
    Ptr(*mut T),
    /// An owned [`Box`] to the content
    Owned(Box<T>),
}

impl<T: Sized> OwnedMutPtr<T> {
    /// Creates a new [`OwnedMutPtr`] from a raw pointer
    ///
    /// # Safety
    /// The raw pointer will later be dereferenced.
    /// It must outlive this `OwnedPtr` type and remain valid.
    pub unsafe fn from_raw_mut(ptr: *mut T) -> Self {
        Self::Ptr(ptr)
    }
}

impl<T: Sized + Serialize> Serialize for OwnedMutPtr<T> {
    fn serialize<S>(&self, se: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(se)
    }
}

impl<'de, T: Sized + serde::de::DeserializeOwned> Deserialize<'de> for OwnedMutPtr<T>
where
    Vec<T>: Deserialize<'de>,
{
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(de).map(OwnedMutPtr::Owned)
    }
}

impl<T: Sized> AsRef<T> for OwnedMutPtr<T> {
    #[must_use]
    fn as_ref(&self) -> &T {
        match self {
            OwnedMutPtr::Ptr(p) => unsafe { p.as_ref().unwrap() },
            OwnedMutPtr::Owned(b) => b.as_ref(),
        }
    }
}

impl<T: Sized> AsMut<T> for OwnedMutPtr<T> {
    fn as_mut(&mut self) -> &mut T {
        match self {
            OwnedMutPtr::Ptr(p) => unsafe { p.as_mut().unwrap() },
            OwnedMutPtr::Owned(b) => b.as_mut(),
        }
    }
}

impl<T> IntoOwned for OwnedMutPtr<T>
where
    T: Sized + Clone,
{
    #[must_use]
    fn is_owned(&self) -> bool {
        match self {
            OwnedMutPtr::Ptr(_) => false,
            OwnedMutPtr::Owned(_) => true,
        }
    }

    #[must_use]
    fn into_owned(self) -> Self {
        match self {
            OwnedMutPtr::Ptr(p) => unsafe {
                OwnedMutPtr::Owned(Box::new(p.as_ref().unwrap().clone()))
            },
            OwnedMutPtr::Owned(v) => OwnedMutPtr::Owned(v),
        }
    }
}
