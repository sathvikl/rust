// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

/*!
Utilities for random number generation

The key functions are `random()` and `Rng::gen()`. These are polymorphic
and so can be used to generate any type that implements `Rand`. Type inference
means that often a simple call to `rand::random()` or `rng.gen()` will
suffice, but sometimes an annotation is required, e.g. `rand::random::<f64>()`.

See the `distributions` submodule for sampling random numbers from
distributions like normal and exponential.

# Task-local RNG

There is built-in support for a RNG associated with each task stored
in task-local storage. This RNG can be accessed via `task_rng`, or
used implicitly via `random`. This RNG is normally randomly seeded
from an operating-system source of randomness, e.g. `/dev/urandom` on
Unix systems, and will automatically reseed itself from this source
after generating 32 KiB of random data.

# Cryptographic security

An application that requires an entropy source for cryptographic purposes
must use `OSRng`, which reads randomness from the source that the operating
system provides (e.g. `/dev/urandom` on Unixes or `CryptGenRandom()` on Windows).
The other random number generators provided by this module are not suitable
for such purposes.

*Note*: many Unix systems provide `/dev/random` as well as `/dev/urandom`.
This module uses `/dev/urandom` for the following reasons:

-   On Linux, `/dev/random` may block if entropy pool is empty; `/dev/urandom` will not block.
    This does not mean that `/dev/random` provides better output than
    `/dev/urandom`; the kernel internally runs a cryptographically secure pseudorandom
    number generator (CSPRNG) based on entropy pool for random number generation,
    so the "quality" of `/dev/random` is not better than `/dev/urandom` in most cases.
    However, this means that `/dev/urandom` can yield somewhat predictable randomness
    if the entropy pool is very small, such as immediately after first booting.
    If an application likely to be run soon after first booting, or on a system with very
    few entropy sources, one should consider using `/dev/random` via `ReaderRng`.
-   On some systems (e.g. FreeBSD, OpenBSD and Mac OS X) there is no difference
    between the two sources. (Also note that, on some systems e.g. FreeBSD, both `/dev/random`
    and `/dev/urandom` may block once if the CSPRNG has not seeded yet.)

# Examples

```rust
use rand::Rng;

let mut rng = rand::task_rng();
if rng.gen() { // bool
    println!("int: {}, uint: {}", rng.gen::<int>(), rng.gen::<uint>())
}
```

```rust
let tuple_ptr = rand::random::<Box<(f64, char)>>();
println!("{:?}", tuple_ptr)
```
*/

#![crate_id = "rand#0.11.0-pre"]
#![license = "MIT/ASL2"]
#![crate_type = "dylib"]
#![crate_type = "rlib"]
#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico",
       html_root_url = "http://static.rust-lang.org/doc/master")]

#![feature(macro_rules, managed_boxes, phase)]
#![deny(deprecated_owned_vector)]

#[cfg(test)]
#[phase(syntax, link)] extern crate log;

use std::io::IoResult;
use std::kinds::marker;
use std::mem;
use std::strbuf::StrBuf;

pub use isaac::{IsaacRng, Isaac64Rng};
pub use os::OSRng;

use distributions::{Range, IndependentSample};
use distributions::range::SampleRange;

pub mod distributions;
pub mod isaac;
pub mod os;
pub mod reader;
pub mod reseeding;
mod rand_impls;

/// A type that can be randomly generated using an `Rng`.
pub trait Rand {
    /// Generates a random instance of this type using the specified source of
    /// randomness.
    fn rand<R: Rng>(rng: &mut R) -> Self;
}

/// A random number generator.
pub trait Rng {
    /// Return the next random u32.
    ///
    /// This rarely needs to be called directly, prefer `r.gen()` to
    /// `r.next_u32()`.
    // FIXME #7771: Should be implemented in terms of next_u64
    fn next_u32(&mut self) -> u32;

    /// Return the next random u64.
    ///
    /// By default this is implemented in terms of `next_u32`. An
    /// implementation of this trait must provide at least one of
    /// these two methods. Similarly to `next_u32`, this rarely needs
    /// to be called directly, prefer `r.gen()` to `r.next_u64()`.
    fn next_u64(&mut self) -> u64 {
        (self.next_u32() as u64 << 32) | (self.next_u32() as u64)
    }

