//! Eilmer Mesher core: the document model, curve geometry with Eilmer's parameterisation,
//! planar topology, blocking, sub-division, boundary layers, offsets and the editing model.
//!
//! This is a port of the Python reference implementation (`eilmer_mesher/model`); golden files
//! generated from Python (`golden/`) check that both produce the same results.

pub mod blocking;
pub mod dimensions;
pub mod document;
pub mod entities;
pub mod face;
pub mod geom;
pub mod importers;
pub mod layers;
pub mod golden;
pub mod jsonfmt;
pub mod num;
pub mod offset;
pub mod operations;
pub mod poly;
pub mod pyfmt;
pub mod snapping;
pub mod spacing;
pub mod subdivide;
pub mod topology;
pub mod typed_input;
pub mod units;

pub use document::{Block, BlockError, Cluster, CurveRef, Dimension, Document, Face, FACES};
pub use entities::{Curve, CurveType, Geom, Kind};
pub use num::P;
