#[cfg(not(feature = "profiling"))]
mod profiling_disabled;
#[cfg(feature = "profiling")]
mod profiling_enabled;

#[cfg(not(feature = "profiling"))]
pub use profiling_disabled::*;
#[cfg(feature = "profiling")]
pub use profiling_enabled::*;

#[cfg(feature = "gpu_profiling")]
mod opengl;

#[cfg(feature = "gpu_profiling")]
pub use opengl::*;