    /// Fill `dest` with random data.
    ///
    /// This has a default implementation in terms of `next_u64` and
    /// `next_u32`, but should be overridden by implementations that
    /// offer a more efficient solution than just calling those
    /// methods repeatedly.
    ///
    /// This method does *not* have a requirement to bear any fixed
    /// relationship to the other methods, for example, it does *not*
    /// have to result in the same output as progressively filling
    /// `dest` with `self.gen::<u8>()`, and any such behaviour should
    /// not be relied upon.
    ///
    /// This method should guarantee that `dest` is entirely filled
    /// with new data, and may fail if this is impossible
    /// (e.g. reading past the end of a file that is being used as the
    /// source of randomness).
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut v = [0u8, .. 13579];
    /// task_rng().fill_bytes(v);
    /// println!("{:?}", v);
    /// ```
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        // this could, in theory, be done by transmuting dest to a
        // [u64], but this is (1) likely to be undefined behaviour for
        // LLVM, (2) has to be very careful about alignment concerns,
        // (3) adds more `unsafe` that needs to be checked, (4)
        // probably doesn't give much performance gain if
        // optimisations are on.
        let mut count = 0;
        let mut num = 0;
        for byte in dest.mut_iter() {
            if count == 0 {
                // we could micro-optimise here by generating a u32 if
                // we only need a few more bytes to fill the vector
                // (i.e. at most 4).
                num = self.next_u64();
                count = 8;
            }

            *byte = (num & 0xff) as u8;
            num >>= 8;
            count -= 1;
        }
    }

    /// Return a random value of a `Rand` type.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// let x: uint = rng.gen();
    /// println!("{}", x);
    /// println!("{:?}", rng.gen::<(f64, bool)>());
    /// ```
    #[inline(always)]
    fn gen<T: Rand>(&mut self) -> T {
        Rand::rand(self)
    }

    /// Return a random vector of the specified length.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// let x: Vec<uint> = rng.gen_vec(10);
    /// println!("{}", x);
    /// println!("{}", rng.gen_vec::<(f64, bool)>(5));
    /// ```
    fn gen_vec<T: Rand>(&mut self, len: uint) -> Vec<T> {
        Vec::from_fn(len, |_| self.gen())
    }

    /// Generate a random value in the range [`low`, `high`). Fails if
    /// `low >= high`.
    ///
    /// This is a convenience wrapper around
    /// `distributions::Range`. If this function will be called
    /// repeatedly with the same arguments, one should use `Range`, as
    /// that will amortize the computations that allow for perfect
    /// uniformity, as they only happen on initialization.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// let n: uint = rng.gen_range(0u, 10);
    /// println!("{}", n);
    /// let m: f64 = rng.gen_range(-40.0, 1.3e5);
    /// println!("{}", m);
    /// ```
    fn gen_range<T: Ord + SampleRange>(&mut self, low: T, high: T) -> T {
        assert!(low < high, "Rng.gen_range called with low >= high");
        Range::new(low, high).ind_sample(self)
    }

    /// Return a bool with a 1 in n chance of true
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// println!("{:b}", rng.gen_weighted_bool(3));
    /// ```
    fn gen_weighted_bool(&mut self, n: uint) -> bool {
        n == 0 || self.gen_range(0, n) == 0
    }

    /// Return a random string of the specified length composed of
    /// A-Z,a-z,0-9.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// println!("{}", task_rng().gen_ascii_str(10));
    /// ```
    fn gen_ascii_str(&mut self, len: uint) -> StrBuf {
        static GEN_ASCII_STR_CHARSET: &'static [u8] = bytes!("ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                                                             abcdefghijklmnopqrstuvwxyz\
                                                             0123456789");
        let mut s = StrBuf::with_capacity(len);
        for _ in range(0, len) {
            s.push_char(*self.choose(GEN_ASCII_STR_CHARSET).unwrap() as char)
        }
        s
    }

    /// Return a random element from `values`.
    ///
    /// Return `None` if `values` is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use rand::{task_rng, Rng};
    ///
    /// let choices = [1, 2, 4, 8, 16, 32];
    /// let mut rng = task_rng();
    /// println!("{}", rng.choose(choices));
    /// assert_eq!(rng.choose(choices.slice_to(0)), None);
    /// ```
    fn choose<'a, T>(&mut self, values: &'a [T]) -> Option<&'a T> {
        if values.is_empty() {
            None
        } else {
            Some(&values[self.gen_range(0u, values.len())])
        }
    }

    /// Deprecated name for `choose()`.
    #[deprecated = "replaced by .choose()"]
    fn choose_option<'a, T>(&mut self, values: &'a [T]) -> Option<&'a T> {
        self.choose(values)
    }

    /// Shuffle a mutable slice in place.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// let mut y = [1,2,3];
    /// rng.shuffle(y);
    /// println!("{}", y.as_slice());
    /// rng.shuffle(y);
    /// println!("{}", y.as_slice());
    /// ```
    fn shuffle<T>(&mut self, values: &mut [T]) {
        let mut i = values.len();
        while i >= 2u {
            // invariant: elements with index >= i have been locked in place.
            i -= 1u;
            // lock element i in place.
            values.swap(i, self.gen_range(0u, i + 1u));
        }
    }

    /// Shuffle a mutable slice in place.
    #[deprecated="renamed to `.shuffle`"]
    fn shuffle_mut<T>(&mut self, values: &mut [T]) {
        self.shuffle(values)
    }

    /// Randomly sample up to `n` elements from an iterator.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{task_rng, Rng};
    ///
    /// let mut rng = task_rng();
    /// let sample = rng.sample(range(1, 100), 5);
    /// println!("{}", sample);
    /// ```
    fn sample<A, T: Iterator<A>>(&mut self, iter: T, n: uint) -> Vec<A> {
        let mut reservoir = Vec::with_capacity(n);
        for (i, elem) in iter.enumerate() {
            if i < n {
                reservoir.push(elem);
                continue
            }

            let k = self.gen_range(0, i + 1);
            if k < reservoir.len() {
                *reservoir.get_mut(k) = elem
            }
        }
        reservoir
    }
}

