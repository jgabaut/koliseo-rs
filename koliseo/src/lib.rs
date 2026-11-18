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
/// let arena = Koliseo::new(2048).expect("failed to create arena");
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
/// /// ## Allocation scopes
///
/// Objects allocated within a scope are destroyed when
/// the scope ends.
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
/// let mut arena = Koliseo::new(1024).unwrap();
///
/// arena.scope(|scope| {
///     scope.alloc_slice_with(3, |_| MyObject).unwrap();
///     assert_eq!(DROPPED.load(Ordering::SeqCst), 0);
/// });
///
/// assert_eq!(DROPPED.load(Ordering::SeqCst), 3);
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

trait ArenaAlloc {
    fn arena_ptr(&self) -> NonNull<koliseo_sys::Koliseo>;

    fn destructors(&self) -> &RefCell<Vec<Destructor>>;
}

impl ArenaAlloc for Koliseo {
    fn arena_ptr(&self) -> NonNull<koliseo_sys::Koliseo> {
        self.ptr
    }

    fn destructors(&self) -> &RefCell<Vec<Destructor>> {
        &self.destructors
    }
}

impl ArenaAlloc for KoliseoScope<'_> {
    fn arena_ptr(&self) -> NonNull<koliseo_sys::Koliseo> {
        self.ptr
    }

    fn destructors(&self) -> &RefCell<Vec<Destructor>> {
        &self.destructors
    }
}

fn alloc_impl<A: ArenaAlloc, T: 'static>(arena: &A, value: T) -> Result<&mut T, KoliseoAllocError> {
    if std::mem::needs_drop::<T>() {
        arena
            .destructors()
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

    let ptr = if size == 0 {
        NonNull::<T>::dangling().as_ptr().cast::<c_void>()
    } else {
        unsafe { koliseo_sys::kls_push_zero(arena.arena_ptr().as_ptr(), size, alignment, 1) }
    };

    let ptr = NonNull::new(ptr.cast::<T>()).ok_or(KoliseoAllocError)?;

    unsafe {
        ptr.as_ptr().write(value);
    }

    if std::mem::needs_drop::<T>() {
        arena.destructors().borrow_mut().push(Destructor {
            ptr: ptr.as_ptr().cast(),
            len: 1,
            drop_fn: drop_value::<T>,
        });
    }

    unsafe { Ok(&mut *ptr.as_ptr()) }
}

fn alloc_slice_with_impl<A: ArenaAlloc, T: 'static>(
    arena: &A,
    len: usize,
    mut init: impl FnMut(usize) -> T,
) -> Result<&mut [T], KoliseoAllocError> {
    if len == 0 {
        return Ok(&mut []);
    }

    if std::mem::needs_drop::<T>() {
        arena
            .destructors()
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

    let count: isize = len.try_into().map_err(|_| KoliseoAllocError)?;

    let ptr = if size == 0 {
        NonNull::<T>::dangling().as_ptr().cast::<c_void>()
    } else {
        unsafe { koliseo_sys::kls_push_zero(arena.arena_ptr().as_ptr(), size, alignment, count) }
    };

    let ptr = NonNull::new(ptr.cast::<T>()).ok_or(KoliseoAllocError)?;

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

    guard.initialized = 0;
    std::mem::forget(guard);

    if std::mem::needs_drop::<T>() {
        arena.destructors().borrow_mut().push(Destructor {
            ptr: ptr.as_ptr().cast(),
            len,
            drop_fn: drop_slice::<T>,
        });
    }

    unsafe { Ok(std::slice::from_raw_parts_mut(ptr.as_ptr(), len)) }
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
        alloc_impl(self, value)
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
        init: impl FnMut(usize) -> T,
    ) -> Result<&mut [T], KoliseoAllocError> {
        alloc_slice_with_impl(self, len, init)
    }

    /// Execute a closure within a temporary allocation scope.
    ///
    /// All previously allocated objects are destroyed before
    /// entering the scope. All objects allocated within the
    /// scope are destroyed when the closure returns.
    ///
    /// The underlying arena is reset both before and after
    /// the scope, allowing its memory to be reused.
    pub fn scope<R>(&mut self, f: impl for<'scope> FnOnce(&'scope KoliseoScope<'scope>) -> R) -> R {
        // Destroy allocations from the previous arena phase.
        {
            let _guard = ResetKoliseo(self.ptr);

            for destructor in self.destructors.get_mut().drain(..).rev() {
                unsafe {
                    (destructor.drop_fn)(destructor.ptr, destructor.len);
                }
            }
        }

        let scope = KoliseoScope {
            ptr: self.ptr,
            destructors: RefCell::new(Vec::new()),
            _arena: PhantomData,
            _marker: PhantomData,
        };

        f(&scope)
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

/// A temporary allocation scope.
///
/// Allocations made through this scope are destroyed when
/// the scope ends.
pub struct KoliseoScope<'arena> {
    ptr: NonNull<koliseo_sys::Koliseo>,
    destructors: RefCell<Vec<Destructor>>,

    // The scope exclusively borrows its parent arena.
    _arena: PhantomData<&'arena mut Koliseo>,

    // Prevent automatic Send/Sync implementations.
    _marker: PhantomData<*mut ()>,
}

impl KoliseoScope<'_> {
    /// Allocate a value of type `T` in this scope.
    pub fn alloc<T: 'static>(&self, value: T) -> Result<&mut T, KoliseoAllocError> {
        alloc_impl(self, value)
    }

    /// Allocate a slice of `len` values initialized by `init`.
    ///
    /// Elements are stored contiguously in the arena and
    /// their destructors are called when the scope ends.
    ///
    /// If initialization panics, already-initialized elements
    /// are dropped automatically.
    pub fn alloc_slice_with<T: 'static>(
        &self,
        len: usize,
        init: impl FnMut(usize) -> T,
    ) -> Result<&mut [T], KoliseoAllocError> {
        alloc_slice_with_impl(self, len, init)
    }
}

struct ResetKoliseo(NonNull<koliseo_sys::Koliseo>);

impl Drop for ResetKoliseo {
    fn drop(&mut self) {
        unsafe {
            koliseo_sys::kls_clear(self.0.as_ptr());
        }
    }
}

impl Drop for KoliseoScope<'_> {
    fn drop(&mut self) {
        let _guard = ResetKoliseo(self.ptr);

        for destructor in self.destructors.get_mut().drain(..).rev() {
            unsafe {
                (destructor.drop_fn)(destructor.ptr, destructor.len);
            }
        }
    }
}
