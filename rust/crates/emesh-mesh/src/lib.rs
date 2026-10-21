//! Meshing: the preview that reproduces Eilmer's construction, the VTK reader for Eilmer's own
//! grids, cell quality, the `lmr` runner (export) and the background live mesher.

pub mod live;
pub mod preview;
pub mod quality;
pub mod runner;
pub mod vtk;

pub use preview::{block_grid, Grid};