/// A random number generator that can be explicitly seeded to produce
/// the same stream of randomness multiple times.
pub trait SeedableRng<Seed>: Rng {
    /// Reseed an RNG with the given seed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{Rng, SeedableRng, StdRng};
    ///
    /// let mut rng: StdRng = SeedableRng::from_seed(&[1, 2, 3, 4]);
    /// println!("{}", rng.gen::<f64>());
    /// rng.reseed([5, 6, 7, 8]);
    /// println!("{}", rng.gen::<f64>());
    /// ```
    fn reseed(&mut self, Seed);

    /// Create a new RNG with the given seed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rand::{Rng, SeedableRng, StdRng};
    ///
    /// let mut rng: StdRng = SeedableRng::from_seed(&[1, 2, 3, 4]);
    /// println!("{}", rng.gen::<f64>());
    /// ```
    fn from_seed(seed: Seed) -> Self;
}

/// Create a random number generator with a default algorithm and seed.
///
/// It returns the strongest `Rng` algorithm currently implemented in
/// pure Rust. If you require a specifically seeded `Rng` for
/// consistency over time you should pick one algorithm and create the
/// `Rng` yourself.
///
/// This is a very expensive operation as it has to read randomness
/// from the operating system and use this in an expensive seeding
/// operation. If one does not require high performance generation of
/// random numbers, `task_rng` and/or `random` may be more
/// appropriate.
#[deprecated="use `task_rng` or `StdRng::new`"]
pub fn rng() -> StdRng {
    StdRng::new().unwrap()
}

/// The standard RNG. This is designed to be efficient on the current
/// platform.
#[cfg(not(target_word_size="64"))]
pub struct StdRng { rng: IsaacRng }

/// The standard RNG. This is designed to be efficient on the current
/// platform.
#[cfg(target_word_size="64")]
pub struct StdRng { rng: Isaac64Rng }

