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

/// A function that knows how to destroy a particular type.
struct Destructor {
    ptr: *mut u8,
    drop_fn: unsafe fn(*mut u8),
}

/// Type-specific destructor.
unsafe fn drop_value<T>(ptr: *mut u8) {
    unsafe {
        std::ptr::drop_in_place(ptr.cast::<T>());
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

        // Move the value into the arena.
        unsafe {
            ptr.as_ptr().write(value);
        }

        // Register its destructor if necessary.
        if std::mem::needs_drop::<T>() {
            self.destructors.borrow_mut().push(Destructor {
                ptr: ptr.as_ptr().cast(),
                drop_fn: drop_value::<T>,
            });
        }

        // Return a reference tied to the arena borrow.
        unsafe { Ok(&mut *ptr.as_ptr()) }
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
                (destructor.drop_fn)(destructor.ptr);
            }
        }
    }
}
