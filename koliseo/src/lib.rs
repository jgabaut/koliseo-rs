// SPDX-License-Identifier: GPL-3.0-only
/*
    Copyright (C) 2026 jgabaut

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, version 3 of the License.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/
/// A region-based allocator backed by Koliseo.
///
/// Allocations are automatically reclaimed when the arena is dropped.
///
/// # Examples
///
/// ```rust
/// use koliseo::Koliseo;
///
/// let arena = Koliseo::new(1024).expect("failed to create arena");
///
/// let number = arena.alloc(42_u32).expect("allocation failed");
/// let text = arena
///     .alloc(String::from("Hello, arena!"))
///     .expect("allocation failed");
///
/// assert_eq!(*number, 42);
/// assert_eq!(text.as_str(), "Hello, arena!");
///
/// let numbers = arena
///     .alloc_slice_with(10, |i| i * 2)
///     .unwrap();
///
/// assert_eq!(numbers, &[0, 2, 4, 6, 8, 10, 12, 14, 16, 18]);
///
/// let strings = arena
///     .alloc_slice_with(3, |i| format!("String {i}"))
///     .unwrap();
///
/// assert_eq!(strings[1], "String 1");
///
/// let values = arena
///     .alloc_slice_with(10, |_| String::from("hello"))
///     .unwrap();
/// assert_eq!(values[1], "hello");
/// ```
///
/// ## Destruction
///
/// Objects allocated in the arena have their destructors called
/// when the arena is dropped.
///
/// ```
/// use koliseo::Koliseo;
/// use std::sync::atomic::{AtomicBool, Ordering};
///
/// static DROPPED: AtomicBool = AtomicBool::new(false);
///
/// struct MyObject;
///
/// impl Drop for MyObject {
///     fn drop(&mut self) {
///         DROPPED.store(true, Ordering::SeqCst);
///     }
/// }
///
/// {
///     let arena = Koliseo::new(1024).unwrap();
///     arena.alloc(MyObject).unwrap();
///
///     assert!(!DROPPED.load(Ordering::SeqCst));
/// }
///
/// assert!(DROPPED.load(Ordering::SeqCst));
/// ```
///
/// ## Slice destruction
///
/// Elements allocated in a slice have their destructors called
/// when the arena is dropped.
///
/// ```rust
/// use koliseo::Koliseo;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// static DROPPED: AtomicUsize = AtomicUsize::new(0);
///
/// struct MyObject;
///
/// impl Drop for MyObject {
///     fn drop(&mut self) {
///         DROPPED.fetch_add(1, Ordering::SeqCst);
///     }
/// }
///
/// {
///     let arena = Koliseo::new(1024).unwrap();
///     arena.alloc_slice_with(3, |_| MyObject).unwrap();
///
///     assert_eq!(DROPPED.load(Ordering::SeqCst), 0);
/// }
///
/// assert_eq!(DROPPED.load(Ordering::SeqCst), 3);
/// ```
///
/// ## Growing allocations
///
/// Existing slices can be grown with [`Koliseo::grow_slice_with`].
/// The initializer receives the index of each newly added element.
///
/// ```rust
/// use koliseo::Koliseo;
///
/// let arena = Koliseo::new(1024).unwrap();
///
/// let numbers = arena
///     .alloc_slice_with(3, |i| i)
///     .unwrap();
///
/// assert_eq!(numbers, &[0, 1, 2]);
///
/// let numbers = arena
///     .grow_slice_with(numbers, 6, |i| i)
///     .unwrap();
///
/// assert_eq!(numbers, &[0, 1, 2, 3, 4, 5]);
/// ```
///
/// ### Growing slices of non-Copy types
///
/// Growing also works with types that implement `Drop`.
/// Existing elements are preserved, and newly added elements
/// are initialized by the supplied closure.
///
/// ```rust
/// use koliseo::Koliseo;
///
/// let arena = Koliseo::new(4096).unwrap();
///
/// let strings = arena
///     .alloc_slice_with(2, |i| format!("String {i}"))
///     .unwrap();
///
/// let strings = arena
///     .grow_slice_with(strings, 4, |i| format!("String {i}"))
///     .unwrap();
///
/// assert_eq!(strings, [
///     "String 0",
///     "String 1",
///     "String 2",
///     "String 3",
/// ]);
/// ```
///
/// ### Growing multiple times
///
/// A slice can be grown multiple times.
///
/// ```rust
/// use koliseo::Koliseo;
///
/// let arena = Koliseo::new(4096).unwrap();
///
/// let values = arena
///     .alloc_slice_with(2, |i| i)
///     .unwrap();
///
/// let values = arena
///     .grow_slice_with(values, 4, |i| i)
///     .unwrap();
///
/// assert_eq!(values, &[0, 1, 2, 3]);
///
/// let values = arena
///     .grow_slice_with(values, 8, |i| i)
///     .unwrap();
///
/// assert_eq!(values, &[0, 1, 2, 3, 4, 5, 6, 7]);
/// ```
///
/// ### Growing to the same length
///
/// Requesting the current length returns the original slice
/// without invoking the initializer.
///
/// ```rust
/// use koliseo::Koliseo;
///
/// let arena = Koliseo::new(1024).unwrap();
///
/// let values = arena
///     .alloc_slice_with(3, |i| i)
///     .unwrap();
///
/// let values = arena
///     .grow_slice_with(values, 3, |_| {
///         panic!("initializer must not be called");
///     })
///     .unwrap();
///
/// assert_eq!(values, &[0, 1, 2]);
/// ```
///
/// ### Initializer panic safety
///
/// If the initializer panics, the original allocation remains
/// unchanged and its elements are still valid.
///
/// ```rust
/// use koliseo::Koliseo;
/// use std::panic::{catch_unwind, AssertUnwindSafe};
///
/// let arena = Koliseo::new(1024).unwrap();
///
/// let values = arena
///     .alloc_slice_with(3, |i| i)
///     .unwrap();
///
/// let result = catch_unwind(AssertUnwindSafe(|| {
///     let _ = arena.grow_slice_with(values, 6, |i| {
///         if i == 4 {
///             panic!("initializer failed");
///         }
///         i
///     });
/// }));
///
/// assert!(result.is_err());
/// assert_eq!(values, &[0, 1, 2]);
/// ```
///
/// ### Destruction after growth
///
/// Both original and newly initialized elements have their
/// destructors called when the arena is dropped.
///
/// ```rust
/// use koliseo::Koliseo;
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// static DROPPED: AtomicUsize = AtomicUsize::new(0);
///
/// struct MyObject(u8);
///
/// impl Drop for MyObject {
///     fn drop(&mut self) {
///         DROPPED.fetch_add(1, Ordering::SeqCst);
///     }
/// }
///
/// DROPPED.store(0, Ordering::SeqCst);
///
/// {
///     let arena = Koliseo::new(2048).unwrap();
///
///     let objects = arena
///         .alloc_slice_with(2, |_| MyObject(1))
///         .unwrap();
///
///     let objects = arena
///         .grow_slice_with(objects, 5, |_| MyObject(2))
///         .unwrap();
///
///     assert_eq!(objects.len(), 5);
///     assert_eq!(DROPPED.load(Ordering::SeqCst), 0);
/// }
///
/// assert_eq!(DROPPED.load(Ordering::SeqCst), 5);
/// ```
use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Return the koliseo API version as [`i32`].
pub fn api_version_int() -> i32 {
    unsafe { koliseo_sys::int_koliseo_version() }
}

/// Return the koliseo API version as `&'static str`.
pub fn api_version_string() -> &'static str {
    let bytes = koliseo_sys::KOLISEO_API_VERSION_STRING;

    // The trailing NUL is part of the C string, not the Rust string.
    let bytes = &bytes[..bytes.len() - 1];

    std::str::from_utf8(bytes).expect("Koliseo API version string must be valid UTF-8")
}

struct Allocation {
    ptr: *mut u8,
    size: usize,
    align: usize,
    count: usize,
    is_slice: bool,
}

/// A function that knows how to destroy a particular type.
struct Destructor {
    ptr: *mut u8,
    len: usize,
    drop_fn: unsafe fn(*mut u8, usize),
}

/// Type-specific destructor.
unsafe fn drop_value<T>(ptr: *mut u8, _len: usize) {
    unsafe {
        std::ptr::drop_in_place(ptr.cast::<T>());
    }
}

/// Type-specific slice destructor.
unsafe fn drop_slice<T>(ptr: *mut u8, len: usize) {
    unsafe {
        let slice = std::ptr::slice_from_raw_parts_mut(ptr.cast::<T>(), len);
        std::ptr::drop_in_place(slice);
    }
}

/// This guard ensures partially initialized slices are cleaned up if the initializer panics.
struct SliceInitGuard<T> {
    ptr: NonNull<T>,
    initialized: usize,
}

impl<T> Drop for SliceInitGuard<T> {
    fn drop(&mut self) {
        unsafe {
            let slice = std::ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.initialized);

            std::ptr::drop_in_place(slice);
        }
    }
}

/// Error for Koliseo creation.
#[derive(Debug)]
pub struct KoliseoCreateError;

/// Error for allocation on Koliseo.
#[derive(Debug)]
pub struct KoliseoAllocError;

/// Arena allocator.
pub struct Koliseo {
    ptr: NonNull<koliseo_sys::Koliseo>,
    destructors: RefCell<Vec<Destructor>>,
    allocations: RefCell<Vec<Allocation>>,
    // Prevents automatic Send/Sync implementations.
    _marker: PhantomData<*mut ()>,
}

impl Koliseo {
    /// Construct a new [`Koliseo`].
    pub fn new(capacity: isize) -> Result<Self, KoliseoCreateError> {
        let ptr = unsafe { koliseo_sys::kls_new(capacity) };

        let ptr = NonNull::new(ptr).ok_or(KoliseoCreateError)?;

        Ok(Self {
            ptr,
            destructors: RefCell::new(Vec::new()),
            allocations: RefCell::new(Vec::new()),
            _marker: PhantomData,
        })
    }

    /// Allocate a value of type `T`.
    pub fn alloc<T: 'static>(&self, value: T) -> Result<&mut T, KoliseoAllocError> {
        // Reserve space in the destructor registry first.
        // This avoids allocating an object that we cannot register.
        if std::mem::needs_drop::<T>() {
            self.destructors
                .borrow_mut()
                .try_reserve(1)
                .map_err(|_| KoliseoAllocError)?;
        }

        let size: isize = std::mem::size_of::<T>()
            .try_into()
            .map_err(|_| KoliseoAllocError)?;
        let alignment: isize = std::mem::align_of::<T>()
            .try_into()
            .map_err(|_| KoliseoAllocError)?;

        // Zero-sized types don't need arena memory.
        let ptr = if size == 0 {
            NonNull::<T>::dangling().as_ptr().cast::<c_void>()
        } else {
            unsafe { koliseo_sys::kls_push_zero(self.ptr.as_ptr(), size, alignment, 1) }
        };

        let ptr = NonNull::new(ptr.cast::<T>()).ok_or(KoliseoAllocError)?;

        self.allocations
            .borrow_mut()
            .try_reserve(1)
            .map_err(|_| KoliseoAllocError)?;

        // Move the value into the arena.
        unsafe {
            ptr.as_ptr().write(value);
        }

        // Register its destructor if necessary.
        if std::mem::needs_drop::<T>() {
            self.destructors.borrow_mut().push(Destructor {
                ptr: ptr.as_ptr().cast(),
                len: 1,
                drop_fn: drop_value::<T>,
            });
        }

        self.allocations.borrow_mut().push(Allocation {
            ptr: ptr.as_ptr().cast(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            count: 1,
            is_slice: false,
        });

        // Return a reference tied to the arena borrow.
        unsafe { Ok(&mut *ptr.as_ptr()) }
    }

    /// Allocate a slice of `len` values initialized by `init`.
    ///
    /// The initializer receives the index of each element.
    ///
    /// Elements are stored contiguously in the arena.
    /// Their destructors are called when the arena is dropped.
    ///
    /// If initialization panics, already-initialized elements
    /// are dropped automatically.
    pub fn alloc_slice_with<T: 'static>(
        &self,
        len: usize,
        mut init: impl FnMut(usize) -> T,
    ) -> Result<&mut [T], KoliseoAllocError> {
        if len == 0 {
            return Ok(&mut []);
        }

        // Reserve destructor registry capacity first.
        if std::mem::needs_drop::<T>() {
            self.destructors
                .borrow_mut()
                .try_reserve(1)
                .map_err(|_| KoliseoAllocError)?;
        }

        let size: isize = std::mem::size_of::<T>()
            .try_into()
            .map_err(|_| KoliseoAllocError)?;

        let alignment: isize = std::mem::align_of::<T>()
            .try_into()
            .map_err(|_| KoliseoAllocError)?;

        let i_len: isize = len.try_into().map_err(|_| KoliseoAllocError)?;

        let ptr = if size == 0 {
            NonNull::<T>::dangling().as_ptr().cast::<c_void>()
        } else {
            unsafe { koliseo_sys::kls_push_zero(self.ptr.as_ptr(), size, alignment, i_len) }
        };

        let ptr = NonNull::new(ptr.cast::<T>()).ok_or(KoliseoAllocError)?;

        self.allocations
            .borrow_mut()
            .try_reserve(len)
            .map_err(|_| KoliseoAllocError)?;

        // Guard partially initialized elements.
        let mut guard = SliceInitGuard {
            ptr,
            initialized: 0,
        };

        for i in 0..len {
            let value = init(i);

            unsafe {
                ptr.as_ptr().add(i).write(value);
            }

            guard.initialized += 1;
        }

        // All elements are initialized.
        // Disarm the guard before registering the destructor.
        guard.initialized = 0;
        std::mem::forget(guard);

        if std::mem::needs_drop::<T>() {
            self.destructors.borrow_mut().push(Destructor {
                ptr: ptr.as_ptr().cast(),
                len,
                drop_fn: drop_slice::<T>,
            });
        }

        self.allocations.borrow_mut().push(Allocation {
            ptr: ptr.as_ptr().cast(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            count: len,
            is_slice: true,
        });

        unsafe { Ok(std::slice::from_raw_parts_mut(ptr.as_ptr(), len)) }
    }

    pub fn grow_slice_with<'a, T: 'static>(
        &self,
        old: &'a mut [T],
        new_len: usize,
        mut init: impl FnMut(usize) -> T,
    ) -> Result<&'a mut [T], KoliseoAllocError> {
        let old_len = old.len();

        if new_len < old_len {
            return Err(KoliseoAllocError);
        }

        if new_len == old_len {
            return Ok(old);
        }

        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();

        // Empty slices aren't registered allocations.
        if old_len == 0 {
            return Err(KoliseoAllocError);
        }

        // A slice pointer cannot uniquely identify a ZST allocation.
        if size == 0 {
            return Err(KoliseoAllocError);
        }

        // Validate all integer conversions before doing anything.
        let size_i: isize = size.try_into().map_err(|_| KoliseoAllocError)?;

        let align_i: isize = align.try_into().map_err(|_| KoliseoAllocError)?;

        let old_count: isize = old_len.try_into().map_err(|_| KoliseoAllocError)?;

        let new_count: isize = new_len.try_into().map_err(|_| KoliseoAllocError)?;

        // Also check multiplication explicitly.
        let total_size = size
            .checked_mul(new_len)
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or(KoliseoAllocError)?;

        let old_ptr = old.as_mut_ptr();

        // Initialize new elements in temporary storage FIRST.
        // If init panics, the original allocation is untouched.
        let additional = new_len - old_len;

        let mut additions = Vec::new();

        additions
            .try_reserve_exact(additional)
            .map_err(|_| KoliseoAllocError)?;

        for i in old_len..new_len {
            additions.push(init(i));
        }

        // Validate the allocation and destructor registrations.
        let mut allocations = self.allocations.borrow_mut();

        let allocation_index = allocations
            .iter()
            .position(|allocation| {
                allocation.ptr == old_ptr.cast()
                    && allocation.size == size
                    && allocation.align == align
                    && allocation.count == old_len
                    && allocation.is_slice
            })
            .ok_or(KoliseoAllocError)?;

        let mut destructors = self.destructors.borrow_mut();

        let destructor_index = if std::mem::needs_drop::<T>() {
            Some(
                destructors
                    .iter()
                    .position(|destructor| {
                        destructor.ptr == old_ptr.cast() && destructor.len == old_len
                    })
                    .ok_or(KoliseoAllocError)?,
            )
        } else {
            None
        };

        // All potentially failing Rust-side checks are complete.
        // Call kls_repush.
        let new_ptr = unsafe {
            koliseo_sys::kls_repush(
                self.ptr.as_ptr(),
                old_ptr.cast(),
                size_i,
                align_i,
                old_count,
                new_count,
            )
        };

        let Some(new_ptr) = NonNull::new(new_ptr.cast::<T>()) else {
            return Err(KoliseoAllocError);
        };

        // Update allocation metadata immediately.
        allocations[allocation_index].ptr = new_ptr.as_ptr().cast();

        allocations[allocation_index].count = new_len;

        // Update the destructor registration.
        if let Some(index) = destructor_index {
            destructors[index].ptr = new_ptr.as_ptr().cast();
            destructors[index].len = new_len;
        }

        drop(destructors);
        drop(allocations);

        // Transfer the initialized values into the arena.
        // No user code or fallible operation remains here.
        for (offset, value) in additions.into_iter().enumerate() {
            unsafe {
                new_ptr.as_ptr().add(old_len + offset).write(value);
            }
        }

        // Return the new slice.
        unsafe { Ok(std::slice::from_raw_parts_mut(new_ptr.as_ptr(), new_len)) }
    }
}

/// Ensures the C arena is destroyed even if a destructor panics.
struct DestroyKoliseo(NonNull<koliseo_sys::Koliseo>);

impl Drop for DestroyKoliseo {
    fn drop(&mut self) {
        unsafe {
            koliseo_sys::kls_free(self.0.as_ptr());
        }
    }
}

impl Drop for Koliseo {
    fn drop(&mut self) {
        // This guard will destroy the C arena even during unwinding.
        let _guard = DestroyKoliseo(self.ptr);

        // Destroy objects in reverse allocation order.
        for destructor in self.destructors.get_mut().drain(..).rev() {
            unsafe {
                (destructor.drop_fn)(destructor.ptr, destructor.len);
            }
        }
    }
}