impl StdRng {
    /// Create a randomly seeded instance of `StdRng`.
    ///
    /// This is a very expensive operation as it has to read
    /// randomness from the operating system and use this in an
    /// expensive seeding operation. If one is only generating a small
    /// number of random numbers, or doesn't need the utmost speed for
    /// generating each number, `task_rng` and/or `random` may be more
    /// appropriate.
    ///
    /// Reading the randomness from the OS may fail, and any error is
    /// propagated via the `IoResult` return value.
    #[cfg(not(target_word_size="64"))]
    pub fn new() -> IoResult<StdRng> {
        IsaacRng::new().map(|r| StdRng { rng: r })
    }
    /// Create a randomly seeded instance of `StdRng`.
    ///
    /// This is a very expensive operation as it has to read
    /// randomness from the operating system and use this in an
    /// expensive seeding operation. If one is only generating a small
    /// number of random numbers, or doesn't need the utmost speed for
    /// generating each number, `task_rng` and/or `random` may be more
    /// appropriate.
    ///
    /// Reading the randomness from the OS may fail, and any error is
    /// propagated via the `IoResult` return value.
    #[cfg(target_word_size="64")]
    pub fn new() -> IoResult<StdRng> {
        Isaac64Rng::new().map(|r| StdRng { rng: r })
    }
}

impl Rng for StdRng {
    #[inline]
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }
}

impl<'a> SeedableRng<&'a [uint]> for StdRng {
    fn reseed(&mut self, seed: &'a [uint]) {
        // the internal RNG can just be seeded from the above
        // randomness.
        self.rng.reseed(unsafe {mem::transmute(seed)})
    }

    fn from_seed(seed: &'a [uint]) -> StdRng {
        StdRng { rng: SeedableRng::from_seed(unsafe {mem::transmute(seed)}) }
    }
}

/// Create a weak random number generator with a default algorithm and seed.
///
/// It returns the fastest `Rng` algorithm currently available in Rust without
/// consideration for cryptography or security. If you require a specifically
/// seeded `Rng` for consistency over time you should pick one algorithm and
/// create the `Rng` yourself.
///
/// This will read randomness from the operating system to seed the
/// generator.
pub fn weak_rng() -> XorShiftRng {
    match XorShiftRng::new() {
        Ok(r) => r,
        Err(e) => fail!("weak_rng: failed to create seeded RNG: {}", e)
    }
}

/// An Xorshift[1] random number
/// generator.
///
/// The Xorshift algorithm is not suitable for cryptographic purposes
/// but is very fast. If you do not know for sure that it fits your
/// requirements, use a more secure one such as `IsaacRng` or `OSRng`.
///
/// [1]: Marsaglia, George (July 2003). ["Xorshift
/// RNGs"](http://www.jstatsoft.org/v08/i14/paper). *Journal of
/// Statistical Software*. Vol. 8 (Issue 14).
pub struct XorShiftRng {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl Rng for XorShiftRng {
    #[inline]
    fn next_u32(&mut self) -> u32 {
        let x = self.x;
        let t = x ^ (x << 11);
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        let w = self.w;
        self.w = w ^ (w >> 19) ^ (t ^ (t >> 8));
        self.w
    }
}

impl SeedableRng<[u32, .. 4]> for XorShiftRng {
    /// Reseed an XorShiftRng. This will fail if `seed` is entirely 0.
    fn reseed(&mut self, seed: [u32, .. 4]) {
        assert!(!seed.iter().all(|&x| x == 0),
                "XorShiftRng.reseed called with an all zero seed.");

        self.x = seed[0];
        self.y = seed[1];
        self.z = seed[2];
        self.w = seed[3];
    }

