//! Mimizan Lab X-Trans: a Fuji X-Trans RAF to a linear DNG.
//!
//! The mosaic is not interpolated. It is taken apart into the luminance
//! and the two chrominances it carries on the tile's own frequencies
//! ([`xtrans`]), under the prior the mosaic itself makes most probable,
//! and the three channels that come out are written as a DNG a raw
//! developer opens like any other ([`convert`], [`dng`]). [`file`] is the
//! whole way from one file to the other.

pub mod convert;
pub mod dng;
pub mod file;
pub mod filter;
pub mod plane;
#[cfg(test)]
mod rng;
pub mod xtrans;