    /// Create a new XorShiftRng. This will fail if `seed` is entirely 0.
    fn from_seed(seed: [u32, .. 4]) -> XorShiftRng {
        assert!(!seed.iter().all(|&x| x == 0),
                "XorShiftRng::from_seed called with an all zero seed.");

        XorShiftRng {
            x: seed[0],
            y: seed[1],
            z: seed[2],
            w: seed[3]
        }
    }
}

impl XorShiftRng {
    /// Create an xor shift random number generator with a random seed.
    pub fn new() -> IoResult<XorShiftRng> {
        let mut s = [0u8, ..16];
        let mut r = try!(OSRng::new());
        loop {
            r.fill_bytes(s);

            if !s.iter().all(|x| *x == 0) {
                break;
            }
        }
        let s: [u32, ..4] = unsafe { mem::transmute(s) };
        Ok(SeedableRng::from_seed(s))
    }
}

/// Controls how the task-local RNG is reseeded.
struct TaskRngReseeder;

impl reseeding::Reseeder<StdRng> for TaskRngReseeder {
    fn reseed(&mut self, rng: &mut StdRng) {
        *rng = match StdRng::new() {
            Ok(r) => r,
            Err(e) => fail!("could not reseed task_rng: {}", e)
        }
    }
}
static TASK_RNG_RESEED_THRESHOLD: uint = 32_768;
type TaskRngInner = reseeding::ReseedingRng<StdRng, TaskRngReseeder>;
/// The task-local RNG.
pub struct TaskRng {
    // This points into TLS (specifically, it points to the endpoint
    // of a Box stored in TLS, to make it robust against TLS moving
    // things internally) and so this struct cannot be legally
    // transferred between tasks *and* it's unsafe to deallocate the
    // RNG other than when a task is finished.
    //
    // The use of unsafe code here is OK if the invariants above are
    // satisfied; and it allows us to avoid (unnecessarily) using a
    // GC'd or RC'd pointer.
    rng: *mut TaskRngInner,
    marker: marker::NoSend,
}

/// Retrieve the lazily-initialized task-local random number
/// generator, seeded by the system. Intended to be used in method
/// chaining style, e.g. `task_rng().gen::<int>()`.
///
/// The RNG provided will reseed itself from the operating system
/// after generating a certain amount of randomness.
///
/// The internal RNG used is platform and architecture dependent, even
/// if the operating system random number generator is rigged to give
/// the same sequence always. If absolute consistency is required,
/// explicitly select an RNG, e.g. `IsaacRng` or `Isaac64Rng`.
pub fn task_rng() -> TaskRng {
    // used to make space in TLS for a random number generator
    local_data_key!(TASK_RNG_KEY: Box<TaskRngInner>)

    match TASK_RNG_KEY.get() {
        None => {
            let r = match StdRng::new() {
                Ok(r) => r,
                Err(e) => fail!("could not initialize task_rng: {}", e)
            };
            let mut rng = box reseeding::ReseedingRng::new(r,
                                                        TASK_RNG_RESEED_THRESHOLD,
                                                        TaskRngReseeder);
            let ptr = &mut *rng as *mut TaskRngInner;

            TASK_RNG_KEY.replace(Some(rng));

            TaskRng { rng: ptr, marker: marker::NoSend }
        }
        Some(rng) => TaskRng {
            rng: &**rng as *_ as *mut TaskRngInner,
            marker: marker::NoSend
        }
    }
}

impl Rng for TaskRng {
    fn next_u32(&mut self) -> u32 {
        unsafe { (*self.rng).next_u32() }
    }

    fn next_u64(&mut self) -> u64 {
        unsafe { (*self.rng).next_u64() }
    }

    #[inline]
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        unsafe { (*self.rng).fill_bytes(bytes) }
    }
}

/// Generate a random value using the task-local random number
/// generator.
///
/// # Example
///
/// ```rust
/// use rand::random;
///
/// if random() {
///     let x = random();
///     println!("{}", 2u * x);
/// } else {
///     println!("{}", random::<f64>());
/// }
/// ```
#[inline]
pub fn random<T: Rand>() -> T {
    task_rng().gen()
}

/// A wrapper for generating floating point numbers uniformly in the
/// open interval `(0,1)` (not including either endpoint).
///
/// Use `Closed01` for the closed interval `[0,1]`, and the default
/// `Rand` implementation for `f32` and `f64` for the half-open
/// `[0,1)`.
///
/// # Example
/// ```rust
/// use rand::{random, Open01};
///
/// let Open01(val) = random::<Open01<f32>>();
/// println!("f32 from (0,1): {}", val);
/// ```
pub struct Open01<F>(pub F);

/// A wrapper for generating floating point numbers uniformly in the
/// closed interval `[0,1]` (including both endpoints).
///
/// Use `Open01` for the closed interval `(0,1)`, and the default
/// `Rand` implementation of `f32` and `f64` for the half-open
/// `[0,1)`.
///
/// # Example
/// ```rust
/// use rand::{random, Closed01};
///
/// let Closed01(val) = random::<Closed01<f32>>();
/// println!("f32 from [0,1]: {}", val);
/// ```
pub struct Closed01<F>(pub F);

#[cfg(test)]
mod test {
    use super::{Rng, task_rng, random, SeedableRng, StdRng};

    struct ConstRng { i: u64 }
    impl Rng for ConstRng {
        fn next_u32(&mut self) -> u32 { self.i as u32 }
        fn next_u64(&mut self) -> u64 { self.i }

        // no fill_bytes on purpose
    }

    #[test]
    fn test_fill_bytes_default() {
        let mut r = ConstRng { i: 0x11_22_33_44_55_66_77_88 };

        // check every remainder mod 8, both in small and big vectors.
        let lengths = [0, 1, 2, 3, 4, 5, 6, 7,
                       80, 81, 82, 83, 84, 85, 86, 87];
        for &n in lengths.iter() {
            let mut v = Vec::from_elem(n, 0u8);
            r.fill_bytes(v.as_mut_slice());

            // use this to get nicer error messages.
            for (i, &byte) in v.iter().enumerate() {
                if byte == 0 {
                    fail!("byte {} of {} is zero", i, n)
                }
            }
        }
    }

    #[test]
    fn test_gen_range() {
        let mut r = task_rng();
        for _ in range(0, 1000) {
            let a = r.gen_range(-3i, 42);
            assert!(a >= -3 && a < 42);
            assert_eq!(r.gen_range(0, 1), 0);
            assert_eq!(r.gen_range(-12, -11), -12);
        }

        for _ in range(0, 1000) {
            let a = r.gen_range(10, 42);
            assert!(a >= 10 && a < 42);
            assert_eq!(r.gen_range(0, 1), 0);
            assert_eq!(r.gen_range(3_000_000u, 3_000_001), 3_000_000);
        }

    }

    #[test]
    #[should_fail]
    fn test_gen_range_fail_int() {
        let mut r = task_rng();
        r.gen_range(5i, -2);
    }

    #[test]
    #[should_fail]
    fn test_gen_range_fail_uint() {
        let mut r = task_rng();
        r.gen_range(5u, 2u);
    }

    #[test]
    fn test_gen_f64() {
        let mut r = task_rng();
        let a = r.gen::<f64>();
        let b = r.gen::<f64>();
        debug!("{:?}", (a, b));
    }

    #[test]
    fn test_gen_weighted_bool() {
        let mut r = task_rng();
        assert_eq!(r.gen_weighted_bool(0u), true);
        assert_eq!(r.gen_weighted_bool(1u), true);
    }

    #[test]
    fn test_gen_ascii_str() {
        let mut r = task_rng();
        debug!("{}", r.gen_ascii_str(10u));
        debug!("{}", r.gen_ascii_str(10u));
        debug!("{}", r.gen_ascii_str(10u));
        assert_eq!(r.gen_ascii_str(0u).len(), 0u);
        assert_eq!(r.gen_ascii_str(10u).len(), 10u);
        assert_eq!(r.gen_ascii_str(16u).len(), 16u);
    }

    #[test]
    fn test_gen_vec() {
        let mut r = task_rng();
        assert_eq!(r.gen_vec::<u8>(0u).len(), 0u);
        assert_eq!(r.gen_vec::<u8>(10u).len(), 10u);
        assert_eq!(r.gen_vec::<f64>(16u).len(), 16u);
    }

    #[test]
    fn test_choose() {
        let mut r = task_rng();
        assert_eq!(r.choose([1, 1, 1]).map(|&x|x), Some(1));

        let v: &[int] = &[];
        assert_eq!(r.choose(v), None);
    }

    #[test]
    fn test_shuffle() {
        let mut r = task_rng();
        let empty: &mut [int] = &mut [];
        r.shuffle(empty);
        let mut one = [1];
        r.shuffle(one);
        assert_eq!(one.as_slice(), &[1]);

        let mut two = [1, 2];
        r.shuffle(two);
        assert!(two == [1, 2] || two == [2, 1]);

        let mut x = [1, 1, 1];
        r.shuffle(x);
        assert_eq!(x.as_slice(), &[1, 1, 1]);
    }

    #[test]
    fn test_task_rng() {
        let mut r = task_rng();
        r.gen::<int>();
        let mut v = [1, 1, 1];
        r.shuffle(v);
        assert_eq!(v.as_slice(), &[1, 1, 1]);
        assert_eq!(r.gen_range(0u, 1u), 0u);
    }

    #[test]
    fn test_random() {
        // not sure how to test this aside from just getting some values
        let _n : uint = random();
        let _f : f32 = random();
        let _o : Option<Option<i8>> = random();
        let _many : ((),
                     (Box<uint>,
                      @int,
                      Box<Option<Box<(@u32, Box<(@bool,)>)>>>),
                     (u8, i8, u16, i16, u32, i32, u64, i64),
                     (f32, (f64, (f64,)))) = random();
    }

    #[test]
    fn test_sample() {
        let min_val = 1;
        let max_val = 100;

        let mut r = task_rng();
        let vals = range(min_val, max_val).collect::<Vec<int>>();
        let small_sample = r.sample(vals.iter(), 5);
        let large_sample = r.sample(vals.iter(), vals.len() + 5);

        assert_eq!(small_sample.len(), 5);
        assert_eq!(large_sample.len(), vals.len());

        assert!(small_sample.iter().all(|e| {
            **e >= min_val && **e <= max_val
        }));
    }

    #[test]
    fn test_std_rng_seeded() {
        let s = task_rng().gen_vec::<uint>(256);
        let mut ra: StdRng = SeedableRng::from_seed(s.as_slice());
        let mut rb: StdRng = SeedableRng::from_seed(s.as_slice());
        assert_eq!(ra.gen_ascii_str(100u), rb.gen_ascii_str(100u));
    }

    #[test]
    fn test_std_rng_reseed() {
        let s = task_rng().gen_vec::<uint>(256);
        let mut r: StdRng = SeedableRng::from_seed(s.as_slice());
        let string1 = r.gen_ascii_str(100);

        r.reseed(s.as_slice());

        let string2 = r.gen_ascii_str(100);
        assert_eq!(string1, string2);
    }
}

#[cfg(test)]
static RAND_BENCH_N: u64 = 100;

#[cfg(test)]
mod bench {
    extern crate test;
    use self::test::Bencher;
    use {XorShiftRng, StdRng, IsaacRng, Isaac64Rng, Rng, RAND_BENCH_N};
    use std::mem::size_of;

    #[bench]
    fn rand_xorshift(b: &mut Bencher) {
        let mut rng = XorShiftRng::new().unwrap();
        b.iter(|| {
            for _ in range(0, RAND_BENCH_N) {
                rng.gen::<uint>();
            }
        });
        b.bytes = size_of::<uint>() as u64 * RAND_BENCH_N;
    }

    #[bench]
    fn rand_isaac(b: &mut Bencher) {
        let mut rng = IsaacRng::new().unwrap();
        b.iter(|| {
            for _ in range(0, RAND_BENCH_N) {
                rng.gen::<uint>();
            }
        });
        b.bytes = size_of::<uint>() as u64 * RAND_BENCH_N;
    }

    #[bench]
    fn rand_isaac64(b: &mut Bencher) {
        let mut rng = Isaac64Rng::new().unwrap();
        b.iter(|| {
            for _ in range(0, RAND_BENCH_N) {
                rng.gen::<uint>();
            }
        });
        b.bytes = size_of::<uint>() as u64 * RAND_BENCH_N;
    }

    #[bench]
    fn rand_std(b: &mut Bencher) {
        let mut rng = StdRng::new().unwrap();
        b.iter(|| {
            for _ in range(0, RAND_BENCH_N) {
                rng.gen::<uint>();
            }
        });
        b.bytes = size_of::<uint>() as u64 * RAND_BENCH_N;
    }

    #[bench]
    fn rand_shuffle_100(b: &mut Bencher) {
        let mut rng = XorShiftRng::new().unwrap();
        let x : &mut[uint] = [1,..100];
        b.iter(|| {
            rng.shuffle(x);
        })
    }
}
